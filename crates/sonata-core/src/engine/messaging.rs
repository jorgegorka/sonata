use bridge_proto::{BotSummary, BridgeCommand, BridgeRequest, BridgeResponse, ErrorCode, PROTOCOL_VERSION};

use super::*;
use crate::attachments::{self, AttachmentError};
use crate::store::Route;

pub const MASTER_HOP_FACTOR: i64 = 5;

pub const DEFAULT_HOP_LIMIT: i64 = 12;

pub(super) enum Target {
    User,
    Bot { bot: Box<Bot>, session: Option<String> },
}

impl Target {
    fn id(&self) -> &str {
        match self {
            Target::User => USER,
            Target::Bot { bot, .. } => &bot.id,
        }
    }

    fn session(&self) -> Option<&str> {
        match self {
            Target::User => None,
            Target::Bot { session, .. } => session.as_deref(),
        }
    }

    fn label(&self) -> String {
        match self {
            Target::User => USER.to_string(),
            Target::Bot { bot, .. } => format!("@{}", bot.handle),
        }
    }
}

pub(super) struct Caller {
    pub(super) bot_id: String,
    pub(super) session_id: String,
}

pub(super) enum PostKind {
    Send,
    Reply,
    Ask { options: Vec<String> },
    Result { request: String },
}

pub(super) fn refuse(code: ErrorCode, message: impl Into<String>) -> BridgeResponse {
    BridgeResponse::error(code, message)
}

pub(super) fn not_a_bot() -> BridgeResponse {
    refuse(ErrorCode::NotABot, "This terminal is not a Sonata bot.")
}

pub(super) fn conversation_removed() -> BridgeResponse {
    refuse(
        ErrorCode::UnknownRecipient,
        "The conversation you are answering was removed, so this message was not sent. Do not send it \
         again; if the user should know, use `sbot send user`.",
    )
}

fn conversation_archived() -> BridgeResponse {
    refuse(
        ErrorCode::UnknownRecipient,
        "The conversation you are answering was archived, so this message was not sent. Do not send \
         it again; if the user should know, use `sbot send user`.",
    )
}

pub(super) fn internal(e: impl std::fmt::Display) -> BridgeResponse {
    refuse(ErrorCode::Internal, format!("Could not save the message: {e}"))
}

pub(super) fn file_error_code(e: &AttachmentError) -> ErrorCode {
    match e {
        AttachmentError::NotFound(_) | AttachmentError::NotAFile(_) => ErrorCode::FileNotFound,
        AttachmentError::TooLarge { .. } => ErrorCode::FileTooLarge,
        AttachmentError::Copy { .. } => ErrorCode::Internal,
    }
}

pub(super) fn role_line(instructions: &str) -> String {
    let line = instructions.lines().map(|l| l.trim().trim_start_matches('#').trim()).find(|l| !l.is_empty());
    line.unwrap_or("").chars().take(120).collect()
}

pub(super) fn status_word(status: BotStatus) -> &'static str {
    match status {
        BotStatus::Offline => "offline",
        BotStatus::Starting => "starting",
        BotStatus::Idle => "idle",
        BotStatus::Working => "working",
        BotStatus::NeedsAttention => "needs attention",
        BotStatus::Limited => "waiting for the usage limit",
    }
}

impl Engine {
    pub async fn handle_bridge(&self, req: BridgeRequest) -> BridgeResponse {
        let session = {
            let live = self.inner.live.lock().await;
            session_for_pane(&live, &req.pane_id)
        };
        let scoped = match session.and_then(|id| self.for_session(&id).ok()) {
            Some(engine) => engine,
            None => return not_a_bot(),
        };
        if let Err(e) = scoped.store.require_active() {
            return internal(e);
        }
        scoped.handle_project_bridge(req).await
    }

    async fn handle_project_bridge(&self, req: BridgeRequest) -> BridgeResponse {
        if req.v != PROTOCOL_VERSION {
            return refuse(
                ErrorCode::UnsupportedVersion,
                format!("sbot speaks bridge protocol {}, the app speaks {PROTOCOL_VERSION}.", req.v),
            );
        }
        if let Some(cli_id) = &req.cli_session_id {
            self.fill_session_id_from_sbot(&req.pane_id, cli_id).await;
        }
        let response = match req.command {
            BridgeCommand::Reply { body, files } => self.bridge_reply(&req.pane_id, body, files).await,
            BridgeCommand::Send { to, body, files } => self.bridge_send(&req.pane_id, &to, body, files).await,
            BridgeCommand::Bots => self.bridge_bots(&req.pane_id).await,
            BridgeCommand::Ask { body, files, options } => self.bridge_ask(&req.pane_id, body, files, options).await,
            BridgeCommand::Result { body, files, outcome } => {
                self.bridge_result(&req.pane_id, body, files, outcome).await
            }
            BridgeCommand::BotShow { handle } => self.bridge_bot_show(&req.pane_id, &handle).await,
            BridgeCommand::Skills => self.bridge_skills(&req.pane_id).await,
            BridgeCommand::SkillCopy { slug } => self.bridge_skill_copy(&req.pane_id, &slug).await,
            BridgeCommand::SkillAdd { folder, recreate } => {
                self.bridge_skill_add(&req.pane_id, &folder, recreate).await
            }
            BridgeCommand::BotCreate { name, color, provider, skills, instructions, recreate, model } => {
                self.bridge_bot_create(
                    &req.pane_id,
                    &name,
                    &color,
                    &provider,
                    model.as_deref(),
                    &skills,
                    &instructions,
                    recreate,
                )
                .await
            }
            BridgeCommand::Tasks => self.bridge_tasks(&req.pane_id).await,
            BridgeCommand::TaskCreate { name, bot, every, at, from, prompt, recreate } => {
                self.bridge_task_create(
                    &req.pane_id,
                    &name,
                    &bot,
                    every,
                    at.as_deref(),
                    from.as_deref(),
                    &prompt,
                    recreate,
                )
                .await
            }
            BridgeCommand::TaskEdit { task, name, bot, every, at, from, enabled, prompt } => {
                self.bridge_task_edit(&req.pane_id, &task, name, bot, every, at, from, enabled, prompt).await
            }
            BridgeCommand::TaskDelete { task } => self.bridge_task_delete(&req.pane_id, &task).await,
            BridgeCommand::Sessions { archived, all } => self.bridge_sessions(&req.pane_id, archived, all).await,
            BridgeCommand::SessionShow { session } => self.bridge_session_show(&req.pane_id, &session).await,
            BridgeCommand::SessionArchive { sessions } => self.bridge_session_archive(&req.pane_id, &sessions).await,
            BridgeCommand::SkillUpdate { slug, folder } => self.bridge_skill_update(&req.pane_id, &slug, &folder).await,
            BridgeCommand::BotEdit { handle, instructions, add_skills, remove_skills, model } => {
                self.bridge_bot_edit(
                    &req.pane_id,
                    &handle,
                    instructions.as_deref(),
                    &add_skills,
                    &remove_skills,
                    model.as_deref(),
                )
                .await
            }
            BridgeCommand::Workflows => self.bridge_workflows(&req.pane_id).await,
            BridgeCommand::WorkflowShow { name } => self.bridge_workflow_show(&req.pane_id, &name).await,
            BridgeCommand::WorkflowRun { name, body, files } => {
                self.bridge_workflow_run(&req.pane_id, &name, body, files).await
            }
            BridgeCommand::WorkflowCreate { definition, recreate } => {
                self.bridge_workflow_create(&req.pane_id, &definition, recreate).await
            }
            BridgeCommand::WorkflowEdit { name, definition } => {
                self.bridge_workflow_edit(&req.pane_id, &name, &definition).await
            }
            BridgeCommand::WorkflowDelete { name } => self.bridge_workflow_delete(&req.pane_id, &name).await,
        };
        if let Some(e) = &response.error {
            tracing::info!(pane = %req.pane_id, code = ?e.code, "sbot request refused: {}", e.message);
        }
        response
    }

    pub(super) async fn caller(&self, pane_id: &str) -> Option<(Caller, Option<InFlight>)> {
        let live = self.inner.live.lock().await;
        let session_id = session_for_pane(&live, pane_id)?;
        let l = live.get(&session_id)?;
        Some((Caller { bot_id: l.bot_id.clone(), session_id }, l.in_flight.clone()))
    }

    async fn bridge_reply(&self, pane_id: &str, body: String, files: Vec<String>) -> BridgeResponse {
        let Some((from, flight)) = self.caller(pane_id).await else { return not_a_bot() };
        let Some(flight) = flight else {
            return refuse(
                ErrorCode::NothingToReplyTo,
                "There is no message to reply to right now. To start a conversation, use \
                 `sbot send @handle` or `sbot send user`.",
            );
        };
        let to = if flight.sender == USER {
            Target::User
        } else {
            match self.store.get_bot(&flight.sender) {
                Ok(Some(bot)) => Target::Bot { bot: Box::new(bot), session: flight.sender_session.clone() },
                Ok(None) => {
                    return refuse(ErrorCode::UnknownRecipient, "The bot that sent this message no longer exists.")
                }
                Err(e) => return internal(e),
            }
        };
        self.post_from_bot(&from, to, body, files, Some(flight), PostKind::Reply).await
    }

    async fn bridge_send(&self, pane_id: &str, to: &str, body: String, files: Vec<String>) -> BridgeResponse {
        let Some((from, flight)) = self.caller(pane_id).await else { return not_a_bot() };
        let mut target = match self.recipient(&from.bot_id, to) {
            Ok(target) => target,
            Err(refusal) => return refusal,
        };
        if let (Target::Bot { bot, session }, Some(f)) = (&mut target, &flight) {
            if f.sender == bot.id {
                session.clone_from(&f.sender_session);
            }
        }
        self.post_from_bot(&from, target, body, files, flight, PostKind::Send).await
    }

    #[allow(clippy::result_large_err)]
    fn recipient(&self, from: &str, to: &str) -> std::result::Result<Target, BridgeResponse> {
        let name = to.trim().trim_start_matches('@').to_lowercase();
        if name == USER {
            return Ok(Target::User);
        }
        match self.store.get_bot_by_handle(&name) {
            Ok(Some(bot)) if bot.id == from => {
                Err(refuse(ErrorCode::SelfSend, "You cannot send a message to yourself."))
            }
            Ok(Some(bot)) => Ok(Target::Bot { bot: Box::new(bot), session: None }),
            Ok(None) => Err(refuse(
                ErrorCode::UnknownRecipient,
                format!("No bot named @{name}. Run `sbot bots` to see your teammates."),
            )),
            Err(e) => Err(internal(e)),
        }
    }

    pub(super) async fn post_from_bot(
        &self,
        from: &Caller,
        to: Target,
        body: String,
        files: Vec<String>,
        flight: Option<InFlight>,
        kind: PostKind,
    ) -> BridgeResponse {
        if body.trim().is_empty() {
            return refuse(
                ErrorCode::EmptyBody,
                "The message is empty. Pass the text on stdin: sbot send @handle <<'EOF' ... EOF",
            );
        }
        let sender = match self.store.get_bot(&from.bot_id) {
            Ok(Some(bot)) => bot,
            Ok(None) => return not_a_bot(),
            Err(e) => return internal(e),
        };
        if let Some(session) = to.session() {
            match self.store.get_session(session) {
                Ok(Some(session)) if session.archived_at.is_some() => return conversation_archived(),
                Ok(Some(_)) => {}
                Ok(None) => return conversation_removed(),
                Err(e) => return internal(e),
            }
        }
        let checked = match attachments::check(&files, &self.bot_directory(&sender)) {
            Ok(checked) => checked,
            Err(e) => return refuse(file_error_code(&e), format!("{e}.")),
        };
        let (chain_id, hop) = match &flight {
            Some(f) => (f.chain_id.clone(), f.hop + 1),
            None => (new_id(), 0),
        };
        if let Target::Bot { bot, .. } = &to {
            match self.limit_for(&chain_id, &from.bot_id, &bot.id) {
                Ok(Some(limit)) if hop > limit => {
                    self.limit_reached(&from.session_id, flight.as_ref(), limit).await;
                    let next = match self.master_to_report_to(&chain_id, &from.bot_id) {
                        Ok(Some(master)) => {
                            format!(", except @{master}: send it your result with `sbot send @{master}`.")
                        }
                        Ok(None) => "; send your result to the user with `sbot send user`.".to_string(),
                        Err(e) => return internal(e),
                    };
                    return refuse(
                        ErrorCode::ChainLimitReached,
                        format!(
                            "This exchange between bots has reached its limit of {limit} messages. Do not \
                             message other bots about it again{next}"
                        ),
                    );
                }
                Ok(_) => {}
                Err(e) => return internal(e),
            }
        }
        let id = new_id();
        let stored = match &to {
            Target::User => attachments::in_place(&checked),
            Target::Bot { .. } if checked.is_empty() => Vec::new(),
            Target::Bot { bot, .. } => match self.checked_bot_dir(&bot.handle) {
                Err(settings::FolderError::Outside(_)) => {
                    let who = self.who(&from.session_id);
                    tracing::error!(bot = %who.bot, session = who.session, message_id = %id, to = %bot.handle, "the recipient's folder leads outside the bots folder");
                    return refuse(
                        ErrorCode::Internal,
                        format!("@{}'s folder leads outside the bots folder, so the files were not sent.", bot.handle),
                    );
                }
                Err(settings::FolderError::Io(e)) => {
                    let who = self.who(&from.session_id);
                    tracing::error!(bot = %who.bot, session = who.session, message_id = %id, to = %bot.handle, error = %e, "could not open the recipient's folder");
                    return refuse(ErrorCode::Internal, format!("{e}."));
                }
                Ok(dir) => match attachments::copy_to_inbox(&checked, &dir, &id) {
                    Ok(copied) => copied,
                    Err(e) => {
                        let who = self.who(&from.session_id);
                        tracing::error!(bot = %who.bot, session = who.session, message_id = %id, to = %bot.handle, error = %e, "could not copy a bot's files into the recipient's inbox");
                        return refuse(ErrorCode::Internal, format!("{e}."));
                    }
                },
            },
        };
        let mut answers = match &kind {
            PostKind::Send | PostKind::Reply => {
                flight.filter(|f| f.sender == to.id() && f.sender_session.as_deref() == to.session())
            }
            PostKind::Ask { .. } | PostKind::Result { .. } => flight,
        };
        let previous = match &answers {
            Some(f) => match self.set_replied(&from.session_id, &f.message_id, true).await {
                Some(was) => Some(was),
                None if matches!(kind, PostKind::Reply) => {
                    return refuse(
                        ErrorCode::NothingToReplyTo,
                        "Your turn on that message already ended, so this reply was not sent. To add \
                         something, use `sbot send @handle` or `sbot send user`.",
                    )
                }
                None => {
                    answers = None;
                    None
                }
            },
            None => None,
        };
        let (in_reply_to, answered) = match &kind {
            PostKind::Send | PostKind::Reply => (answers.as_ref().map(|f| f.message_id.clone()), None),
            PostKind::Ask { .. } => (answers.as_ref().filter(|f| f.sender == USER).map(|f| f.message_id.clone()), None),
            PostKind::Result { request } => asking::result_answers(request, answers.as_ref(), to.session()),
        };
        let new = NewMessage {
            sender: from.bot_id.clone(),
            recipient: to.id().to_string(),
            sender_session: Some(from.session_id.clone()),
            recipient_session: to.session().map(str::to_string),
            kind: MessageKind::Text,
            body: body.trim_end().to_string(),
            system_code: None,
            system_params: None,
            attachments: stored,
            status: if matches!(to, Target::User) { MessageStatus::Delivered } else { MessageStatus::Queued },
            chain_id,
            hop,
            in_reply_to,
        };
        let inserted = match (&kind, &to) {
            (PostKind::Ask { options }, _) => {
                match self.store_question(from, &sender, &id, &new, options, answers.as_ref()) {
                    Ok(question) => return self.question_asked(from, question),
                    Err(e) => Err(e),
                }
            }
            (PostKind::Result { .. }, _) => {
                self.store.insert_result(&id, &new, answered.as_deref()).map(|(message, _)| (message, false))
            }
            (_, Target::Bot { session: None, .. }) => {
                self.store.insert_routed(&id, &new, Route::Shared).map(|routed| (routed.message, routed.created))
            }
            _ => self.store.insert_message_with_id(&id, &new).map(|message| (message, false)),
        };
        let (message, opened) = match inserted {
            Ok(inserted) => inserted,
            Err(e) => {
                if let (Some(f), Some(was)) = (&answers, previous) {
                    self.set_replied(&from.session_id, &f.message_id, was).await;
                }
                if let CoreError::Refused(Refusal::SessionNotFound(gone)) = &e {
                    return if *gone == from.session_id { not_a_bot() } else { conversation_removed() };
                }
                if let CoreError::Refused(Refusal::SessionArchived(archived)) = &e {
                    return if *archived == from.session_id { not_a_bot() } else { conversation_archived() };
                }
                let who = self.who(&from.session_id);
                tracing::error!(bot = %who.bot, session = who.session, error = %e, "could not store a bot's message");
                return internal(e);
            }
        };
        let response = BridgeResponse::sent(&message.id, to.label());
        let recipient_session = message.recipient_session.clone();
        if opened {
            self.emit_activity(&message, false);
        }
        if let (Some(session), true) = (&recipient_session, opened) {
            match self.store.get_session(session) {
                Ok(Some(session)) => self.emit(UiEvent::SessionAdded { session: self.session_view(session) }),
                Ok(None) => {}
                Err(e) => tracing::error!(session_id = %session, error = %e, "could not read the session just opened"),
            }
        }
        if opened {
            self.emit(UiEvent::MessageAdded { message });
        } else {
            self.emit_message(message);
        }
        if let Some(session) = recipient_session {
            if opened {
                let (who, asker) = (self.who(&session), self.who(&from.session_id));
                tracing::info!(bot = %who.bot, session = who.session, asker = %asker.bot, asker_session = asker.session, "opened a session for another bot's message");
            }
            tokio::spawn(self.pump_session(&session));
        }
        response
    }

    pub(super) async fn set_replied(&self, session_id: &str, message_id: &str, replied: bool) -> Option<bool> {
        let mut live = self.inner.live.lock().await;
        let flight = live.get_mut(session_id)?.in_flight.as_mut()?;
        (flight.message_id == message_id).then(|| std::mem::replace(&mut flight.replied, replied))
    }

    pub(crate) fn limit_for(&self, chain_id: &str, from_bot: &str, to_bot: &str) -> Result<Option<i64>> {
        if self.is_master(to_bot)? {
            return Ok(None);
        }
        let limit = self.hop_limit()?;
        let master_run = match self.master_id()? {
            Some(master) => from_bot == master || self.store.chain_has_party(chain_id, &master)?,
            None => false,
        };
        Ok(Some(if master_run { MASTER_HOP_FACTOR * limit } else { limit }))
    }

    fn master_to_report_to(&self, chain_id: &str, from_bot: &str) -> Result<Option<String>> {
        match self.master_bot()? {
            Some(master) if master.id != from_bot && self.store.chain_has_party(chain_id, &master.id)? => {
                Ok(Some(master.handle))
            }
            _ => Ok(None),
        }
    }

    async fn limit_reached(&self, session_id: &str, flight: Option<&InFlight>, limit: i64) {
        if let Some(f) = flight {
            let mut live = self.inner.live.lock().await;
            if let Some(current) = live.get_mut(session_id).and_then(|l| l.in_flight.as_mut()) {
                if current.message_id == f.message_id {
                    if current.limit_noticed {
                        return;
                    }
                    current.limit_noticed = true;
                }
            }
        }
        self.notice(session_id, "chain_limit_reached", json!({"limit": limit.to_string()}));
    }

    async fn bridge_bots(&self, pane_id: &str) -> BridgeResponse {
        let Some((from, _)) = self.caller(pane_id).await else { return not_a_bot() };
        let summaries = self.list_bots().await.and_then(|bots| {
            bots.into_iter()
                .filter(|v| v.bot.id != from.bot_id)
                .map(|v| {
                    Ok(BotSummary {
                        handle: v.bot.handle,
                        display_name: v.bot.display_name,
                        provider: v.bot.provider.as_str().to_string(),
                        status: status_word(v.status).to_string(),
                        role: role_line(&v.bot.instructions),
                        waiting_for_user: self.opened_session_waits(&v.bot.id, &from.session_id)?,
                    })
                })
                .collect::<Result<Vec<_>>>()
        });
        match summaries {
            Ok(bots) => BridgeResponse::bots(bots),
            Err(e) => refuse(ErrorCode::Internal, format!("Could not list the bots: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_role_is_the_first_non_empty_line_without_heading_marks() {
        assert_eq!(role_line("\n\n## Researcher\nFinds things."), "Researcher");
        assert_eq!(role_line("   "), "");
        assert_eq!(role_line(&"x".repeat(300)).len(), 120);
    }
}
