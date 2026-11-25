use super::*;

pub(super) fn checked_bot(new: NewBot) -> Result<NewBot> {
    let display_name = new.display_name.trim().to_string();
    if display_name.is_empty() {
        return Err(Refusal::NameRequired.into());
    }
    if adapter_for(new.provider).is_none() {
        return Err(Refusal::ProviderUnavailable(new.provider.as_str().into()).into());
    }
    if let Some(model) = new.model.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        if new.provider == Provider::Opencode && !names_its_provider(model) {
            return Err(Refusal::ModelFormat(model.to_string()).into());
        }
    }
    Ok(NewBot { display_name, ..new })
}

fn names_its_provider(model: &str) -> bool {
    model.split_once('/').is_some_and(|(provider, name)| !provider.trim().is_empty() && !name.trim().is_empty())
}

impl Engine {
    pub fn update_bot(&self, bot_id: &str, edit: NewBot) -> Result<Bot> {
        self.store.require_active()?;
        let edit = checked_bot(edit)?;
        let edit = NewBot { skill_ids: self.checked_skill_ids(&edit.skill_ids)?, ..edit };
        let before = self.store.get_bot(bot_id)?;
        let bot = self.store.update_bot(bot_id, &edit)?.ok_or_else(|| Refusal::BotNotFound(bot_id.into()))?;
        let model = |bot: &Bot| bot.model.as_deref().map(str::trim).filter(|m| !m.is_empty()).map(str::to_string);
        let new_model = before.is_some_and(|before| before.provider == bot.provider && model(&before) != model(&bot));
        if new_model && adapter_for(bot.provider).is_some_and(|adapter| adapter.keeps_its_model_on_resume()) {
            self.store.forget_cli_sessions(&bot.id)?;
            tracing::info!(bot = %bot.handle, "a new model: the bot's sessions start new conversations");
        }
        tracing::info!(bot = %bot.handle, config_version = bot.config_version, provider = bot.provider.as_str(), "bot edited");
        Ok(bot)
    }

    pub async fn delete_bot(&self, bot_id: &str) -> Result<PathBuf> {
        if self.inner.live.lock().await.values().any(|l| l.bot_id == bot_id && l.status == BotStatus::Starting) {
            return Err(Refusal::BotStarting.into());
        }
        for session in self.store.sessions_of(bot_id)? {
            if let Some((run, key)) = self.store.orchestration_of_session(&session.id)? {
                let direct = key
                    .as_ref()
                    .is_none_or(|key| run.tasks.iter().any(|t| t.spec.key == *key && t.workflow_run_id.is_none()));
                if direct && run.status == crate::orchestration::Status::Active {
                    self.cancel_orchestration(&run.id, key.as_deref()).await?;
                }
            }
        }
        let (bot, numbers, states, cancelled) = {
            let mut live = self.inner.live.lock().await;
            if live.values().any(|l| l.bot_id == bot_id && l.status == BotStatus::Starting) {
                return Err(Refusal::BotStarting.into());
            }
            let numbers: HashMap<String, i64> =
                self.store.sessions_of(bot_id)?.into_iter().map(|s| (s.id, s.number)).collect();
            let (bot, cancelled) = self.store.delete_bot(bot_id)?.ok_or_else(|| Refusal::BotNotFound(bot_id.into()))?;
            let sessions: Vec<String> =
                live.iter().filter(|(_, l)| l.bot_id == bot_id).map(|(id, _)| id.clone()).collect();
            let states: Vec<(i64, Live)> = sessions
                .iter()
                .filter_map(|id| Some((numbers.get(id).copied().unwrap_or(0), live.remove(id)?)))
                .collect();
            (bot, numbers, states, cancelled)
        };
        self.pause_runs_missing_a_session(Some(&bot.handle));
        self.inner.shown.lock().unwrap().remove(bot_id);
        self.inner.shown_sessions.lock().unwrap().retain(|_, (b, _)| b != bot_id);
        self.inner.start_locks.lock().unwrap().remove(bot_id);
        self.inner.written.lock().unwrap().remove(bot_id);
        let mut sessions: Vec<(String, i64)> = numbers.into_iter().collect();
        sessions.sort_by_key(|(_, number)| *number);
        for (session_id, _) in sessions {
            self.emit(UiEvent::SessionRemoved { session_id, bot_id: bot.id.clone(), thread_ids: Vec::new() });
        }
        for message in cancelled {
            self.emit(UiEvent::MessageUpdated { message });
        }
        for (session, state) in &states {
            if let Some(flight) = state.in_flight.as_ref().or(state.limited_turn.as_ref()).filter(|f| !f.follow_up) {
                match self.store.set_message_status(&flight.message_id, MessageStatus::Cancelled) {
                    Ok(message) => self.emit(UiEvent::MessageUpdated { message }),
                    Err(e) if removed_with_its_conversation(&e) => {
                        tracing::info!(bot = %bot.handle, session, message_id = %flight.message_id, "the message was removed with its conversation")
                    }
                    Err(e) => {
                        tracing::error!(bot = %bot.handle, session, message_id = %flight.message_id, error = %e, "could not cancel the deleted bot's message")
                    }
                }
            }
            if let Some(rt) = &state.rt {
                if let Err(e) = self.inner.runtime.stop_bot(rt).await {
                    tracing::warn!(bot = %bot.handle, session, error = %e, "could not close the deleted bot's terminal");
                }
            }
        }
        tracing::info!(bot = %bot.handle, "bot deleted");
        self.bot_directory(&bot)
    }
}
