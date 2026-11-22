use super::*;

impl Engine {
    pub(super) async fn open_follow_up(&self, session_id: &str, rt: &RuntimeBot) {
        match self.inner.runtime.inspect(rt).await {
            Ok(inspection) if inspection.state == AgentState::Working => {}
            Ok(_) => return,
            Err(e) => {
                let who = self.who(session_id);
                tracing::warn!(bot = %who.bot, session = who.session, error = %e, "could not confirm that the session's CLI works on its own");
                return;
            }
        }
        let last = match self.store.last_delivered_to_session(session_id) {
            Ok(Some(message)) => message,
            Ok(None) => return,
            Err(e) => {
                let who = self.who(session_id);
                tracing::error!(bot = %who.bot, session = who.session, error = %e, "could not read the message a follow-up turn answers");
                return;
            }
        };
        {
            let mut live = self.inner.live.lock().await;
            let Some(l) = live.get_mut(session_id) else { return };
            let same_cli = l.rt.as_ref().is_some_and(|r| r.pane_id == rt.pane_id);
            if !same_cli || l.in_flight.is_some() || l.status != BotStatus::Idle {
                return;
            }
            l.in_flight = Some(InFlight {
                message_id: last.id.clone(),
                sender: last.sender.clone(),
                sender_session: last.sender_session.clone(),
                chain_id: last.chain_id.clone(),
                hop: last.hop,
                in_reply_to: last.in_reply_to.clone(),
                replied: false,
                seen_working: true,
                limit_noticed: false,
                delivered_at: Some(tokio::time::Instant::now()),
                enter_pressed_at: None,
                stopped: false,
                follow_up: true,
                after_limit: false,
                scheduled: last.task_id.is_some(),
                from_run: last.workflow_mark.is_some(),
            });
            l.status = BotStatus::Working;
        }
        let who = self.who(session_id);
        tracing::info!(bot = %who.bot, session = who.session, message_id = %last.id, "the session's CLI works on its own; following it as a turn");
        self.emit_status(session_id).await;
    }

    pub(super) async fn background_work(&self, session_id: &str, rt: &RuntimeBot) -> Option<bool> {
        let Some(adapter) = self.session_with_bot(session_id).and_then(|(bot, _)| adapter_for(bot.provider)) else {
            return Some(false);
        };
        match self.inner.runtime.snapshot(rt, super::controls::SNAPSHOT_LINES).await {
            Ok(screen) => Some(adapter.background_work(&screen)),
            Err(e) => {
                let who = self.who(session_id);
                tracing::warn!(bot = %who.bot, session = who.session, error = %e, "could not read the session's screen for background work");
                None
            }
        }
    }
}
