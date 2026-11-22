use super::*;

const IDLE_STOP_AFTER: Duration = Duration::from_secs(30 * 60);

impl Engine {
    pub fn run_watchdog(&self, every: Duration) -> tokio::task::JoinHandle<()> {
        let engine = self.clone();
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(every);
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            timer.tick().await;
            loop {
                timer.tick().await;
                if engine.closing() {
                    break;
                }
                engine.watchdog_tick().await;
            }
        })
    }

    pub async fn watchdog_tick(&self) {
        let grace = self.inner.cfg.turn_start_grace.unwrap_or_default();
        self.check_usage_limits().await;
        self.stop_idle_sessions().await;
        let running: Vec<(String, RuntimeBot, Option<InFlight>, BotStatus)> = {
            let live = self.inner.live.lock().await;
            live.iter().filter_map(|(id, l)| Some((id.clone(), l.rt.clone()?, l.in_flight.clone(), l.status))).collect()
        };
        for (session_id, rt, flight, status) in running {
            let Ok(scoped) = self.for_session(&session_id) else { continue };
            match flight {
                Some(InFlight { delivered_at: None, .. }) => {}
                Some(f) if !f.seen_working && f.delivered_at.is_some_and(|at| at.elapsed() >= grace) => {
                    scoped.check_stalled_turn(&session_id, &f.message_id).await
                }
                flight => scoped.check_alive(&session_id, &rt, flight, status == BotStatus::Limited).await,
            }
        }
    }

    async fn stop_idle_sessions(&self) {
        let idle: Vec<(String, RuntimeBot)> = {
            let live = self.inner.live.lock().await;
            live.iter()
                .filter(|(_, l)| idle_long_enough(l))
                .filter_map(|(id, l)| Some((id.clone(), l.rt.clone()?)))
                .collect()
        };
        for (session_id, cli) in idle {
            match self.inner.runtime.inspect(&cli).await {
                Ok(inspection) if inspection.state == AgentState::Idle => {}
                _ => continue,
            }
            match self.background_work(&session_id, &cli).await {
                Some(false) => {}
                Some(true) => {
                    let mut live = self.inner.live.lock().await;
                    if let Some(l) = live.get_mut(&session_id).filter(|l| idle_long_enough(l)) {
                        l.idle_since = Some(tokio::time::Instant::now());
                    }
                    continue;
                }
                None => continue,
            }
            let rt = {
                let mut live = self.inner.live.lock().await;
                let queued = match self.store.queued_for_session(&session_id) {
                    Ok(queued) => !queued.is_empty(),
                    Err(e) => {
                        let who = self.who(&session_id);
                        tracing::error!(bot = %who.bot, session = who.session, error = %e, "could not read the session's queue for an idle stop");
                        true
                    }
                };
                if queued || !live.get(&session_id).is_some_and(idle_long_enough) {
                    continue;
                }
                self.drop_live(&mut live, &session_id).0
            };
            let who = self.who(&session_id);
            if let Some(rt) = rt {
                if let Err(e) = self.inner.runtime.stop_bot(&rt).await {
                    tracing::warn!(bot = %who.bot, session = who.session, error = %e, "could not close the idle session's terminal");
                }
            }
            tracing::info!(bot = %who.bot, session = who.session, "session stopped after 30 idle minutes");
        }
    }

    async fn check_alive(&self, session_id: &str, rt: &RuntimeBot, flight: Option<InFlight>, limited: bool) {
        match self.inner.runtime.inspect(rt).await {
            Err(RuntimeError::Gone) => match flight {
                Some(f) => self.vanished_mid_turn(session_id, &f.message_id, rt).await,
                None => self.vanished_while_idle(session_id, rt).await,
            },
            Ok(_) if limited => {}
            Ok(inspection) => match (inspection.state, flight) {
                (AgentState::Blocked, _) => self.on_state(&rt.pane_id, AgentState::Blocked).await,
                (AgentState::Idle, Some(f)) if f.seen_working => self.end_turn(session_id, &f.message_id, false).await,
                (AgentState::Working, None) => self.open_follow_up(session_id, rt).await,
                _ => {}
            },
            Err(e) => {
                let who = self.who(session_id);
                tracing::warn!(bot = %who.bot, session = who.session, error = %e, "watchdog could not inspect the session's CLI")
            }
        }
    }

    async fn vanished_while_idle(&self, session_id: &str, rt: &RuntimeBot) {
        let limited = {
            let mut live = self.inner.live.lock().await;
            let Some(l) = live.get_mut(session_id) else { return };
            let same = l.in_flight.is_none() && l.rt.as_ref().is_some_and(|r| r.pane_id == rt.pane_id);
            if !same {
                return;
            }
            l.rt = None;
            l.status = BotStatus::Offline;
            l.limited_turn.take().filter(|f| !f.follow_up)
        };
        let who = self.who(session_id);
        if let Some(f) = limited {
            match self.store.set_message_status(&f.message_id, MessageStatus::Interrupted) {
                Ok(message) => self.report_interrupted(message),
                Err(e) if removed_with_its_conversation(&e) => {
                    tracing::info!(bot = %who.bot, session = who.session, message_id = %f.message_id, "the message was removed with its conversation")
                }
                Err(e) => {
                    tracing::error!(bot = %who.bot, session = who.session, message_id = %f.message_id, error = %e, "could not mark the message interrupted")
                }
            }
        }
        tracing::info!(bot = %who.bot, session = who.session, "the session's CLI is gone; it is offline now");
        self.emit_status(session_id).await;
        let _ = self.inner.runtime.stop_bot(rt).await;
        tokio::spawn(self.pump_session(session_id));
    }
}

fn idle_long_enough(l: &Live) -> bool {
    l.rt.is_some()
        && l.status == BotStatus::Idle
        && l.in_flight.is_none()
        && l.idle_since.is_some_and(|since| since.elapsed() >= IDLE_STOP_AFTER)
}
