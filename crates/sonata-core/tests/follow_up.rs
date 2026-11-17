mod support;

use sonata_core::engine::UiEvent;
use sonata_core::model::*;
use sonata_core::runtime::{AgentState, RuntimeError, RuntimeEvent};
use support::*;

async fn answered(h: &Harness) -> (Bot, Message) {
    let (bot, hello) = bot_with_delivered(h, "hello").await;
    assert!(h.engine.handle_bridge(reply("w1:p1", "on it")).await.ok);
    finish_turn(h, "w1:p1").await;
    (bot, hello)
}

async fn wakes_up(h: &Harness, pane: &str) {
    h.rt.state().state = AgentState::Working;
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: pane.into(), state: AgentState::Working }).await;
    settle().await;
}

async fn settles(h: &Harness, pane: &str) {
    h.rt.state().state = AgentState::Idle;
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: pane.into(), state: AgentState::Idle }).await;
    settle().await;
}

fn from_bot(h: &Harness, bot: &Bot) -> Vec<Message> {
    let thread = h.engine.user_thread(&bot.id).unwrap();
    thread.into_iter().filter(|m| m.sender == bot.id && m.kind != MessageKind::System).collect()
}

#[tokio::test]
async fn work_the_cli_picks_up_on_its_own_is_a_turn_on_its_last_message() {
    let h = harness();
    let (bot, hello) = answered(&h).await;
    let session = latest_session(&h, &bot.id);
    let mut rx = h.engine.subscribe_ui();

    wakes_up(&h, "w1:p1").await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
    let seen = events(&mut rx);
    assert!(
        seen.iter().any(|e| matches!(e, UiEvent::SessionStatus { session_id, status: BotStatus::Working, .. } if *session_id == session)),
        "{seen:?}"
    );

    h.rt.state().output = "All 4341 specs pass.".into();
    settles(&h, "w1:p1").await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Idle);
    let said = from_bot(&h, &bot);
    let last = said.last().unwrap();
    assert_eq!(
        (last.kind, last.body.as_str(), last.in_reply_to.as_deref()),
        (MessageKind::RawFallback, "All 4341 specs pass.", Some(hello.id.as_str()))
    );
    assert!(events(&mut rx).iter().any(|e| matches!(e, UiEvent::MessageAdded { message } if message.id == last.id)));
    assert_eq!(stored(&h, &hello), MessageStatus::Delivered);
}

#[tokio::test]
async fn a_message_waits_while_the_cli_works_on_its_own() {
    let h = harness();
    let (bot, _) = answered(&h).await;
    wakes_up(&h, "w1:p1").await;
    h.engine.queue_user_message(&bot.id, "next").unwrap();
    h.engine.pump(&bot.id).await;
    assert_eq!(h.rt.state().prompts.len(), 1, "the follow-up turn is still on");
    settles(&h, "w1:p1").await;
    assert_eq!(h.rt.state().prompts.len(), 2, "delivered once it ends");
}

#[tokio::test]
async fn a_reply_during_follow_up_work_reaches_the_asker_and_needs_no_fallback() {
    let h = harness();
    let (bot, hello) = answered(&h).await;
    wakes_up(&h, "w1:p1").await;
    let sent = h.engine.handle_bridge(reply("w1:p1", "Done: all specs pass.")).await;
    assert!(sent.ok, "{:?}", sent.error);
    h.rt.state().output = "terminal text".into();
    settles(&h, "w1:p1").await;
    let said: Vec<(MessageKind, String, Option<String>)> =
        from_bot(&h, &bot).into_iter().map(|m| (m.kind, m.body, m.in_reply_to)).collect();
    assert_eq!(
        said,
        vec![
            (MessageKind::Text, "on it".to_string(), Some(hello.id.clone())),
            (MessageKind::Text, "Done: all specs pass.".to_string(), Some(hello.id.clone())),
        ]
    );
}

#[tokio::test]
async fn a_late_working_event_opens_no_turn() {
    let h = harness();
    let (bot, _) = answered(&h).await;
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: "w1:p1".into(), state: AgentState::Working }).await;
    settle().await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Idle);
    h.engine.watchdog_tick().await;
    settle().await;
    assert_eq!(from_bot(&h, &bot).len(), 1, "no stray fallback");
}

#[tokio::test]
async fn follow_up_work_on_a_bots_answer_gets_no_fallback() {
    let h = harness();
    let (researcher, _writer, _) = team(&h).await;
    assert!(h.engine.handle_bridge(send("w1:p1", "@writer", "Draft it.")).await.ok);
    settle().await;
    assert!(h.engine.handle_bridge(reply("w1:p1", "Asked @writer.")).await.ok);
    finish_turn(&h, "w1:p1").await;
    assert!(h.engine.handle_bridge(reply("w1:p2", "The draft.")).await.ok);
    finish_turn(&h, "w1:p2").await;
    finish_turn(&h, "w1:p1").await;

    wakes_up(&h, "w1:p1").await;
    assert_eq!(status_of(&h, &researcher.id).await, BotStatus::Working);
    h.rt.state().output = "Read the draft.".into();
    settles(&h, "w1:p1").await;
    let threads = h.engine.bot_threads().unwrap();
    let from_researcher: Vec<String> = h
        .engine
        .thread_messages(&threads[0].id)
        .unwrap()
        .into_iter()
        .filter(|m| m.sender == researcher.id)
        .map(|m| m.body)
        .collect();
    assert_eq!(from_researcher, vec!["Draft it.".to_string()], "an answer needs no answer");
}

#[tokio::test]
async fn follow_up_work_for_another_bot_answers_that_bot() {
    let h = harness();
    let (_researcher, writer, _) = team(&h).await;
    assert!(h.engine.handle_bridge(send("w1:p1", "@writer", "Draft it.")).await.ok);
    settle().await;
    assert!(h.engine.handle_bridge(reply("w1:p2", "Drafting.")).await.ok);
    finish_turn(&h, "w1:p2").await;
    wakes_up(&h, "w1:p2").await;
    h.rt.state().pane_outputs.insert("w1:p2".into(), "The draft is in draft.md.".into());
    settles(&h, "w1:p2").await;
    let threads = h.engine.bot_threads().unwrap();
    let from_writer: Vec<(MessageKind, String)> = h
        .engine
        .thread_messages(&threads[0].id)
        .unwrap()
        .into_iter()
        .filter(|m| m.sender == writer.id)
        .map(|m| (m.kind, m.body))
        .collect();
    assert_eq!(
        from_writer,
        vec![
            (MessageKind::Text, "Drafting.".to_string()),
            (MessageKind::RawFallback, "The draft is in draft.md.".to_string()),
        ]
    );
}

#[tokio::test]
async fn follow_up_work_is_followed_while_all_bots_are_paused() {
    let h = harness();
    let (bot, _) = answered(&h).await;
    h.engine.set_paused(true).await.unwrap();
    wakes_up(&h, "w1:p1").await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
    h.rt.state().output = "All specs pass.".into();
    settles(&h, "w1:p1").await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Idle);
    assert_eq!(from_bot(&h, &bot).last().unwrap().body, "All specs pass.");
}

#[tokio::test]
async fn stopping_follow_up_work_ends_it_and_still_shows_the_user_the_terminal() {
    let h = harness();
    let (bot, hello) = answered(&h).await;
    wakes_up(&h, "w1:p1").await;
    h.engine.stop_turn(&latest_session(&h, &bot.id)).await.unwrap();
    assert_eq!(h.rt.state().keys.last().unwrap().1, vec!["esc".to_string()]);
    h.rt.state().output = "Interrupted by user".into();
    settles(&h, "w1:p1").await;
    assert_eq!(stored(&h, &hello), MessageStatus::Delivered);
    let last = from_bot(&h, &bot).pop().unwrap();
    assert_eq!((last.kind, last.body.as_str()), (MessageKind::RawFallback, "Interrupted by user"));
}

#[tokio::test]
async fn closing_a_session_during_follow_up_work_keeps_its_message_answered() {
    let h = harness();
    let (bot, hello) = answered(&h).await;
    wakes_up(&h, "w1:p1").await;
    h.engine.stop_session(&latest_session(&h, &bot.id)).await.unwrap();
    assert_eq!(stored(&h, &hello), MessageStatus::Delivered, "no Retry for an answered message");
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Offline);
}

#[tokio::test]
async fn archiving_a_session_during_follow_up_work_keeps_its_message_answered() {
    let h = harness();
    let (bot, hello) = answered(&h).await;
    wakes_up(&h, "w1:p1").await;
    h.engine.archive_session(&latest_session(&h, &bot.id)).await.unwrap();
    assert_eq!(stored(&h, &hello), MessageStatus::Delivered);
    assert_eq!(h.rt.state().stopped, vec!["w1:p1".to_string()]);
}

#[tokio::test]
async fn restarting_a_session_during_follow_up_work_keeps_its_message_answered() {
    let h = harness();
    let (bot, hello) = answered(&h).await;
    wakes_up(&h, "w1:p1").await;
    h.engine.restart_session(&latest_session(&h, &bot.id)).await.unwrap();
    assert_eq!(stored(&h, &hello), MessageStatus::Delivered);
}

#[tokio::test]
async fn a_cli_that_exits_during_follow_up_work_leaves_its_message_answered() {
    let h = harness();
    let (bot, hello) = answered(&h).await;
    wakes_up(&h, "w1:p1").await;
    h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: "w1:p1".into() }).await;
    settle().await;
    assert_eq!(stored(&h, &hello), MessageStatus::Delivered);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Offline);
}

#[tokio::test]
async fn a_cli_that_vanishes_during_follow_up_work_leaves_its_message_answered() {
    let h = harness();
    let (bot, hello) = answered(&h).await;
    wakes_up(&h, "w1:p1").await;
    h.rt.state().inspect_failures.push_back(RuntimeError::Gone);
    h.engine.watchdog_tick().await;
    settle().await;
    assert_eq!(stored(&h, &hello), MessageStatus::Delivered);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Offline);
}

#[tokio::test]
async fn deleting_the_bot_during_follow_up_work_keeps_its_message_answered() {
    let h = harness();
    let (bot, hello) = answered(&h).await;
    wakes_up(&h, "w1:p1").await;
    h.engine.delete_bot(&bot.id).await.unwrap();
    assert_eq!(stored(&h, &hello), MessageStatus::Delivered, "its thread stays readable, answered");
}

#[tokio::test]
async fn follow_up_work_that_ends_with_a_shell_still_running_posts_nothing_yet() {
    let h = harness();
    let (bot, _) = answered(&h).await;
    wakes_up(&h, "w1:p1").await;
    h.rt.state().screen = CLAUDE_SHELL_RUNNING.into();
    h.rt.state().output = "Waiting on the final spec run.".into();
    settles(&h, "w1:p1").await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Idle);
    assert_eq!(from_bot(&h, &bot).len(), 1, "only its first reply");

    wakes_up(&h, "w1:p1").await;
    h.rt.state().screen = CLAUDE_NOTHING_RUNNING.into();
    h.rt.state().output = "All specs pass.".into();
    settles(&h, "w1:p1").await;
    let said = from_bot(&h, &bot);
    assert_eq!(said.len(), 2);
    assert_eq!((said[1].kind, said[1].body.as_str()), (MessageKind::RawFallback, "All specs pass."));
}

#[tokio::test]
async fn only_follow_up_work_that_ends_silently_sets_the_reminder() {
    let h = harness();
    let (bot, _) = answered(&h).await;
    wakes_up(&h, "w1:p1").await;
    h.rt.state().screen = CLAUDE_SHELL_RUNNING.into();
    settles(&h, "w1:p1").await;
    h.engine.queue_user_message(&bot.id, "status?").unwrap();
    h.engine.pump(&bot.id).await;
    assert!(!h.rt.state().prompts.last().unwrap().1.contains("Reminder:"), "it is not done yet");
    assert!(h.engine.handle_bridge(reply("w1:p1", "Still running.")).await.ok);
    finish_turn(&h, "w1:p1").await;

    wakes_up(&h, "w1:p1").await;
    h.rt.state().screen = CLAUDE_NOTHING_RUNNING.into();
    h.rt.state().output = "All specs pass.".into();
    settles(&h, "w1:p1").await;
    h.engine.queue_user_message(&bot.id, "next").unwrap();
    h.engine.pump(&bot.id).await;
    assert!(h.rt.state().prompts.last().unwrap().1.contains("Reminder:"));
}

#[tokio::test]
async fn follow_up_work_whose_screen_cannot_be_read_still_posts_its_fallback() {
    let h = harness();
    let (bot, _) = answered(&h).await;
    wakes_up(&h, "w1:p1").await;
    h.rt.state().snapshot_failures.push_back(RuntimeError::Other("herdr did not answer".into()));
    h.rt.state().output = "All specs pass.".into();
    settles(&h, "w1:p1").await;
    assert_eq!(from_bot(&h, &bot).last().unwrap().body, "All specs pass.");
}

#[tokio::test]
async fn a_delivered_turn_gets_its_fallback_even_with_a_shell_still_running() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    h.rt.state().screen = CLAUDE_SHELL_RUNNING.into();
    h.rt.state().output = "Started the specs in the background.".into();
    finish_turn(&h, "w1:p1").await;
    let last = from_bot(&h, &bot).pop().unwrap();
    assert_eq!((last.kind, last.body.as_str()), (MessageKind::RawFallback, "Started the specs in the background."));
}

#[tokio::test]
async fn a_missed_working_event_opens_the_follow_up_turn_at_the_next_tick() {
    let h = harness();
    let (bot, _) = answered(&h).await;
    h.rt.state().state = AgentState::Working;
    h.engine.watchdog_tick().await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
    h.rt.state().output = "All specs pass.".into();
    settles(&h, "w1:p1").await;
    assert_eq!(from_bot(&h, &bot).last().unwrap().body, "All specs pass.");
}
