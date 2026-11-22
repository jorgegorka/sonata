use std::collections::BTreeSet;

use super::drafts::drafts_relative;
use super::*;
use crate::context::real_dir_chain;

impl Engine {
    pub async fn remove_session(&self, session_id: &str) -> Result<RemovedSessions> {
        let (mut removed, stopped, released, runs) = {
            let mut live = self.inner.live.lock().await;
            let starting = |live: &HashMap<String, Live>, sessions: &[Session]| -> Result<()> {
                if sessions.iter().any(|s| live.get(&s.id).is_some_and(|l| l.status == BotStatus::Starting)) {
                    return Err(Refusal::SessionStarting.into());
                }
                Ok(())
            };
            let tree = self.store.session_tree(session_id)?;
            starting(&live, &tree)?;
            let runs = self.stop_runs_of_sessions(&tree, "removed")?;
            let removed = self
                .store
                .remove_session(session_id, |sessions| starting(&live, sessions))?
                .ok_or_else(|| Refusal::SessionNotFound(session_id.into()))?;
            let stopped: Vec<(Session, RuntimeBot)> = removed
                .sessions
                .iter()
                .filter_map(|r| Some((r.session.clone(), live.remove(&r.session.id)?.rt?)))
                .collect();
            self.inner
                .shown_sessions
                .lock()
                .unwrap()
                .retain(|id, _| !removed.sessions.iter().any(|r| r.session.id == *id));
            for r in &removed.sessions {
                self.emit(UiEvent::SessionRemoved {
                    session_id: r.session.id.clone(),
                    bot_id: r.session.bot_id.clone(),
                    thread_ids: r.thread_ids.clone(),
                });
            }
            let bots: BTreeSet<&str> = removed.sessions.iter().map(|r| r.session.bot_id.as_str()).collect();
            for bot_id in bots {
                self.publish_status(&live, bot_id);
            }
            let released = self.release_claims(&mut live, &removed.messages);
            (removed, stopped, released, runs)
        };
        for (session, rt) in &stopped {
            if let Err(e) = self.inner.runtime.stop_bot(rt).await {
                tracing::warn!(bot = %self.handle_of(&session.bot_id), session = session.number, error = %e, "could not close the removed session's terminal");
            }
        }
        self.wind_down(&runs).await;
        self.remove_inbox_files(&removed.messages);
        for session_id in released {
            tokio::spawn(self.pump_session(&session_id));
        }
        for r in &removed.sessions {
            tracing::info!(bot = %self.handle_of(&r.session.bot_id), session = r.session.number, threads = r.thread_ids.len(), "session removed");
        }
        removed.drafts = self.removed_drafts(&removed.sessions);
        Ok(removed)
    }

    fn removed_drafts(&self, removed: &[RemovedSession]) -> Vec<PathBuf> {
        let mut drafts = Vec::new();
        for r in removed {
            let bot = match self.store.get_bot(&r.session.bot_id) {
                Ok(Some(bot)) => bot,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(bot = %r.session.bot_id, session = r.session.number, error = %e, "could not read the bot to find the removed session's drafts");
                    continue;
                }
            };
            let dir = self.bot_directory(&bot);
            let relative = drafts_relative(&r.session);
            let folder = dir.join(&relative);
            let Ok(meta) = std::fs::symlink_metadata(&folder) else { continue };
            let real = |path: &Path| std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir());
            if real(&dir) && real_dir_chain(&dir, &relative).is_ok() && meta.is_dir() && folder_inside(&folder, &dir) {
                drafts.push(folder);
            } else {
                tracing::warn!(bot = %bot.handle, session = r.session.number, folder = %folder.display(), "the removed session's drafts are behind a link or not a folder; they stay");
            }
        }
        drafts
    }

    fn release_claims(&self, live: &mut HashMap<String, Live>, removed: &[Message]) -> Vec<String> {
        let claimable: HashSet<&str> =
            removed.iter().filter(|m| m.status == MessageStatus::Queued).map(|m| m.id.as_str()).collect();
        let mut released = Vec::new();
        for (id, l) in live.iter_mut() {
            if l.in_flight.as_ref().is_some_and(|f| claimable.contains(f.message_id.as_str())) {
                l.in_flight = None;
                l.status = BotStatus::Idle;
                released.push((id.clone(), l.bot_id.clone()));
            }
        }
        for (_, bot_id) in &released {
            self.publish_status(live, bot_id);
        }
        released.into_iter().map(|(id, _)| id).collect()
    }

    fn remove_inbox_files(&self, messages: &[Message]) {
        let mut by_bot: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for m in messages.iter().filter(|m| m.recipient != USER) {
            by_bot.entry(&m.recipient).or_default().push(format!("{}-", m.id));
        }
        for (bot_id, prefixes) in by_bot {
            let bot = match self.store.get_bot(bot_id) {
                Ok(Some(bot)) => bot,
                Ok(None) => continue,
                Err(e) => {
                    tracing::error!(bot = %bot_id, error = %e, "could not read the bot to delete its inbox files");
                    continue;
                }
            };
            let dir = self.bot_directory(&bot);
            let inbox = dir.join("inbox");
            match std::fs::symlink_metadata(&inbox) {
                Ok(meta) if meta.is_dir() && folder_inside(&dir, &self.bots_root()) => {}
                Ok(_) => {
                    tracing::warn!(bot = %bot.handle, folder = %inbox.display(), "the bot's inbox is not a folder in its own folder; its files stay");
                    continue;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    tracing::warn!(bot = %bot.handle, folder = %inbox.display(), error = %e, "could not read the bot's inbox");
                    continue;
                }
            }
            let entries = match std::fs::read_dir(&inbox) {
                Ok(entries) => entries,
                Err(e) => {
                    tracing::warn!(bot = %bot.handle, folder = %inbox.display(), error = %e, "could not read the bot's inbox");
                    continue;
                }
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                if !prefixes.iter().any(|p| name.as_encoded_bytes().starts_with(p.as_bytes())) {
                    continue;
                }
                if let Err(e) = std::fs::remove_file(entry.path()) {
                    tracing::warn!(bot = %bot.handle, file = %entry.path().display(), error = %e, "could not delete a removed message's inbox file");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AppPaths;
    use crate::runtime::fake::FakeRuntime;

    #[tokio::test]
    async fn a_claim_on_a_removed_message_is_given_up_and_the_next_message_delivered() {
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
        let bot = |name: &str| {
            let new = NewBot {
                display_name: name.into(),
                color: "#3b82f6".into(),
                provider: Provider::Claude,
                model: None,
                instructions: String::new(),
                skill_ids: vec![],
            };
            engine.create_bot(new).unwrap()
        };
        let (planner, reviewer) = (bot("Planner"), bot("Reviewer"));
        let x = engine.store.create_session(&planner.id, None, None).unwrap();
        let r = engine.store.create_session(&reviewer.id, None, Some(&x.id)).unwrap();
        let question = engine
            .store
            .insert_message(&NewMessage {
                sender: reviewer.id.clone(),
                recipient: planner.id.clone(),
                sender_session: Some(r.id.clone()),
                recipient_session: Some(x.id.clone()),
                kind: MessageKind::Text,
                body: "Which release?".into(),
                attachments: vec![],
                system_code: None,
                system_params: None,
                status: MessageStatus::Queued,
                chain_id: new_id(),
                hop: 0,
                in_reply_to: None,
            })
            .unwrap();
        let waiting = engine.queue_user_message_to_session(&x.id, "Plan the release.", &[]).unwrap();
        {
            let mut live = engine.inner.live.lock().await;
            let l = live.entry(x.id.clone()).or_insert_with(|| Live::new(&planner.id));
            l.rt = Some(RuntimeBot {
                handle: "planner".into(),
                pane_id: "w1:p1".into(),
                tab_id: "t".into(),
                session_id: None,
            });
            l.status = BotStatus::Working;
            l.in_flight = Some(InFlight {
                message_id: question.id.clone(),
                sender: reviewer.id.clone(),
                sender_session: Some(r.id.clone()),
                chain_id: question.chain_id.clone(),
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
                scheduled: question.task_id.is_some(),
                from_run: question.workflow_mark.is_some(),
            });
        }

        engine.remove_session(&r.id).await.unwrap();
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }

        let prompts = rt.state().prompts.clone();
        assert_eq!(prompts.len(), 1, "{prompts:?}");
        assert_eq!((prompts[0].0.as_str(), prompts[0].1.lines().nth(1)), ("w1:p1", Some("Plan the release.")));
        let delivered = engine.store.get_message(&waiting.id).unwrap().unwrap();
        assert_eq!(delivered.status, MessageStatus::Delivered);
        let chat = engine.store.thread_messages(&waiting.thread_id).unwrap();
        assert!(chat.iter().all(|m| m.system_code.is_none()), "no failed delivery to report: {chat:?}");
        let live = engine.inner.live.lock().await;
        assert_eq!(live[&x.id].in_flight.as_ref().map(|f| f.message_id.as_str()), Some(waiting.id.as_str()));
    }
}
