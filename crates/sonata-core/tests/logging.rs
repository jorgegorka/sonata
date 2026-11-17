mod support;

use std::sync::Mutex;

use sonata_core::model::*;
use sonata_core::runtime::{RuntimeError, RuntimeEvent};
use support::*;

#[tokio::test]
async fn a_failed_start_is_logged_with_the_bot_handle_its_session_and_the_error() {
    let (log, _guard) = capture();
    let h = harness();
    h.rt.state().start_failures.push_back(RuntimeError::StartFailed("claude: command not found".into()));
    bot_with_delivered(&h, "hello").await;
    let text = log.text();
    assert!(text.contains("bot_start_failed") && text.contains("bot=researcher session=1"), "{text}");
    assert!(text.contains("command not found"), "{text}");
}

#[tokio::test]
async fn a_failed_delivery_is_logged_with_the_message_id() {
    let (log, _guard) = capture();
    let h = harness();
    h.rt.state().prompt_failures.push_back(RuntimeError::Other("herdr did not answer".into()));
    let (_, msg) = bot_with_delivered(&h, "hello").await;
    let text = log.text();
    assert!(text.contains(&format!("message_id={}", msg.id)), "{text}");
    assert!(text.contains("herdr did not answer"), "{text}");
}

#[tokio::test]
async fn a_failed_attachment_copy_is_logged_with_the_bot_handle_and_the_message_id() {
    let (log, _guard) = capture();
    let h = harness();
    let (researcher, writer, _) = team(&h).await;
    let file = h.paths.bot_dir(&researcher.handle).join("notes.md");
    std::fs::write(&file, "n").unwrap();
    std::fs::write(h.paths.bot_dir(&writer.handle).join("inbox"), "").unwrap();
    let request = bridge_proto::BridgeRequest {
        v: 1,
        pane_id: "w1:p1".into(),
        cli_session_id: None,
        command: bridge_proto::BridgeCommand::Send {
            to: "@writer".into(),
            body: "Notes.".into(),
            files: vec![file.to_string_lossy().into_owned()],
        },
    };
    assert!(!h.engine.handle_bridge(request).await.ok);
    let line = log.text().lines().find(|l| l.contains("could not copy")).map(str::to_string).unwrap_or_default();
    assert!(line.contains("bot=researcher") && line.contains("message_id="), "{}", log.text());
}

async fn turn_on_a_question() -> (Harness, String, String, String, String) {
    let ids = Mutex::new(Vec::new());
    let h = harness_after(|store| {
        let planner = store.insert_bot(&claude_bot("Planner"), "planner").unwrap();
        let reviewer = store.insert_bot(&claude_bot("Reviewer"), "reviewer").unwrap();
        let p1 = store.create_session(&planner.id, None, None).unwrap();
        let r1 = store.create_session(&reviewer.id, None, None).unwrap();
        let question = NewMessage {
            sender: planner.id,
            recipient: reviewer.id.clone(),
            sender_session: Some(p1.id.clone()),
            recipient_session: Some(r1.id.clone()),
            kind: MessageKind::Text,
            body: "Review the plan.".into(),
            attachments: vec![],
            system_code: None,
            system_params: None,
            status: MessageStatus::Queued,
            chain_id: sonata_core::ids::new_id(),
            hop: 0,
            in_reply_to: None,
        };
        let question = store.insert_message(&question).unwrap();
        *ids.lock().unwrap() = vec![p1.id, r1.id, reviewer.id, question.id];
    });
    let [p1, r1, reviewer, question]: [String; 4] = ids.into_inner().unwrap().try_into().unwrap();
    h.engine.pump_session(&r1).await;
    (h, p1, reviewer, r1, question)
}

async fn turn_on_a_removed_question() -> (Harness, String, String, String) {
    let (h, p1, reviewer, r1, question) = turn_on_a_question().await;
    h.engine.remove_session(&p1).await.unwrap();
    (h, reviewer, r1, question)
}

#[tokio::test]
async fn a_fallback_for_a_removed_session_is_dropped_with_a_log_line() {
    let (log, _guard) = capture();
    let (h, _, _, _) = turn_on_a_removed_question().await;
    h.rt.state().output = "Reviewed.".into();
    finish_turn(&h, "w1:p1").await;

    let text = log.text();
    let line = text.lines().find(|l| l.contains("the asking session was removed; no fallback")).unwrap_or_default();
    assert!(line.contains("INFO") && line.contains("bot=reviewer session=1"), "{text}");
    assert!(!text.contains("ERROR"), "{text}");
}

#[tokio::test]
async fn a_turn_on_a_removed_message_ends_with_an_info_line() {
    for case in ["stop", "restart", "exit", "vanished", "delete bot"] {
        let (log, _guard) = capture();
        let (h, reviewer, r1, question) = turn_on_a_removed_question().await;
        match case {
            "stop" => h.engine.stop_session(&r1).await.unwrap(),
            "restart" => h.engine.restart_session(&r1).await.unwrap(),
            "exit" => h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: "w1:p1".into() }).await,
            "vanished" => {
                h.rt.state().inspect_failures.push_back(RuntimeError::Gone);
                h.engine.check_stalled_turn(&r1, &question).await;
            }
            _ => drop(h.engine.delete_bot(&reviewer).await.unwrap()),
        }
        settle().await;

        let text = log.text();
        let line =
            text.lines().find(|l| l.contains("the message was removed with its conversation")).unwrap_or_default();
        assert!(line.contains("INFO") && line.contains("bot=reviewer") && line.contains(&question), "{case}: {text}");
        assert!(!text.contains("ERROR"), "{case}: {text}");
    }
}

#[tokio::test]
async fn a_fallback_for_an_archived_session_is_dropped_with_a_log_line() {
    let (log, _guard) = capture();
    let (h, p1, _, _, _) = turn_on_a_question().await;
    h.engine.archive_session(&p1).await.unwrap();
    h.rt.state().output = "Reviewed.".into();
    finish_turn(&h, "w1:p1").await;

    let text = log.text();
    let line = text.lines().find(|l| l.contains("the asking session was archived; no fallback")).unwrap_or_default();
    assert!(line.contains("INFO") && line.contains("bot=reviewer session=1"), "{text}");
    assert!(!text.contains("ERROR"), "{text}");
}

#[tokio::test]
async fn a_notice_for_an_archived_session_is_dropped_with_an_info_line() {
    let (log, _guard) = capture();
    let (h, p1, _, r1, _) = turn_on_a_question().await;
    h.engine.archive_session(&p1).await.unwrap();
    h.engine.stop_session(&r1).await.unwrap();
    settle().await;

    let text = log.text();
    let line = text.lines().find(|l| l.contains("no notice for an archived session")).unwrap_or_default();
    assert!(line.contains("INFO") && line.contains("bot_message_interrupted"), "{text}");
    assert!(!text.contains("ERROR"), "{text}");
}
