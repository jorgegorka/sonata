use bridge_proto::{BridgeResponse, ErrorCode};

use super::messaging::{conversation_removed, internal, not_a_bot, refuse, Caller, PostKind, Target};
use super::*;
use crate::store::Question;

pub(super) fn result_answers(
    request: &str,
    answers: Option<&InFlight>,
    requester: Option<&str>,
) -> (Option<String>, Option<String>) {
    match answers {
        Some(f) if f.sender_session.is_some() && f.sender_session.as_deref() == requester => {
            (Some(f.message_id.clone()), None)
        }
        Some(f) => (Some(request.to_string()), Some(f.message_id.clone())),
        None => (Some(request.to_string()), None),
    }
}

pub(super) fn ask_options(options: Vec<String>) -> std::result::Result<Vec<String>, String> {
    if options.len() > 4 {
        return Err(format!("A question takes at most 4 options; you gave {}.", options.len()));
    }
    let options: Vec<String> = options.iter().map(|o| o.trim().to_string()).collect();
    if options.iter().any(|o| o.is_empty()) {
        return Err("An option is empty. Give each --option a short text.".into());
    }
    if let Some(o) = options.iter().find(|o| o.contains('\n') || o.contains('\r')) {
        return Err(format!("The option \"{o}\" has a line break; keep each option on one line."));
    }
    if let Some(o) = options.iter().find(|o| o.chars().count() > 60) {
        let n = o.chars().count();
        return Err(format!("The option \"{o}\" is {n} characters long; keep each option to 60 characters."));
    }
    for (i, o) in options.iter().enumerate() {
        if options[..i].iter().any(|p| p.to_lowercase() == o.to_lowercase()) {
            return Err(format!("The option \"{o}\" is given twice."));
        }
    }
    Ok(options)
}

impl Engine {
    pub(super) async fn bridge_ask(
        &self,
        pane_id: &str,
        body: String,
        files: Vec<String>,
        options: Vec<String>,
    ) -> BridgeResponse {
        let Some((from, flight)) = self.caller(pane_id).await else { return not_a_bot() };
        let options = match ask_options(options) {
            Ok(options) => options,
            Err(text) => return refuse(ErrorCode::BadRequest, text),
        };
        self.post_from_bot(&from, Target::User, body, files, flight, PostKind::Ask { options }).await
    }

    pub(super) async fn bridge_result(
        &self,
        pane_id: &str,
        body: String,
        files: Vec<String>,
        outcome: Option<String>,
    ) -> BridgeResponse {
        let response = self.bridge_result_inner(pane_id, body, files, outcome).await;
        if response.ok {
            if let Err(error) = self.reconcile_work_items() {
                tracing::error!(%error, "could not continue tasks after result");
            }
        }
        response
    }

    async fn bridge_result_inner(
        &self,
        pane_id: &str,
        body: String,
        files: Vec<String>,
        outcome: Option<String>,
    ) -> BridgeResponse {
        let Some((from, flight)) = self.caller(pane_id).await else { return not_a_bot() };
        if body.trim().is_empty() {
            return refuse(ErrorCode::EmptyBody, "A final result is required.");
        }
        if let Ok(Some((run, visit))) = self.store.open_visit_of_session(&from.session_id) {
            if let Some(step) = crate::workflow::step(&run.def, &visit.step_id) {
                if let Err(error) = crate::workflow::follow(step, outcome.as_deref()) {
                    return refuse(ErrorCode::BadRequest, error);
                }
            }
        } else if outcome.is_some() {
            return refuse(
                ErrorCode::BadRequest,
                "--outcome is only for a workflow step, and this conversation has no open step.",
            );
        }
        let staged = (|| {
            let bot =
                self.store.get_bot(&from.bot_id)?.ok_or_else(|| crate::orchestration::invalid("Bot not found."))?;
            let checked = attachments::check(&files, &self.bot_directory(&bot)?).map_err(Refusal::Attachment)?;
            let paths = checked.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>();
            self.store.stage_work_item_result(&from.session_id, &body, &paths, outcome.as_deref())
        })();
        match staged {
            Ok(true) => {
                if let Some(f) = flight {
                    self.set_replied(&from.session_id, &f.message_id, true).await;
                }
                return BridgeResponse::output(
                    "Final result saved. Waiting for subtasks and required reviews. End your turn.",
                );
            }
            Err(e) => return refuse(ErrorCode::BadRequest, e.to_string()),
            _ => {}
        }
        match self.store.own_orchestration(&from.session_id) {
            Ok(own) if own.is_some() || self.store.session_is_task_followup(&from.session_id).unwrap_or(false) => {
                return match self.finish_waiting_task(&from.session_id, &body, &files, outcome) {
                    Ok(message) => {
                        if let Some(f) = flight {
                            self.set_replied(&from.session_id, &f.message_id, true).await;
                        }
                        self.emit_message(message.clone());
                        let mut response = BridgeResponse::output("Task result recorded. End your turn.");
                        response.message_id = Some(message.id);
                        response
                    }
                    Err(e) => refuse(ErrorCode::BadRequest, e.to_string()),
                }
            }
            Err(e) => return internal(e),
            _ => {}
        }
        match self.store.open_visit_of_session(&from.session_id) {
            Ok(Some((run, visit))) => {
                return self.step_result(&from, flight, run, visit, body, files, outcome).await;
            }
            Ok(None) => {}
            Err(e) => return internal(e),
        }
        if outcome.is_some() {
            return refuse(
                ErrorCode::BadRequest,
                "--outcome is only for a workflow step, and this conversation has no open step.",
            );
        }
        let session = match self.store.get_session(&from.session_id) {
            Ok(Some(session)) => session,
            Ok(None) => return not_a_bot(),
            Err(e) => return internal(e),
        };
        let Some(request) = session.open_request_id.clone() else {
            let followup = flight
                .as_ref()
                .and_then(|f| self.store.get_message(&f.message_id).ok().flatten())
                .is_some_and(|m| m.sender == USER && m.workflow_mark.is_none() && m.system_code.is_none());
            if !self.store.session_is_primary_task(&session.id).unwrap_or(false)
                || (self.store.session_has_workflow(&session.id).unwrap_or(true) && !followup)
            {
                return match self.opener_handle(&session) {
                    Ok(Some(handle)) => refuse(
                        ErrorCode::NoOpenRequest,
                        format!(
                            "No bot is waiting for a result from this conversation. Send your result to @{handle}, \
                         who opened this conversation, with `sbot send @{handle}`."
                        ),
                    ),
                    Ok(None) => refuse(
                        ErrorCode::NoOpenRequest,
                        "No bot is waiting for a result from this conversation. Answer with `sbot reply`, or message \
                     someone with `sbot send`.",
                    ),
                    Err(e) => internal(e),
                };
            }
            if body.trim().is_empty() {
                return refuse(ErrorCode::EmptyBody, "A final result is required.");
            }
            let result = (|| {
                let bot =
                    self.store.get_bot(&from.bot_id)?.ok_or_else(|| crate::orchestration::invalid("Bot not found."))?;
                let checked = attachments::check(&files, &self.bot_directory(&bot)?).map_err(Refusal::Attachment)?;
                self.store.finish_standalone_task(&session.id, &body, &attachments::in_place(&checked))
            })();
            return match result {
                Ok(message) => {
                    if let Some(flight) = flight {
                        self.set_replied(&session.id, &flight.message_id, true).await;
                    }
                    self.emit_message(message.clone());
                    let mut response = BridgeResponse::output("Task finished. End your turn.");
                    response.message_id = Some(message.id);
                    response
                }
                Err(e) => refuse(ErrorCode::BadRequest, e.to_string()),
            };
        };
        let request = match self.store.get_message(&request) {
            Ok(Some(request)) => request,
            Ok(None) => return conversation_removed(),
            Err(e) => return internal(e),
        };
        let bot = match self.store.get_bot(&request.sender) {
            Ok(Some(bot)) => bot,
            Ok(None) => {
                return refuse(
                    ErrorCode::UnknownRecipient,
                    "The bot that asked no longer exists, so the result was not sent. If the user should know, \
                     use `sbot send user`.",
                )
            }
            Err(e) => return internal(e),
        };
        let handle = bot.handle.clone();
        let to = Target::Bot { bot: Box::new(bot), session: request.sender_session };
        let response =
            self.post_from_bot(&from, to, body, files, flight, PostKind::Result { request: request.id }).await;
        if let (true, Some(id)) = (response.ok, &response.message_id) {
            let who = self.who(&from.session_id);
            tracing::info!(bot = %who.bot, session = who.session, message_id = %id, to = %handle, "sent the result of the open request, which closes it");
        }
        response
    }

    pub(super) fn store_question(
        &self,
        from: &Caller,
        asker: &Bot,
        id: &str,
        question: &NewMessage,
        options: &[String],
        answers: Option<&InFlight>,
    ) -> Result<Question> {
        let Some(flight) = answers.filter(|f| f.sender != USER) else {
            return self.store.insert_question(id, question, options, None, None, None);
        };
        let request = match flight.in_reply_to {
            None => Some(flight.message_id.clone()),
            Some(_) => self.exchange_request(&from.session_id, flight)?,
        };
        let Some(request) = request else {
            return self.store.insert_question(id, question, options, None, None, None);
        };
        let who = self.who(&from.session_id);
        if self.is_master(&asker.id)? {
            tracing::info!(bot = %who.bot, session = who.session, message_id = %flight.message_id, "Master asked the user on a bot's message; Master's sessions open no request");
            return self.store.insert_question(id, question, options, None, None, Some(&flight.message_id));
        }
        let requester = match flight.sender_session.as_deref() {
            Some(session) => self.store.get_session(session)?.filter(|s| s.archived_at.is_none()),
            None => None,
        };
        let Some(requester) = requester else {
            tracing::info!(bot = %who.bot, session = who.session, message_id = %flight.message_id, "the requesting session is archived or removed; the question opens no request");
            return self.store.insert_question(id, question, options, None, None, Some(&flight.message_id));
        };
        if request != flight.message_id {
            tracing::info!(bot = %who.bot, session = who.session, message_id = %flight.message_id, request = %request, "asked the user on a bot's answer; the question opens the request their exchange began with");
        }
        let notice = NewMessage::system(&requester, "session_waiting", json!({"handle": asker.handle}));
        self.store.insert_question(id, question, options, Some(&request), Some(&notice), Some(&flight.message_id))
    }

    fn exchange_request(&self, session_id: &str, flight: &InFlight) -> Result<Option<String>> {
        let mut next = flight.in_reply_to.clone();
        let mut first = None;
        while let Some(id) = next {
            let Some(message) = self.store.get_message(&id)? else { return Ok(None) };
            next = message.in_reply_to.clone();
            first = Some(message);
        }
        Ok(first
            .filter(|m| {
                m.sender != USER
                    && m.sender_session.is_some()
                    && m.sender_session == flight.sender_session
                    && m.recipient_session.as_deref() == Some(session_id)
            })
            .map(|m| m.id))
    }

    fn opener_handle(&self, session: &Session) -> Result<Option<String>> {
        let Some(opener) = session.opened_by.as_deref() else { return Ok(None) };
        if self.store.session_of_users_run(&session.id)? {
            return Ok(None);
        }
        let Some(opener) = self.store.get_session(opener)?.filter(|s| s.archived_at.is_none()) else {
            return Ok(None);
        };
        Ok(self.store.get_bot(&opener.bot_id)?.map(|bot| bot.handle))
    }

    pub(super) fn question_asked(&self, from: &Caller, question: Question) -> BridgeResponse {
        let response = BridgeResponse::sent(&question.message.id, USER);
        let who = self.who(&from.session_id);
        if question.notice.is_some() {
            tracing::info!(bot = %who.bot, session = who.session, message_id = %question.message.id, "asked the user on another bot's request, which stays open until its result");
        } else {
            tracing::info!(bot = %who.bot, session = who.session, message_id = %question.message.id, "asked the user; the session waits for the answer");
        }
        self.emit(UiEvent::SessionUpdated { session: self.session_view(question.session) });
        self.emit(UiEvent::MessageAdded { message: question.message });
        if let Some(notice) = question.notice {
            self.emit(UiEvent::MessageAdded { message: notice });
        }
        response
    }

    pub(super) fn requester_handle(&self, request_id: &str) -> Result<Option<String>> {
        let Some(request) = self.store.get_message(request_id)? else { return Ok(None) };
        Ok(self.store.get_bot(&request.sender)?.map(|bot| bot.handle))
    }

    pub(super) fn opened_session_waits(&self, bot_id: &str, opener: &str) -> Result<bool> {
        let sessions = self.store.sessions_of(bot_id)?;
        Ok(sessions.iter().any(|s| s.opened_by.as_deref() == Some(opener) && s.waiting_question_id.is_some()))
    }

    pub fn answered_by(&self, message_id: &str) -> Result<Option<String>> {
        self.store.answered_by(message_id)
    }
}

#[cfg(test)]
mod tests {
    use super::ask_options;

    #[test]
    fn ask_options_trims_each_option() {
        assert_eq!(ask_options(vec![" Approve ".into()]), Ok(vec!["Approve".into()]));
    }
}
