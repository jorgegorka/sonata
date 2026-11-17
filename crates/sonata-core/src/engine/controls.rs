use super::*;

pub(super) const SNAPSHOT_LINES: u32 = 60;

const TERMINAL_LINES: u32 = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogAnswer {
    Continue,
    Cancel,
}

impl DialogAnswer {
    fn key(self) -> &'static str {
        match self {
            DialogAnswer::Continue => "enter",
            DialogAnswer::Cancel => "esc",
        }
    }
}

impl Engine {
    pub async fn cancel_message(&self, message_id: &str) -> Result<Message> {
        let live = self.inner.live.lock().await;
        if live.values().any(|l| l.in_flight.as_ref().is_some_and(|f| f.message_id == message_id)) {
            return Err(Refusal::AlreadyDelivering.into());
        }
        let message = self.inner.store.cancel_queued(message_id)?.ok_or(Refusal::NotQueued)?;
        drop(live);
        self.emit(UiEvent::MessageUpdated { message: message.clone() });
        Ok(message)
    }

    pub fn retry_message(&self, message_id: &str) -> Result<Message> {
        let only_interrupted = || Refusal::NotInterrupted;
        let old =
            self.inner.store.get_message(message_id)?.ok_or_else(|| Refusal::MessageNotFound(message_id.into()))?;
        if old.status != MessageStatus::Interrupted {
            return Err(only_interrupted().into());
        }
        for bot_id in [Some(&old.recipient), Some(&old.sender).filter(|s| *s != USER)].into_iter().flatten() {
            if self.inner.store.get_bot(bot_id)?.is_none() {
                return Err(Refusal::BotNotFound(bot_id.clone()).into());
            }
        }
        let from_user = old.sender == USER;
        let copy = NewMessage {
            sender: old.sender,
            recipient: old.recipient,
            sender_session: old.sender_session,
            recipient_session: old.recipient_session,
            kind: if from_user { MessageKind::Text } else { old.kind },
            body: old.body,
            system_code: None,
            system_params: None,
            attachments: old.attachments,
            status: MessageStatus::Queued,
            chain_id: if from_user { new_id() } else { old.chain_id },
            hop: if from_user { 0 } else { old.hop },
            in_reply_to: if from_user { None } else { old.in_reply_to },
        };
        let (original, message) =
            self.inner.store.retry_interrupted(message_id, &copy)?.ok_or_else(only_interrupted)?;
        self.emit(UiEvent::MessageUpdated { message: original });
        self.emit_message(message.clone());
        Ok(message)
    }

    pub async fn stop_turn(&self, session_id: &str) -> Result<()> {
        let (rt, message_id, keys) = {
            let mut live = self.inner.live.lock().await;
            match live.get_mut(session_id) {
                Some(Live { rt: Some(rt), in_flight: Some(flight), started, .. }) => {
                    flight.stopped = true;
                    let keys = started
                        .as_ref()
                        .and_then(|s| adapter_for(s.provider))
                        .map_or(&["esc"][..], |adapter| adapter.interrupt_keys());
                    (rt.clone(), flight.message_id.clone(), keys)
                }
                _ => return Err(Refusal::NotWorking.into()),
            }
        };
        let Err(e) = self.inner.runtime.send_keys(&rt, keys).await else { return Ok(()) };
        let who = self.who(session_id);
        tracing::warn!(bot = %who.bot, session = who.session, message_id = %message_id, error = %e, "could not stop the session's turn");
        if let Some(flight) = self.inner.live.lock().await.get_mut(session_id).and_then(|l| l.in_flight.as_mut()) {
            if flight.message_id == message_id {
                flight.stopped = false;
            }
        }
        Err(Refusal::StopFailed(e.to_string()).into())
    }

    pub async fn answer_dialog(&self, session_id: &str, answer: DialogAnswer) -> Result<()> {
        let rt = match self.inner.live.lock().await.get(session_id) {
            Some(Live { rt: Some(rt), status: BotStatus::NeedsAttention, .. }) => rt.clone(),
            Some(Live { rt: Some(_), .. }) => return Err(Refusal::NotWaiting.into()),
            _ => return Err(Refusal::NotRunning.into()),
        };
        self.inner
            .runtime
            .send_keys(&rt, &[answer.key()])
            .await
            .map_err(|e| Refusal::AnswerFailed(e.to_string()).into())
    }

    pub async fn snapshot(&self, session_id: &str) -> Result<Option<String>> {
        let Some(rt) = self.running(session_id).await else { return Ok(None) };
        self.inner
            .runtime
            .snapshot(&rt, SNAPSHOT_LINES)
            .await
            .map(Some)
            .map_err(|e| Refusal::SnapshotFailed(e.to_string()).into())
    }

    pub async fn terminal(&self, session_id: &str) -> Result<Option<String>> {
        let Some(rt) = self.running(session_id).await else { return Ok(None) };
        self.inner
            .runtime
            .terminal(&rt, TERMINAL_LINES)
            .await
            .map(Some)
            .map_err(|e| Refusal::SnapshotFailed(e.to_string()).into())
    }

    async fn running(&self, session_id: &str) -> Option<RuntimeBot> {
        self.inner.live.lock().await.get(session_id).and_then(|l| l.rt.clone())
    }

    pub async fn restart_session(&self, session_id: &str) -> Result<()> {
        let (rt, flight) = {
            let mut live = self.inner.live.lock().await;
            let session =
                self.inner.store.get_session(session_id)?.ok_or_else(|| Refusal::SessionNotFound(session_id.into()))?;
            if session.archived_at.is_some() {
                return Err(Refusal::SessionArchived(session.id).into());
            }
            let l = live.entry(session_id.to_string()).or_insert_with(|| Live::new(&session.bot_id));
            if l.status == BotStatus::Starting {
                return Err(Refusal::SessionStarting.into());
            }
            l.status = BotStatus::Offline;
            (l.rt.take(), l.in_flight.take().or_else(|| l.limited_turn.take()))
        };
        let who = self.who(session_id);
        if let Some(f) = flight.filter(|f| !f.follow_up) {
            match self.inner.store.set_message_status(&f.message_id, MessageStatus::Interrupted) {
                Ok(message) => self.report_interrupted(message),
                Err(e) if removed_with_its_conversation(&e) => {
                    tracing::info!(bot = %who.bot, session = who.session, message_id = %f.message_id, "the message was removed with its conversation")
                }
                Err(e) => {
                    tracing::error!(bot = %who.bot, session = who.session, message_id = %f.message_id, error = %e, "could not mark the message interrupted")
                }
            }
        }
        self.emit_status(session_id).await;
        if let Some(rt) = rt {
            if let Err(e) = self.inner.runtime.stop_bot(&rt).await {
                tracing::warn!(bot = %who.bot, session = who.session, error = %e, "could not close the session's terminal for a restart");
            }
        }
        tracing::info!(bot = %who.bot, session = who.session, "restarting the session");
        tokio::spawn(self.pump_session(session_id));
        Ok(())
    }

    pub async fn stop_session(&self, session_id: &str) -> Result<()> {
        if self.inner.store.get_session(session_id)?.is_none() {
            return Err(Refusal::SessionNotFound(session_id.into()).into());
        }
        let stop = {
            let mut live = self.inner.live.lock().await;
            if live.get(session_id).is_some_and(|l| l.status == BotStatus::Starting) {
                return Err(Refusal::SessionStarting.into());
            }
            self.stop_locked(&mut live, session_id)?
        };
        let who = self.who(session_id);
        self.stopped(stop).await;
        tracing::info!(bot = %who.bot, session = who.session, "session stopped");
        Ok(())
    }

    pub(super) fn stop_locked(&self, live: &mut HashMap<String, Live>, session_id: &str) -> Result<Stop> {
        let claimed = live.get(session_id).and_then(|l| l.in_flight.as_ref()).map(|f| f.message_id.clone());
        let mut cancelled = Vec::new();
        for queued in
            self.inner.store.queued_for_session(session_id)?.into_iter().filter(|m| Some(&m.id) != claimed.as_ref())
        {
            cancelled.extend(self.inner.store.cancel_queued(&queued.id)?);
        }
        let (rt, flight) = self.drop_live(live, session_id);
        Ok(Stop { session_id: session_id.to_string(), rt, flight, cancelled })
    }

    pub(super) async fn stopped(&self, stop: Stop) {
        let Stop { session_id, rt, flight, cancelled } = stop;
        let who = self.who(&session_id);
        if let Some(f) = flight.filter(|f| !f.follow_up) {
            match self.inner.store.set_message_status(&f.message_id, MessageStatus::Interrupted) {
                Ok(message) => self.report_interrupted(message),
                Err(e) if removed_with_its_conversation(&e) => {
                    tracing::info!(bot = %who.bot, session = who.session, message_id = %f.message_id, "the message was removed with its conversation")
                }
                Err(e) => {
                    tracing::error!(bot = %who.bot, session = who.session, message_id = %f.message_id, error = %e, "could not mark the message interrupted")
                }
            }
        }
        for message in cancelled {
            self.emit(UiEvent::MessageUpdated { message });
        }
        if let Some(rt) = rt {
            if let Err(e) = self.inner.runtime.stop_bot(&rt).await {
                tracing::warn!(bot = %who.bot, session = who.session, error = %e, "could not close the stopped session's terminal");
            }
        }
    }

    pub(super) fn drop_live(
        &self,
        live: &mut HashMap<String, Live>,
        session_id: &str,
    ) -> (Option<RuntimeBot>, Option<InFlight>) {
        let Some(l) = live.remove(session_id) else { return (None, None) };
        self.publish_status(live, &l.bot_id);
        (l.rt, l.in_flight.or(l.limited_turn))
    }

    pub fn paused(&self) -> Result<bool> {
        Ok(self.inner.store.get_setting("paused")?.as_deref() == Some("true"))
    }

    pub async fn set_paused(&self, paused: bool) -> Result<()> {
        self.inner.store.set_setting("paused", if paused { "true" } else { "false" })?;
        tracing::info!(paused, "pause all bots");
        self.emit(UiEvent::Paused { paused });
        if !paused {
            for session_id in self.inner.store.sessions_with_queued()? {
                tokio::spawn(self.pump_session(&session_id));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AppPaths;
    use crate::runtime::fake::FakeRuntime;

    struct Claimed {
        engine: Engine,
        rt: Arc<FakeRuntime>,
        session: Session,
        pane: RuntimeBot,
        claimed: Message,
        behind: Message,
        _tmp: tempfile::TempDir,
    }

    async fn claimed() -> Claimed {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
        paths.ensure().unwrap();
        let rt = FakeRuntime::new();
        let cfg = EngineConfig {
            paths: paths.clone(),
            sbot_dir: tmp.path().join("bin"),
            fallback_lines: 200,
            turn_start_grace: None,
        };
        let engine = Engine::new(Store::open(&paths.db_path()).unwrap(), rt.clone(), cfg);
        let bot = engine
            .create_bot(NewBot {
                display_name: "Reviewer".into(),
                color: "#3b82f6".into(),
                provider: Provider::Claude,
                model: None,
                instructions: String::new(),
                skill_ids: vec![],
            })
            .unwrap();
        let session = engine.inner.store.create_session(&bot.id, None, None).unwrap();
        let claimed = engine.queue_user_message_to_session(&session.id, "being sent", &[]).unwrap();
        let behind = engine.queue_user_message_to_session(&session.id, "behind it", &[]).unwrap();
        let pane =
            RuntimeBot { handle: "reviewer".into(), pane_id: "w1:p1".into(), tab_id: "t".into(), session_id: None };
        {
            let mut live = engine.inner.live.lock().await;
            let l = live.entry(session.id.clone()).or_insert_with(|| Live::new(&bot.id));
            l.rt = Some(pane.clone());
            l.status = BotStatus::Working;
            l.in_flight = Some(InFlight {
                message_id: claimed.id.clone(),
                sender: USER.into(),
                sender_session: None,
                chain_id: claimed.chain_id.clone(),
                hop: 0,
                in_reply_to: None,
                replied: false,
                seen_working: false,
                limit_noticed: false,
                delivered_at: None,
                enter_pressed_at: None,
                stopped: false,
                follow_up: false,
                after_limit: false,
                scheduled: claimed.task_id.is_some(),
                from_run: claimed.workflow_mark.is_some(),
            });
        }
        Claimed { engine, rt, session, pane, claimed, behind, _tmp: tmp }
    }

    #[tokio::test]
    async fn stopping_between_the_claim_and_the_prompt_interrupts_the_claimed_message() {
        let Claimed { engine, session, claimed, behind, .. } = claimed().await;
        let mut rx = engine.subscribe_ui();

        engine.stop_session(&session.id).await.unwrap();

        let status = |m: &Message| engine.inner.store.get_message(&m.id).unwrap().unwrap().status;
        assert_eq!(status(&claimed), MessageStatus::Interrupted);
        assert_eq!(status(&behind), MessageStatus::Cancelled);
        let seen: Vec<UiEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            !seen.iter().any(|e| matches!(e, UiEvent::MessageUpdated { message } if message.id == claimed.id && message.status == MessageStatus::Cancelled)),
            "{seen:?}"
        );
        engine.retry_message(&claimed.id).unwrap();
    }

    fn prompted(rt: &FakeRuntime, body: &str) -> bool {
        rt.state().prompts.iter().any(|(_, text)| text.contains(body))
    }

    #[tokio::test]
    async fn a_delivery_whose_claim_a_stop_took_sends_nothing() {
        let Claimed { engine, rt, session, pane, claimed, .. } = claimed().await;
        engine.stop_session(&session.id).await.unwrap();
        let mut rx = engine.subscribe_ui();

        engine.send_claimed(&session.id, pane, claimed.clone(), false, "being sent".into()).await.unwrap();

        let status = engine.inner.store.get_message(&claimed.id).unwrap().unwrap().status;
        assert_eq!(status, MessageStatus::Interrupted);
        assert!(!prompted(&rt, "being sent"), "{:?}", rt.state().prompts);
        let seen: Vec<UiEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(!seen.iter().any(|e| matches!(e, UiEvent::MessageUpdated { .. })), "{seen:?}");
        engine.retry_message(&claimed.id).unwrap();
    }

    #[tokio::test]
    async fn a_delivery_whose_claim_a_restart_took_sends_nothing() {
        let Claimed { engine, rt, session, pane, claimed, behind, .. } = claimed().await;
        engine.restart_session(&session.id).await.unwrap();

        engine.send_claimed(&session.id, pane, claimed.clone(), false, "being sent".into()).await.unwrap();

        let status = |m: &Message| engine.inner.store.get_message(&m.id).unwrap().unwrap().status;
        assert_eq!(status(&claimed), MessageStatus::Interrupted);
        for _ in 0..100 {
            if status(&behind) == MessageStatus::Delivered {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(status(&behind), MessageStatus::Delivered, "the restarted session takes the next one");
        assert!(prompted(&rt, "behind it") && !prompted(&rt, "being sent"), "{:?}", rt.state().prompts);
    }

    #[tokio::test]
    async fn a_claim_on_a_message_no_longer_queued_is_released() {
        let Claimed { engine, rt, session, pane, claimed, behind, .. } = claimed().await;
        engine.inner.store.set_message_status(&claimed.id, MessageStatus::Interrupted).unwrap();

        engine.send_claimed(&session.id, pane, claimed.clone(), false, "being sent".into()).await.unwrap();

        let status = |m: &Message| engine.inner.store.get_message(&m.id).unwrap().unwrap().status;
        for _ in 0..100 {
            if status(&behind) == MessageStatus::Delivered {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!((status(&claimed), status(&behind)), (MessageStatus::Interrupted, MessageStatus::Delivered));
        assert!(prompted(&rt, "behind it") && !prompted(&rt, "being sent"), "{:?}", rt.state().prompts);
    }
}
