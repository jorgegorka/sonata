use super::*;

pub(super) enum Archive {
    Done(Vec<Session>),
    Blocked(Block),
}

pub(super) enum Block {
    Working,
    HoldsAsker,
}

impl Engine {
    pub async fn archive_session(&self, session_id: &str) -> Result<Vec<Session>> {
        if let Some((run, Some(key))) = self.store.orchestration_of_session(session_id)? {
            self.archive_orchestration_task(&run.id, &key, true).await?;
            let ids = self
                .orchestration_view(&run.id)?
                .executions
                .into_iter()
                .find(|t| t.task.spec.key == key)
                .map(|t| t.session_ids)
                .unwrap_or_default();
            return ids
                .iter()
                .map(|id| self.store.get_session(id))
                .collect::<Result<Vec<_>>>()
                .map(|s| s.into_iter().flatten().collect());
        }
        match self.archive_tree(session_id, None).await? {
            Archive::Done(archived) => Ok(archived),
            Archive::Blocked(_) => unreachable!("no guard was given"),
        }
    }

    pub(super) async fn archive_tree(&self, session_id: &str, guard: Option<&str>) -> Result<Archive> {
        self.guard_orchestration_archive(session_id)?;
        let (archived, stops, runs) = {
            let mut live = self.inner.live.lock().await;
            if let Some(asker) = guard {
                let tree = self.store.session_tree(session_id)?;
                if tree.iter().any(|s| s.id == asker) {
                    return Ok(Archive::Blocked(Block::HoldsAsker));
                }
                for session in tree.iter().filter(|s| s.archived_at.is_none()) {
                    let working = live.get(&session.id).is_some_and(|l| {
                        l.in_flight.is_some() || l.limited_turn.is_some() || l.status == BotStatus::Working
                    });
                    if working || !self.store.queued_for_session(&session.id)?.is_empty() {
                        return Ok(Archive::Blocked(Block::Working));
                    }
                }
            }
            let archived = self
                .store
                .archive_session(session_id, |sessions| {
                    if sessions.iter().any(|s| live.get(&s.id).is_some_and(|l| l.status == BotStatus::Starting)) {
                        return Err(Refusal::SessionStarting.into());
                    }
                    Ok(())
                })?
                .ok_or_else(|| Refusal::SessionNotFound(session_id.into()))?;
            let runs = self.stop_runs_of_sessions(&archived, "archived").unwrap_or_else(|e| {
                let who = self.who(session_id);
                tracing::error!(bot = %who.bot, session = who.session, error = %e, "could not stop the workflow runs of the archived sessions");
                Vec::new()
            });
            let mut stops = Vec::new();
            for session in &archived {
                match self.stop_locked(&mut live, &session.id) {
                    Ok(stop) => stops.push(stop),
                    Err(e) => {
                        tracing::error!(bot = %self.handle_of(&session.bot_id), session = session.number, error = %e, "could not cancel the archived session's queue");
                        let (rt, flight) = self.drop_live(&mut live, &session.id);
                        stops.push(Stop { session_id: session.id.clone(), rt, flight, cancelled: Vec::new() });
                    }
                }
            }
            for session in &archived {
                self.emit(UiEvent::SessionUpdated { session: self.session_view(session.clone()) });
            }
            (archived, stops, runs)
        };
        for stop in stops {
            self.stopped(stop).await;
        }
        self.wind_down(&runs).await;
        for session in &archived {
            tracing::info!(bot = %self.handle_of(&session.bot_id), session = session.number, "session archived");
        }
        Ok(Archive::Done(archived))
    }

    pub async fn restore_session(&self, session_id: &str) -> Result<Vec<Session>> {
        if let Some((run, Some(key))) = self.store.orchestration_of_session(session_id)? {
            self.archive_orchestration_task(&run.id, &key, false).await?;
            let ids = self
                .orchestration_view(&run.id)?
                .executions
                .into_iter()
                .find(|t| t.task.spec.key == key)
                .map(|t| t.session_ids)
                .unwrap_or_default();
            return ids
                .iter()
                .map(|id| self.store.get_session(id))
                .collect::<Result<Vec<_>>>()
                .map(|s| s.into_iter().flatten().collect());
        }
        let restored =
            self.store.restore_session(session_id)?.ok_or_else(|| Refusal::SessionNotFound(session_id.into()))?;
        for session in &restored {
            self.emit(UiEvent::SessionUpdated { session: self.session_view(session.clone()) });
            tracing::info!(bot = %self.handle_of(&session.bot_id), session = session.number, "session restored");
        }
        Ok(restored)
    }

    pub async fn rename_session(&self, session_id: &str, name: &str) -> Result<SessionView> {
        let _live = self.inner.live.lock().await;
        self.store.get_session(session_id)?.ok_or_else(|| Refusal::SessionNotFound(session_id.into()))?;
        self.store.set_session_title(session_id, session_title(name).as_deref())?;
        let session = self.store.get_session(session_id)?.ok_or_else(|| Refusal::SessionNotFound(session_id.into()))?;
        let view = self.session_view(session);
        self.emit(UiEvent::SessionUpdated { session: view.clone() });
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AppPaths;
    use crate::runtime::fake::FakeRuntime;

    #[derive(Clone, Default)]
    struct Log(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Log {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn planner() -> (Engine, Arc<FakeRuntime>, Session, tempfile::TempDir) {
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
        let new = NewBot {
            display_name: "Planner".into(),
            color: "#3b82f6".into(),
            provider: Provider::Claude,
            model: None,
            instructions: String::new(),
            skill_ids: vec![],
        };
        let planner = engine.create_bot(new).unwrap();
        let x = engine.store.create_session(&planner.id, None, None).unwrap();
        (engine, rt, x, tmp)
    }

    fn flight(message: &Message) -> InFlight {
        InFlight {
            message_id: message.id.clone(),
            sender: message.sender.clone(),
            sender_session: message.sender_session.clone(),
            chain_id: message.chain_id.clone(),
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
            scheduled: message.task_id.is_some(),
            from_run: message.workflow_mark.is_some(),
        }
    }

    #[tokio::test]
    async fn a_rename_waits_for_an_archive_and_announces_the_session_as_stored() {
        let (engine, _rt, x, _tmp) = planner();
        let live = engine.inner.live.lock().await;
        let rename = tokio::spawn({
            let (engine, id) = (engine.clone(), x.id.clone());
            async move { engine.rename_session(&id, "Release").await }
        });
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(!rename.is_finished(), "it waits for the archive under way");
        engine.store.archive_session(&x.id, |_| Ok(())).unwrap();
        drop(live);

        let view = rename.await.unwrap().unwrap();
        assert_eq!((view.session.title.as_deref(), view.session.archived_at.is_some()), (Some("Release"), true));
    }

    #[tokio::test]
    async fn archiving_between_the_claim_and_the_prompt_interrupts_the_claimed_message() {
        let (engine, rt, x, _tmp) = planner();
        let claimed = engine.queue_user_message_to_session(&x.id, "Plan the release.", &[]).unwrap();
        let waiting = engine.queue_user_message_to_session(&x.id, "And the notes.", &[]).unwrap();
        {
            let mut live = engine.inner.live.lock().await;
            let l = live.entry(x.id.clone()).or_insert_with(|| Live::new(&x.bot_id));
            l.rt = Some(RuntimeBot {
                handle: "planner".into(),
                pane_id: "w1:p1".into(),
                tab_id: "t".into(),
                session_id: None,
            });
            l.status = BotStatus::Working;
            l.in_flight = Some(flight(&claimed));
        }

        engine.archive_session(&x.id).await.unwrap();

        let status = |m: &Message| engine.store.get_message(&m.id).unwrap().unwrap().status;
        assert_eq!((status(&claimed), status(&waiting)), (MessageStatus::Interrupted, MessageStatus::Cancelled));
        assert_eq!(rt.state().stopped, vec!["w1:p1".to_string()]);
        assert!(!engine.inner.live.lock().await.contains_key(&x.id), "it is offline");
    }

    #[tokio::test]
    async fn a_fallback_for_a_session_archived_after_its_turn_ended_is_dropped() {
        let (engine, _rt, x, _tmp) = planner();
        let asked = engine.queue_user_message_to_session(&x.id, "Plan the release.", &[]).unwrap();
        engine.store.archive_session(&x.id, |_| Ok(())).unwrap();
        let log = Log::default();
        let writer = log.clone();
        let subscriber = tracing_subscriber::fmt().with_ansi(false).with_writer(move || writer.clone()).finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        engine.post_fallback(&x.id, "The plan.", &flight(&asked));

        let chat = engine.store.thread_messages(&asked.thread_id).unwrap();
        assert_eq!(chat.len(), 1, "{chat:?}");
        let text = String::from_utf8_lossy(&log.0.lock().unwrap()).into_owned();
        let line = text.lines().find(|l| l.contains("the session was archived; no fallback")).unwrap_or_default();
        assert!(line.contains("INFO") && line.contains("bot=planner session=1"), "{text}");
        assert!(!text.contains("ERROR"), "{text}");
    }
}
