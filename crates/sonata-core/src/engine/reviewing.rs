use super::archiving::{Archive, Block};
use super::messaging::{internal, refuse, status_word};
use super::*;
use bridge_proto::{BridgeResponse, ErrorCode};
use chrono::{DateTime, Local, Utc};

fn minute(stored: &str) -> String {
    match DateTime::parse_from_rfc3339(stored) {
        Ok(t) => local_minute(t.with_timezone(&Utc), &Local),
        Err(_) => stored.to_string(),
    }
}

fn title_of(session: &Session) -> String {
    format!("\"{}\"", session.title.clone().unwrap_or_else(|| format!("Session {}", session.number)))
}

fn app_label(bot: &Bot, session: &Session) -> String {
    match &session.title {
        Some(title) => format!("{} · {title}", bot.display_name),
        None => format!("{} #{}", bot.display_name, session.number),
    }
}

impl Engine {
    pub(super) fn session_ref(bot: &Bot, session: &Session) -> String {
        format!("@{}/{}", bot.handle, session.number)
    }

    pub(super) fn session_by_ref(&self, reference: &str) -> Result<Option<(Bot, Session)>> {
        let text = reference.trim().trim_start_matches('@');
        let Some((handle, number)) = text.split_once('/') else { return Ok(None) };
        let Ok(number) = number.parse::<i64>() else { return Ok(None) };
        let Some(bot) = self.store.get_bot_by_handle(&handle.to_lowercase())? else { return Ok(None) };
        let session = self.store.sessions_of(&bot.id)?.into_iter().find(|s| s.number == number);
        Ok(session.map(|s| (bot, s)))
    }

    fn end_label(&self, bot_id: &str, session_id: Option<&str>) -> String {
        if bot_id == USER {
            return "user".to_string();
        }
        let session = session_id.and_then(|id| self.store.get_session(id).ok().flatten());
        match (self.store.get_bot(bot_id), session) {
            (Ok(Some(bot)), Some(session)) => Self::session_ref(&bot, &session),
            (Ok(Some(bot)), None) => format!("@{}", bot.handle),
            _ => "a deleted bot".to_string(),
        }
    }

    fn session_label_by_id(&self, id: &str) -> String {
        match self.store.get_session(id) {
            Ok(Some(session)) => self.end_label(&session.bot_id, Some(id)),
            _ => "a deleted session".to_string(),
        }
    }

    pub(super) async fn bridge_sessions(&self, pane_id: &str, archived: bool, all: bool) -> BridgeResponse {
        if let Err(refusal) = self.master_caller(pane_id, "sessions").await {
            return refusal;
        }
        let listed = (|| -> Result<Vec<String>> {
            let mut found = Vec::new();
            for bot in self.store.list_bots()? {
                for session in self.store.sessions_of(&bot.id)? {
                    if all || session.archived_at.is_some() == archived {
                        found.push((bot.clone(), session));
                    }
                }
            }
            found.sort_by(|a, b| {
                (&b.1.last_activity_at, &b.1.created_at).cmp(&(&a.1.last_activity_at, &a.1.created_at))
            });
            let views: Vec<(Bot, SessionView)> =
                found.into_iter().map(|(bot, session)| (bot, self.session_view(session))).collect();
            Ok(views.iter().map(|(bot, view)| self.session_line(bot, view)).collect())
        })();
        match listed {
            Ok(lines) if lines.is_empty() => BridgeResponse::output("No sessions."),
            Ok(lines) => BridgeResponse::output(lines.join("\n")),
            Err(e) => internal(e),
        }
    }

    fn session_line(&self, bot: &Bot, view: &SessionView) -> String {
        let session = &view.session;
        let opener = match &session.opened_by {
            Some(id) => self.session_label_by_id(id),
            None => "the user".to_string(),
        };
        let mut line = format!(
            "{} · {} · {} · opened by {} · created {} · last activity {}",
            Self::session_ref(bot, session),
            title_of(session),
            status_word(view.status),
            opener,
            minute(&session.created_at),
            minute(&session.last_activity_at),
        );
        if let Some(at) = &session.archived_at {
            line.push_str(&format!(" · archived {}", minute(at)));
        }
        line
    }

    fn no_such_session(reference: &str) -> BridgeResponse {
        refuse(ErrorCode::BadRequest, format!("No session {reference}; run `sbot sessions`."))
    }

    pub(super) async fn bridge_session_show(&self, pane_id: &str, reference: &str) -> BridgeResponse {
        if let Err(refusal) = self.master_caller(pane_id, "session show").await {
            return refusal;
        }
        let (bot, session) = match self.session_by_ref(reference) {
            Ok(Some(found)) => found,
            Ok(None) => return Self::no_such_session(reference),
            Err(e) => return internal(e),
        };
        match self.session_transcript(&bot, &session) {
            Ok(text) => BridgeResponse::output(text),
            Err(e) => internal(e),
        }
    }

    fn session_transcript(&self, bot: &Bot, session: &Session) -> Result<String> {
        let store = &self.store;
        let mut out = format!(
            "{} · {}\ncreated {} · last activity {}",
            Self::session_ref(bot, session),
            title_of(session),
            minute(&session.created_at),
            minute(&session.last_activity_at)
        );
        if let Some(at) = &session.archived_at {
            out.push_str(&format!(" · archived {}", minute(at)));
        }
        let opener = match &session.opened_by {
            Some(id) => self.session_label_by_id(id),
            None => "the user".to_string(),
        };
        out.push_str(&format!("\nopened by: {opener}"));
        let mut opened = Vec::new();
        for other in store.list_bots()? {
            for s in store.sessions_of(&other.id)? {
                if s.opened_by.as_deref() == Some(session.id.as_str()) {
                    opened.push((s.created_at.clone(), Self::session_ref(&other, &s)));
                }
            }
        }
        opened.sort();
        if !opened.is_empty() {
            let refs: Vec<String> = opened.into_iter().map(|(_, r)| r).collect();
            out.push_str(&format!("\nopened: {}", refs.join(", ")));
        }
        out.push_str("\n---");
        for m in store.messages_of_session(&session.id)? {
            let at = minute(&m.created_at);
            if m.kind == MessageKind::System {
                let code = m.system_code.as_deref().unwrap_or("");
                match &m.system_params {
                    Some(params) => out.push_str(&format!("\n[{at}] notice: {code} {params}")),
                    None => out.push_str(&format!("\n[{at}] notice: {code}")),
                }
                continue;
            }
            let from = self.end_label(&m.sender, m.sender_session.as_deref());
            let to = self.end_label(&m.recipient, m.recipient_session.as_deref());
            out.push_str(&format!("\n[{at}] {from} → {to}"));
            if m.kind == MessageKind::RawFallback {
                out.push_str(" (raw terminal output)");
            }
            if m.status != MessageStatus::Delivered {
                out.push_str(&format!(" ({})", m.status.as_str()));
            }
            if let Some(task) = &m.task_name {
                out.push_str(&format!(" (recurring task \"{task}\")"));
            }
            if !m.body.is_empty() {
                out.push_str(&format!("\n{}", m.body));
            }
            if !m.attachments.is_empty() {
                let paths: Vec<&str> = m.attachments.iter().map(|a| a.path.as_str()).collect();
                out.push_str(&format!("\nattachments: {}", paths.join(", ")));
            }
        }
        Ok(out)
    }

    pub(super) async fn bridge_session_archive(&self, pane_id: &str, references: &[String]) -> BridgeResponse {
        let (caller, _) = match self.master_caller(pane_id, "session archive").await {
            Ok(found) => found,
            Err(refusal) => return refusal,
        };
        let mut lines = Vec::new();
        let mut failed = false;
        let mut archived_refs: Vec<String> = Vec::new();
        let mut archived_labels: Vec<String> = Vec::new();
        for reference in references {
            let (bot, session) = match self.session_by_ref(reference) {
                Ok(Some(found)) => found,
                Ok(None) => {
                    lines.push(format!("{reference}: no such session"));
                    failed = true;
                    continue;
                }
                Err(e) => return internal(e),
            };
            let name = Self::session_ref(&bot, &session);
            let current = match self.store.get_session(&session.id) {
                Ok(Some(s)) => s,
                Ok(None) => {
                    lines.push(format!("{reference}: no such session"));
                    failed = true;
                    continue;
                }
                Err(e) => return internal(e),
            };
            if current.archived_at.is_some() {
                lines.push(format!("{name}: already archived"));
                continue;
            }
            match self.archive_tree(&session.id, Some(&caller.session_id)).await {
                Ok(Archive::Done(archived)) => {
                    let refs: Vec<String> = archived.iter().map(|s| self.end_label(&s.bot_id, Some(&s.id))).collect();
                    let with = if refs.len() > 1 { format!(", with {}", refs[1..].join(", ")) } else { String::new() };
                    lines.push(format!("{name}: archived{with}"));
                    archived_refs.extend(refs);
                    archived_labels.push(app_label(&bot, &current));
                }
                Ok(Archive::Blocked(Block::Working)) => {
                    lines.push(format!("{name}: not archived: it is working"));
                    failed = true;
                }
                Ok(Archive::Blocked(Block::HoldsAsker)) => {
                    lines.push(format!("{name}: not archived: it contains this conversation"));
                    failed = true;
                }
                Err(CoreError::Refused(Refusal::SessionStarting)) => {
                    lines.push(format!("{name}: not archived: it is starting"));
                    failed = true;
                }
                Err(e) => return internal(e),
            }
        }
        if archived_refs.is_empty() {
            return if failed {
                refuse(ErrorCode::BadRequest, lines.join("\n"))
            } else {
                BridgeResponse::output(lines.join("\n"))
            };
        }
        let number = self.store.get_session(&caller.session_id).ok().flatten().map_or(0, |s| s.number);
        tracing::info!(bot = %self.handle_of(&caller.bot_id), session = number, archived = %archived_refs.join(", "), "Master archived sessions");
        self.notice(&caller.session_id, "master_archived_sessions", json!({"sessions": archived_labels.join(", ")}));
        BridgeResponse::output(lines.join("\n"))
    }
}
