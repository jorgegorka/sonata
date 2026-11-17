mod support;

use sonata_core::engine::UiEvent;
use sonata_core::model::*;
use sonata_core::runtime::RuntimeEvent;
use sonata_core::{CoreError, Refusal};
use support::*;

#[tokio::test]
async fn deleting_a_working_bot_stops_it_and_cancels_its_messages() {
    let h = harness();
    let (bot, first) = bot_with_delivered(&h, "first").await;
    let second = h.engine.queue_user_message(&bot.id, "second").unwrap();

    let folder = h.engine.delete_bot(&bot.id).await.unwrap();

    assert_eq!(folder, h.paths.bot_dir("researcher"));
    assert!(folder.is_dir(), "the engine leaves the folder to the app, which moves it to the Trash");
    assert_eq!(h.rt.state().stopped, vec!["w1:p1".to_string()]);
    assert!(h.engine.list_bots().await.unwrap().is_empty());
    let thread = h.engine.thread_messages(&first.thread_id).unwrap();
    for id in [&first.id, &second.id] {
        assert_eq!(thread.iter().find(|m| &m.id == id).unwrap().status, MessageStatus::Cancelled);
    }
    h.engine.pump(&bot.id).await;
    assert_eq!(h.rt.state().starts.len(), 1, "a deleted bot is never started again");
    let err = h.engine.delete_bot(&bot.id).await.unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::BotNotFound(_))), "{err}");
}

#[tokio::test]
async fn deleting_a_bot_its_teammates_talk_to_stops_the_traffic_and_keeps_the_thread() {
    let h = harness();
    let (researcher, _writer, _) = team(&h).await;
    h.engine.handle_bridge(send("w1:p1", "@writer", "Draft.")).await;
    h.engine.handle_bridge(send("w1:p1", "@writer", "Also a title.")).await;
    settle().await;
    let thread = h.engine.bot_threads().unwrap().remove(0);
    let before: Vec<_> = h.engine.thread_messages(&thread.id).unwrap().into_iter().map(|m| m.status).collect();
    assert_eq!(before, vec![MessageStatus::Delivered, MessageStatus::Queued]);

    h.engine.delete_bot(&researcher.id).await.unwrap();
    h.rt.state().output = "Here is the draft.".into();
    finish_turn(&h, "w1:p2").await;

    let after = h.engine.thread_messages(&thread.id).unwrap();
    assert_eq!(after.len(), 2, "no answer was queued to the deleted bot: {after:?}");
    assert_eq!(after[1].status, MessageStatus::Cancelled, "what waited from it was cancelled");
    assert_eq!(h.engine.bot_threads().unwrap().len(), 1, "the thread stays readable");
    assert_eq!(h.rt.state().prompts.len(), 2, "Writer got nothing more");
}

#[tokio::test]
async fn a_starting_bot_cannot_be_deleted() {
    let h = harness();
    h.rt.state().hang_starts = true;
    let bot = h.engine.create_bot(claude_bot("Researcher")).unwrap();
    h.engine.queue_user_message(&bot.id, "hello").unwrap();
    let pump = tokio::spawn(h.engine.pump(&bot.id));
    settle().await;
    let err = h.engine.delete_bot(&bot.id).await.unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::BotStarting)), "{err}");
    pump.abort();
}

#[tokio::test]
async fn a_deleted_bots_name_is_free_once_its_folder_is_gone() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Researcher")).unwrap();
    let folder = h.engine.delete_bot(&bot.id).await.unwrap();
    let twin = h.engine.create_bot(claude_bot("Researcher")).unwrap();
    assert_ne!(twin.handle, "researcher", "its folder, with its MEMORY.md, is still there");
    std::fs::remove_dir_all(&folder).unwrap();
    assert_eq!(h.engine.create_bot(claude_bot("Researcher")).unwrap().handle, "researcher");
}

#[tokio::test]
async fn a_message_cut_off_after_the_delete_cannot_be_retried() {
    let h = harness();
    let (researcher, _writer, _) = team(&h).await;
    h.engine.handle_bridge(send("w1:p1", "@writer", "Draft.")).await;
    settle().await;
    let thread = h.engine.bot_threads().unwrap().remove(0);
    let draft = h.engine.thread_messages(&thread.id).unwrap().remove(0);
    assert_eq!(draft.status, MessageStatus::Delivered);

    h.engine.delete_bot(&researcher.id).await.unwrap();
    h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: "w1:p2".into() }).await;
    settle().await;

    let after = h.engine.thread_messages(&thread.id).unwrap();
    assert_eq!(after[0].status, MessageStatus::Interrupted);
    let err = h.engine.retry_message(&draft.id).unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::BotNotFound(_))), "{err}");
    assert_eq!(h.engine.thread_messages(&thread.id).unwrap().len(), after.len(), "no copy was stored");
    let notices = h.engine.user_thread(&researcher.id).unwrap();
    assert!(notices.iter().all(|m| m.system_code.as_deref() != Some("bot_message_interrupted")), "{notices:?}");
}

#[tokio::test]
async fn interrupted_messages_of_a_deleted_bot_are_cancelled() {
    let h = harness();
    let (bot, first) = bot_with_delivered(&h, "first").await;
    h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: "w1:p1".into() }).await;
    settle().await;
    assert_eq!(h.engine.thread_messages(&first.thread_id).unwrap()[0].status, MessageStatus::Interrupted);

    h.engine.delete_bot(&bot.id).await.unwrap();

    assert_eq!(h.engine.thread_messages(&first.thread_id).unwrap()[0].status, MessageStatus::Cancelled);
    assert!(h.engine.retry_message(&first.id).is_err());
}

#[tokio::test]
async fn deleting_a_bot_removes_its_sessions_and_keeps_its_bot_threads() {
    let h = harness();
    let (researcher, writer, _) = team(&h).await;
    assert!(h.engine.handle_bridge(send("w1:p1", "@writer", "Draft.")).await.ok);
    settle().await;
    h.engine.start_session_with_message(&researcher.id, "Another topic.", &[]).unwrap();
    let mut sessions: Vec<String> = h.engine.sessions_of(&researcher.id).unwrap().into_iter().map(|s| s.id).collect();
    sessions.sort();
    assert_eq!(sessions.len(), 2);
    let thread = h.engine.bot_threads().unwrap().remove(0);
    let mut rx = h.engine.subscribe_ui();

    h.engine.delete_bot(&researcher.id).await.unwrap();

    assert!(h.engine.sessions_of(&researcher.id).unwrap().is_empty());
    let mut removed: Vec<(String, String, Vec<String>)> = std::iter::from_fn(|| rx.try_recv().ok())
        .filter_map(|e| match e {
            UiEvent::SessionRemoved { session_id, bot_id, thread_ids } => Some((session_id, bot_id, thread_ids)),
            _ => None,
        })
        .collect();
    removed.sort();
    let expected: Vec<_> = sessions.iter().map(|id| (id.clone(), researcher.id.clone(), vec![])).collect();
    assert_eq!(removed, expected, "each of its sessions, with no thread: the threads stay");
    let threads: Vec<String> = h.engine.bot_threads().unwrap().into_iter().map(|t| t.id).collect();
    assert_eq!(threads, vec![thread.id.clone()], "its thread with Writer stays readable");
    let kept = h.engine.thread_messages(&thread.id).unwrap();
    assert_eq!(kept.iter().map(|m| m.body.as_str()).collect::<Vec<_>>(), vec!["Draft."]);
    let theirs = h.engine.sessions_of(&writer.id).unwrap();
    assert_eq!(theirs.len(), 1, "the session it opened in Writer stays");
    assert_eq!(theirs[0].opened_by, None, "no longer opened by a session");
}
