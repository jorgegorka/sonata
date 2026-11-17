mod support;

use sonata_core::engine::UiEvent;
use sonata_core::model::*;
use sonata_core::runtime::{AgentState, RuntimeError, RuntimeEvent};
use sonata_core::store::Store;
use sonata_core::{CoreError, Refusal};
use support::*;

#[tokio::test]
async fn marking_a_session_seen_stamps_it_and_tells_the_ui() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    let session = latest_session(&h, &bot.id);
    let mut rx = h.engine.subscribe_ui();
    h.engine.mark_session_seen(&session).unwrap();
    let updated = events(&mut rx).into_iter().find_map(|e| match e {
        UiEvent::SessionUpdated { session } => Some(session),
        _ => None,
    });
    assert!(updated.expect("an update").session.seen_at.is_some());
}

#[tokio::test]
async fn marking_an_unknown_session_seen_is_refused() {
    let h = harness();
    let err = h.engine.mark_session_seen("nope").unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::SessionNotFound(_))));
}

fn only<T>(mut items: Vec<T>) -> T {
    assert_eq!(items.len(), 1);
    items.remove(0)
}

async fn activity(h: &Harness, session: &str) -> SessionActivity {
    only(h.engine.session_activity(&[session.to_string()]).await.unwrap())
}

#[tokio::test]
async fn activity_names_the_message_in_flight_and_when_its_turn_started() {
    let h = harness();
    let (bot, hello) = bot_with_delivered(&h, "\n\nFix the login bug\nDetails follow").await;
    let a = activity(&h, &latest_session(&h, &bot.id)).await;
    let line = a.working_on.expect("in flight");
    assert_eq!(line.text, "Fix the login bug");
    let stored = h.engine.thread_messages(&hello.thread_id).unwrap().into_iter().find(|m| m.id == hello.id).unwrap();
    assert_eq!(Some(line.at), stored.delivered_at);
}

#[tokio::test]
async fn activity_names_a_files_only_message_in_flight() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Researcher")).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("notes.md");
    std::fs::write(&file, "# Notes").unwrap();
    let msg = h.engine.queue_user_message_with_files(&bot.id, "", &[file.to_string_lossy().into_owned()]).unwrap();
    h.engine.pump(&bot.id).await;
    let stored = h.engine.thread_messages(&msg.thread_id).unwrap().into_iter().find(|m| m.id == msg.id).unwrap();
    let line = activity(&h, &latest_session(&h, &bot.id)).await.working_on;
    assert_eq!(line, Some(CardLine { text: String::new(), at: stored.delivered_at.unwrap() }));
}

#[tokio::test]
async fn activity_counts_the_queue() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "first").await;
    h.engine.queue_user_message(&bot.id, "second").unwrap();
    h.engine.pump(&bot.id).await;
    assert_eq!(activity(&h, &latest_session(&h, &bot.id)).await.queued, 1);
}

#[tokio::test]
async fn activity_has_the_question_in_full() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    let session = h.engine.sessions_of(&bot.id).unwrap().remove(0);
    let body = "Which database?\nPostgres or SQLite?";
    assert!(h.engine.handle_bridge(ask(&pane(&h, &session), body)).await.ok);
    assert_eq!(activity(&h, &session.id).await.question.as_deref(), Some(body));
}

#[tokio::test]
async fn activity_has_the_latest_hand_off() {
    let h = harness();
    let ([_, reviewer, _], [p1, _, _], _) = chain(&h).await;
    let sent = activity(&h, &p1.id).await.last_sent.expect("sent");
    assert_eq!((sent.recipient, sent.text), (reviewer.id, "Review the plan.".to_string()));
}

#[tokio::test]
async fn activity_marks_a_cut_off_message_until_a_new_one_lands() {
    let h = harness();
    let (bot, hello) = bot_with_delivered(&h, "hello").await;
    let session = latest_session(&h, &bot.id);
    h.engine.restart_session(&session).await.unwrap();
    let cut = activity(&h, &session).await.cut_off.expect("cut off");
    assert_eq!((cut.message_id.as_str(), cut.sender.as_str()), (hello.id.as_str(), "user"));
    h.engine.retry_message(&hello.id).unwrap();
    assert_eq!(activity(&h, &session).await.cut_off, None);
}

#[tokio::test]
async fn activity_has_the_last_reply_and_when_the_user_last_got_something() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    let session = h.engine.sessions_of(&bot.id).unwrap().remove(0);
    assert!(h.engine.handle_bridge(reply(&pane(&h, &session), "Done.\nAll green.")).await.ok);
    let msgs = h.engine.session_messages(&session.id).unwrap();
    let sent = msgs.iter().rev().find(|m| m.body == "Done.\nAll green.").unwrap();
    let a = activity(&h, &session.id).await;
    let last = a.last_reply.expect("reply");
    assert_eq!((last.text.as_str(), last.raw), ("Done.\nAll green.", false));
    assert_eq!(a.last_for_user_at.as_deref(), Some(sent.created_at.as_str()));
}

const BANNER: &str = " ▐▛███▛█   Claude Code v2.1.318\n▝▜██████▀  Opus 5.5\nHanded it to @reviewer.\n";

#[tokio::test]
async fn raw_output_is_not_what_a_session_last_sent() {
    let h = harness();
    let ([_, reviewer, _], [p1, _, _], [p_pane, _, _]) = chain(&h).await;
    h.rt.state().pane_outputs.insert(p_pane.clone(), BANNER.into());
    finish_turn(&h, &p_pane).await;
    let msgs = h.engine.session_messages(&p1.id).unwrap();
    assert!(msgs.iter().any(|m| m.kind == MessageKind::RawFallback), "{msgs:?}");
    let sent = activity(&h, &p1.id).await.last_sent.expect("the hand-off");
    assert_eq!((sent.recipient, sent.text), (reviewer.id, "Review the plan.".to_string()));
}

#[tokio::test]
async fn raw_output_as_the_final_answer_is_marked_raw() {
    let h = harness();
    h.rt.state().output = BANNER.into();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    finish_turn(&h, "w1:p1").await;
    let a = activity(&h, &latest_session(&h, &bot.id)).await;
    assert_eq!(a.last_sent, None, "the session sent nothing of its own");
    let last = a.last_reply.expect("the raw output is its final answer");
    assert!(last.raw);
    assert!(last.text.contains("Handed it to @reviewer."), "{}", last.text);
}

#[tokio::test]
async fn a_reply_with_only_files_keeps_their_names() {
    let h = harness_after(|_| {});
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    let session = h.engine.sessions_of(&bot.id).unwrap().remove(0);
    let mut m = NewMessage::system(&session, "x", serde_json::json!({}));
    m.kind = MessageKind::Text;
    m.system_code = None;
    m.system_params = None;
    m.attachments = vec![Attachment { name: "report.md".into(), path: "/tmp/report.md".into() }];
    let stored = Store::open(&h.paths.db_path()).unwrap().insert_message(&m).unwrap();
    let last = activity(&h, &session.id).await.last_reply.expect("reply");
    assert_eq!((last.text.as_str(), last.files), ("", vec!["report.md".to_string()]));
    assert_eq!(last.message_id, stored.id);
}

#[tokio::test]
async fn activity_names_the_recurring_task_a_run_belongs_to() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let task = h
        .engine
        .create_task(NewTask {
            name: "Wiki update".into(),
            bot_id: bot.id.clone(),
            prompt: "Update the wiki.".into(),
            every_days: 7,
            at: "09:00".into(),
            starts_on: None,
        })
        .unwrap();
    let run = h.engine.run_task_now(&task.id).unwrap();
    settle().await;
    assert_eq!(activity(&h, &run.id).await.run_of.as_deref(), Some("Wiki update"));
    let plain = h.engine.create_bot(claude_bot("Plain")).unwrap();
    h.engine.queue_user_message(&plain.id, "hello").unwrap();
    h.engine.pump(&plain.id).await;
    assert_eq!(activity(&h, &latest_session(&h, &plain.id)).await.run_of, None);
}

#[tokio::test]
async fn activity_skips_sessions_that_are_gone() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    let ids = [latest_session(&h, &bot.id), "gone".to_string()];
    assert_eq!(h.engine.session_activity(&ids).await.unwrap().len(), 1);
}

async fn glances(h: &Harness, session: &str) -> Vec<Glance> {
    h.engine.session_glances(&[session.to_string()]).await
}

#[tokio::test]
async fn a_glance_reads_the_step_of_a_working_session() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    h.rt.state().screen = include_str!("fixtures/screens/claude-working.txt").into();
    let glance = only(glances(&h, &latest_session(&h, &bot.id)).await);
    assert_eq!(glance.step.as_deref(), Some("Reading shared memory file"));
    assert!(!glance.background_work);
    assert_eq!(glance.tail, None);
}

#[tokio::test]
async fn a_glance_sees_background_work_of_an_idle_session() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    h.rt.state().screen = CLAUDE_SHELL_RUNNING.into();
    finish_turn(&h, "w1:p1").await;
    let glance = only(glances(&h, &latest_session(&h, &bot.id)).await);
    assert!(glance.background_work);
    assert_eq!(glance.step, None);
    assert_eq!(glance.tail, None);
}

#[tokio::test]
async fn a_glance_has_the_last_lines_of_a_dialog() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    let screen: Vec<String> = (1..=10).map(|i| format!("line {i}")).collect();
    h.rt.state().screen = format!("{}\n\n", screen.join("\n\n"));
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: "w1:p1".into(), state: AgentState::Blocked }).await;
    let glance = only(glances(&h, &latest_session(&h, &bot.id)).await);
    assert_eq!(glance.tail.as_deref(), Some("line 5\nline 6\nline 7\nline 8\nline 9\nline 10"));
    assert_eq!(glance.step, None);
    assert!(!glance.background_work);
}

#[tokio::test]
async fn a_glance_skips_a_stopped_session() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    let session = latest_session(&h, &bot.id);
    h.engine.stop_session(&session).await.unwrap();
    assert!(glances(&h, &session).await.is_empty());
    assert!(glances(&h, "nope").await.is_empty());
}

async fn delivered_on(h: &Harness, provider: Provider) -> String {
    let bot = h.engine.create_bot(NewBot { provider, ..claude_bot("Coder") }).unwrap();
    h.engine.queue_user_message(&bot.id, "hello").unwrap();
    h.engine.pump(&bot.id).await;
    latest_session(h, &bot.id)
}

async fn reads_screen(h: &Harness, session: &str) -> bool {
    h.rt.state().snapshot_failures.push_back(RuntimeError::Other("not read".into()));
    glances(h, session).await;
    let read = h.rt.state().snapshot_failures.is_empty();
    h.rt.state().snapshot_failures.clear();
    read
}

#[tokio::test]
async fn a_glance_reads_a_codex_screen_only_for_its_dialog() {
    let h = harness();
    let session = delivered_on(&h, Provider::Codex).await;
    assert!(!reads_screen(&h, &session).await, "codex's screen names no step");
    finish_turn(&h, "w1:p1").await;
    assert!(!reads_screen(&h, &session).await, "codex never wakes for its background work");
    assert!(glances(&h, &session).await.is_empty());
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: "w1:p1".into(), state: AgentState::Blocked }).await;
    assert!(reads_screen(&h, &session).await, "a dialog's last lines are always read");
}

#[tokio::test]
async fn a_glance_reads_a_grok_screen_only_while_it_works() {
    let h = harness();
    let session = delivered_on(&h, Provider::Grok).await;
    assert!(reads_screen(&h, &session).await, "grok names its step");
    finish_turn(&h, "w1:p1").await;
    assert!(!reads_screen(&h, &session).await, "grok's screen shows no background work");
}

#[tokio::test]
async fn a_glance_skips_a_session_whose_screen_cannot_be_read() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    h.rt.state().snapshot_failures.push_back(RuntimeError::Other("herdr did not answer".into()));
    assert!(h.engine.session_glances(&[latest_session(&h, &bot.id)]).await.is_empty());
}
