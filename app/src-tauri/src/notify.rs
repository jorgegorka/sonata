use sonata_core::engine::UiEvent;
use sonata_core::model::{Bot, BotStatus, MessageKind, PauseReason, RunStatus, Session, USER};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::locale::t;

pub const BODY_CHARS: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

pub fn session_name(session_id: &str, lookup: impl Fn(&str) -> Option<(Bot, Session)>) -> Option<String> {
    let (bot, session) = lookup(session_id)?;
    let label = match own_title(&session) {
        Some(title) => title.to_string(),
        None => match session.opened_by.as_deref().and_then(&lookup) {
            Some((asker_bot, asker)) => t(
                "sessions.openedBy",
                &[
                    ("handle", &format!("@{}", asker_bot.handle)),
                    ("label", &own_title(&asker).map_or_else(|| numbered(&asker), str::to_string)),
                ],
            ),
            None => numbered(&session),
        },
    };
    Some(format!("{} · {label}", bot.display_name))
}

fn own_title(session: &Session) -> Option<&str> {
    session.title.as_deref().map(str::trim).filter(|title| !title.is_empty())
}

fn numbered(session: &Session) -> String {
    t("sessions.numbered", &[("n", &session.number.to_string())])
}

pub fn notice_for(
    event: &UiEvent,
    window_in_front: impl FnOnce() -> bool,
    session_name: impl Fn(&str) -> Option<String>,
) -> Option<Notice> {
    match event {
        UiEvent::MessageAdded { message }
            if message.recipient == USER
                && message.sender != USER
                && matches!(message.kind, MessageKind::Text | MessageKind::RawFallback) =>
        {
            if window_in_front() {
                return None;
            }
            let title = session_name(message.sender_session.as_deref()?)?;
            let text = message.body.split_whitespace().collect::<Vec<_>>().join(" ");
            let body = if text.is_empty() { t("notifications.files", &[]) } else { cut(&text, BODY_CHARS) };
            Some(Notice { title, body })
        }
        UiEvent::SessionStatus { session_id, status: BotStatus::NeedsAttention, .. } => {
            if window_in_front() {
                return None;
            }
            let name = session_name(session_id)?;
            Some(Notice {
                title: t("notifications.attentionTitle", &[("name", &name)]),
                body: t("notifications.attentionBody", &[]),
            })
        }
        UiEvent::WorkflowRunChanged { name, status: RunStatus::Paused, pause: Some(pause), step, .. } => {
            if window_in_front() {
                return None;
            }
            let kind = match pause {
                PauseReason::VisitCap { .. } => "visit_cap",
                PauseReason::NoResult { .. } => "no_result",
                PauseReason::BotMissing { .. } => "bot_missing",
            };
            let step = step.as_deref().unwrap_or(name);
            Some(Notice {
                title: t("notifications.workflowPausedTitle", &[("name", name)]),
                body: t(&format!("notifications.workflowPaused_{kind}"), &[("step", step)]),
            })
        }
        _ => None,
    }
}

fn cut(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

pub fn send(app: &AppHandle, notice: &Notice) {
    if let Err(e) = app.notification().builder().title(&notice.title).body(&notice.body).show() {
        tracing::warn!(error = %e, "could not show a notification");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sonata_core::model::{Attachment, Message, MessageStatus, Provider, Session, SessionView};

    fn message(sender: &str, recipient: &str, kind: MessageKind, body: &str, files: usize) -> Message {
        let from_session = (sender != USER).then(|| format!("s-{sender}"));
        Message {
            id: "m1".into(),
            thread_id: "t1".into(),
            sender: sender.into(),
            recipient: recipient.into(),
            sender_session: from_session,
            recipient_session: None,
            kind,
            body: body.into(),
            attachments: (0..files)
                .map(|i| Attachment { name: format!("f{i}.md"), path: format!("/f{i}.md") })
                .collect(),
            system_code: None,
            system_params: None,
            status: MessageStatus::Delivered,
            chain_id: "c".into(),
            hop: 0,
            in_reply_to: None,
            created_at: "t".into(),
            delivered_at: None,
            task_id: None,
            task_name: None,
            task_since: None,
            workflow_mark: None,
            workflow_request: false,
        }
    }

    fn session(number: i64, title: Option<&str>) -> Session {
        Session {
            id: "s-b1".into(),
            bot_id: "b1".into(),
            number,
            title: title.map(String::from),
            provider_session_id: None,
            session_config_version: None,
            opened_by: None,
            created_at: "t".into(),
            last_activity_at: "t".into(),
            archived_at: None,
            waiting_question_id: None,
            waiting_since: None,
            open_request_id: None,
            seen_at: None,
            waiting_options: vec![],
        }
    }

    fn bot(id: &str, name: &str) -> Bot {
        Bot {
            id: id.into(),
            handle: name.to_lowercase(),
            display_name: name.into(),
            color: "#3b82f6".into(),
            provider: Provider::Claude,
            model: None,
            instructions: String::new(),
            config_version: 1,
            skill_ids: vec![],
            created_at: "t".into(),
            updated_at: "t".into(),
        }
    }

    fn lookup(id: &str) -> Option<(Bot, Session)> {
        let researcher = |number: i64, title: Option<&str>, opened_by: Option<&str>| {
            let session = Session { id: id.into(), opened_by: opened_by.map(String::from), ..session(number, title) };
            Some((bot("b1", "Researcher"), session))
        };
        let planner = |number: i64, title: Option<&str>| {
            Some((bot("b2", "Planner"), Session { id: id.into(), bot_id: "b2".into(), ..session(number, title) }))
        };
        match id {
            "s-b1" => researcher(1, Some("Find the report"), None),
            "s-b2" => researcher(2, None, None),
            "s-b3" => researcher(3, None, Some("s-p1")),
            "s-b4" => researcher(4, None, Some("s-p2")),
            "s-b5" => researcher(5, None, Some("s-gone")),
            "s-b6" => researcher(6, Some("Sources"), Some("s-p1")),
            "s-p1" => planner(1, Some("Plan the launch")),
            "s-p2" => planner(2, Some("  ")),
            _ => None,
        }
    }

    fn names(id: &str) -> Option<String> {
        session_name(id, lookup)
    }

    fn added(message: Message) -> UiEvent {
        UiEvent::MessageAdded { message }
    }

    fn status(session_id: &str, status: BotStatus) -> UiEvent {
        UiEvent::SessionStatus { session_id: session_id.into(), bot_id: "b1".into(), status }
    }

    #[test]
    fn a_session_is_named_by_its_title_or_its_number() {
        assert_eq!(names("s-b1").as_deref(), Some("Researcher · Find the report"));
        assert_eq!(names("s-b2").as_deref(), Some("Researcher · Session 2"));
        assert_eq!(names("s-p2").as_deref(), Some("Planner · Session 2"), "a blank title counts as none");
        assert_eq!(names("s-gone"), None);
    }

    #[test]
    fn an_untitled_session_another_bot_opened_is_named_by_who_opened_it_as_on_the_board() {
        assert_eq!(names("s-b3").as_deref(), Some("Researcher · ↳ @planner · Plan the launch"));
        assert_eq!(names("s-b4").as_deref(), Some("Researcher · ↳ @planner · Session 2"));
        assert_eq!(names("s-b5").as_deref(), Some("Researcher · Session 5"), "its asker is gone");
        assert_eq!(names("s-b6").as_deref(), Some("Researcher · Sources"), "its own title comes first");
    }

    #[test]
    fn a_sessions_message_to_the_user_notifies_only_while_the_window_is_not_in_front() {
        let event = added(message("b1", USER, MessageKind::Text, "Done.\nSee the file.", 0));
        let expected = Notice { title: "Researcher · Find the report".into(), body: "Done. See the file.".into() };
        assert_eq!(notice_for(&event, || false, names), Some(expected));
        assert_eq!(notice_for(&event, || true, names), None);
        let raw = added(message("b1", USER, MessageKind::RawFallback, "raw", 0));
        assert_eq!(notice_for(&raw, || false, names).unwrap().body, "raw");
        let files = added(message("b1", USER, MessageKind::Text, "", 2));
        assert_eq!(notice_for(&files, || false, names).unwrap().body, "sent you files");
    }

    #[test]
    fn a_notification_names_the_session_as_the_board_does() {
        let reply = Message { sender_session: Some("s-b2".into()), ..message("b1", USER, MessageKind::Text, "hi", 0) };
        assert_eq!(notice_for(&added(reply), || false, names).unwrap().title, "Researcher · Session 2");
        let opened = Message { sender_session: Some("s-b3".into()), ..message("b1", USER, MessageKind::Text, "hi", 0) };
        assert_eq!(
            notice_for(&added(opened), || false, names).unwrap().title,
            "Researcher · ↳ @planner · Plan the launch"
        );
        let attention = notice_for(&status("s-b4", BotStatus::NeedsAttention), || false, names).unwrap();
        assert_eq!(attention.title, "Researcher · ↳ @planner · Session 2 needs attention");
    }

    #[test]
    fn nothing_for_notices_bot_threads_the_users_own_messages_or_unknown_sessions() {
        let system =
            Message { system_code: Some("session_reset".into()), ..message("b1", USER, MessageKind::System, "", 0) };
        assert_eq!(notice_for(&added(system), || false, names), None);
        assert_eq!(notice_for(&added(message("b1", "b2", MessageKind::Text, "hi", 0)), || false, names), None);
        assert_eq!(notice_for(&added(message(USER, "b1", MessageKind::Text, "hi", 0)), || false, names), None);
        assert_eq!(notice_for(&added(message("b9", USER, MessageKind::Text, "hi", 0)), || false, names), None);
        let sessionless = Message { sender_session: None, ..message("b1", USER, MessageKind::Text, "hi", 0) };
        assert_eq!(notice_for(&added(sessionless), || false, names), None);
        let updated = UiEvent::MessageUpdated { message: message("b1", USER, MessageKind::Text, "hi", 0) };
        assert_eq!(notice_for(&updated, || false, names), None);
    }

    #[test]
    fn other_session_events_notify_nothing() {
        let view = SessionView { session: session(1, None), status: BotStatus::NeedsAttention };
        let events = [
            UiEvent::SessionAdded { session: view.clone() },
            UiEvent::SessionUpdated { session: view },
            UiEvent::SessionRemoved { session_id: "s-b1".into(), bot_id: "b1".into(), thread_ids: vec!["t1".into()] },
        ];
        for event in &events {
            assert_eq!(notice_for(event, || false, names), None, "{event:?}");
        }
    }

    #[test]
    fn each_session_needing_attention_notifies_and_the_aggregate_does_not() {
        let notice = notice_for(&status("s-b1", BotStatus::NeedsAttention), || false, names).unwrap();
        assert_eq!(notice.title, "Researcher · Find the report needs attention");
        assert_eq!(notice.body, "Open Sonata to see what its terminal shows.");
        let second = notice_for(&status("s-b2", BotStatus::NeedsAttention), || false, names).unwrap();
        assert_eq!(second.title, "Researcher · Session 2 needs attention");
        assert_eq!(notice_for(&status("s-b1", BotStatus::NeedsAttention), || true, names), None);
        assert_eq!(notice_for(&status("s-b1", BotStatus::Working), || false, names), None);
        assert_eq!(notice_for(&status("s-gone", BotStatus::NeedsAttention), || false, names), None);
        let aggregate = UiEvent::BotStatus { bot_id: "b1".into(), status: BotStatus::NeedsAttention };
        assert_eq!(notice_for(&aggregate, || false, names), None);
        assert_eq!(notice_for(&UiEvent::Paused { paused: true }, || false, names), None);
    }

    #[test]
    fn the_window_is_asked_about_only_for_events_that_could_notify() {
        let asked = std::cell::Cell::new(0);
        let in_front = || {
            asked.set(asked.get() + 1);
            false
        };
        assert_eq!(notice_for(&UiEvent::Paused { paused: true }, in_front, names), None);
        let aggregate = UiEvent::BotStatus { bot_id: "b1".into(), status: BotStatus::NeedsAttention };
        assert_eq!(notice_for(&aggregate, in_front, names), None);
        assert_eq!(notice_for(&status("s-b1", BotStatus::Working), in_front, names), None);
        assert_eq!(notice_for(&added(message("b1", "b2", MessageKind::Text, "hi", 0)), in_front, names), None);
        assert_eq!(asked.get(), 0);
        assert!(notice_for(&added(message("b1", USER, MessageKind::Text, "hi", 0)), in_front, names).is_some());
        assert!(notice_for(&status("s-b1", BotStatus::NeedsAttention), in_front, names).is_some());
        assert_eq!(asked.get(), 2);
    }

    fn run_changed(status: RunStatus, pause: Option<PauseReason>, step: Option<&str>) -> UiEvent {
        UiEvent::WorkflowRunChanged {
            run_id: "r1".into(),
            name: "Ticket to MR".into(),
            status,
            pause,
            step: step.map(String::from),
            session_ids: vec![],
        }
    }

    #[test]
    fn a_paused_run_notifies_while_the_window_is_not_in_front() {
        let cap = run_changed(RunStatus::Paused, Some(PauseReason::VisitCap { step_id: "a".into() }), Some("Review"));
        let notice = notice_for(&cap, || false, names).unwrap();
        assert_eq!(notice.title, "Ticket to MR needs you");
        assert_eq!(notice.body, "Review already ran 3 times.");
        assert_eq!(notice_for(&cap, || true, names), None);
        let none = run_changed(RunStatus::Paused, Some(PauseReason::NoResult { step_id: "a".into() }), Some("Review"));
        assert_eq!(notice_for(&none, || false, names).unwrap().body, "Review ended without choosing an outcome.");
        let missing = run_changed(RunStatus::Paused, Some(PauseReason::BotMissing { step_id: "a".into() }), None);
        assert_eq!(notice_for(&missing, || false, names).unwrap().body, "Step Ticket to MR has no bot.");
    }

    #[test]
    fn a_running_or_finished_run_notifies_nothing() {
        for status in [RunStatus::Running, RunStatus::Finished, RunStatus::Stopped] {
            assert_eq!(notice_for(&run_changed(status, None, Some("Review")), || false, names), None, "{status:?}");
        }
        assert_eq!(notice_for(&UiEvent::WorkflowsChanged, || false, names), None);
    }

    #[test]
    fn a_long_message_with_accents_is_cut_at_a_character_boundary() {
        let long = added(message("b1", USER, MessageKind::Text, &"é".repeat(130), 0));
        let body = notice_for(&long, || false, names).unwrap().body;
        assert_eq!(body.chars().count(), BODY_CHARS + 1);
        assert!(body.ends_with('…') && body.starts_with("éé"));
        let exact = added(message("b1", USER, MessageKind::Text, &"é".repeat(BODY_CHARS), 0));
        assert_eq!(notice_for(&exact, || false, names).unwrap().body.chars().count(), BODY_CHARS);
    }
}
