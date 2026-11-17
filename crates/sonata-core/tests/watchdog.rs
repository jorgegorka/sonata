mod support;

use std::time::Duration;

use sonata_core::model::*;
use sonata_core::runtime::{AgentState, RuntimeError, RuntimeEvent};
use support::*;

async fn idle_bot(h: &Harness) -> Bot {
    let (bot, _) = bot_with_delivered(h, "hello").await;
    h.engine.handle_bridge(reply("w1:p1", "hi")).await;
    finish_turn(h, "w1:p1").await;
    bot
}

fn codes(h: &Harness, bot_id: &str) -> Vec<String> {
    h.engine.user_thread(bot_id).unwrap().into_iter().filter_map(|m| m.system_code).collect()
}

#[tokio::test]
async fn an_idle_bot_whose_cli_vanished_goes_offline_and_restarts_with_the_next_message() {
    let h = harness();
    let bot = idle_bot(&h).await;
    h.rt.state().inspect_failures.push_back(RuntimeError::Gone);
    h.engine.watchdog_tick().await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Offline);
    assert_eq!(h.rt.state().stopped, vec!["w1:p1".to_string()]);
    h.engine.queue_user_message(&bot.id, "again").unwrap();
    h.engine.pump(&bot.id).await;
    assert_eq!(h.rt.state().prompts.last().unwrap().0, "w1:p2");
}

#[tokio::test]
async fn a_working_bot_whose_cli_vanished_interrupts_its_message() {
    let h = harness();
    let (bot, msg) = bot_with_delivered(&h, "hello").await;
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: "w1:p1".into(), state: AgentState::Working }).await;
    h.rt.state().inspect_failures.push_back(RuntimeError::Gone);
    h.engine.watchdog_tick().await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Offline);
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread.iter().find(|m| m.id == msg.id).unwrap().status, MessageStatus::Interrupted);
}

#[tokio::test]
async fn a_turn_never_seen_working_is_checked_again_until_it_ends() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    h.rt.state().state = AgentState::Unknown;
    h.engine.watchdog_tick().await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working, "unknown proves nothing");
    h.rt.state().state = AgentState::Idle;
    h.rt.state().output = "the answer".into();
    h.engine.watchdog_tick().await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working, "Enter first: the prompt may sit in the input");
    h.engine.watchdog_tick().await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Idle);
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert!(thread.iter().any(|m| m.kind == MessageKind::RawFallback && m.body == "the answer"));
}

#[tokio::test]
async fn a_dialog_nobody_reported_needs_attention_once() {
    let h = harness();
    let bot = idle_bot(&h).await;
    h.rt.state().state = AgentState::Blocked;
    h.engine.watchdog_tick().await;
    h.engine.watchdog_tick().await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::NeedsAttention);
    assert_eq!(codes(&h, &bot.id).iter().filter(|c| *c == "bot_blocked").count(), 1);
}

#[tokio::test]
async fn a_dialog_in_a_turn_never_seen_working_needs_attention() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    h.rt.state().state = AgentState::Blocked;
    h.engine.watchdog_tick().await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::NeedsAttention);
    assert_eq!(codes(&h, &bot.id).iter().filter(|c| *c == "bot_blocked").count(), 1);
}

#[tokio::test]
async fn a_turn_whose_idle_event_was_lost_ends_at_the_next_tick() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: "w1:p1".into(), state: AgentState::Working }).await;
    h.engine.handle_bridge(reply("w1:p1", "hi")).await;
    h.rt.state().state = AgentState::Idle;
    h.engine.watchdog_tick().await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Idle);
    h.engine.queue_user_message(&bot.id, "next").unwrap();
    h.engine.pump(&bot.id).await;
    assert_eq!(h.rt.state().prompts.len(), 2, "the bot takes the next message");
}

#[tokio::test]
async fn an_inspection_error_changes_nothing() {
    let h = harness();
    let bot = idle_bot(&h).await;
    h.rt.state().inspect_failures.push_back(RuntimeError::Other("herdr did not answer".into()));
    h.engine.watchdog_tick().await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Idle);
    assert!(h.rt.state().stopped.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_prompt_left_in_the_input_gets_enter_and_another_grace_period() {
    let h = harness_with_grace(Duration::from_secs(10));
    h.rt.state().output = "the screen".into();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    tokio::time::sleep(Duration::from_secs(10)).await;
    settle().await;
    assert_eq!(h.rt.state().keys, vec![("w1:p1".to_string(), vec!["enter".to_string()])]);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);

    tokio::time::sleep(Duration::from_secs(1)).await;
    h.engine.watchdog_tick().await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
    assert!(!h.engine.user_thread(&bot.id).unwrap().iter().any(|m| m.kind == MessageKind::RawFallback));

    tokio::time::sleep(Duration::from_secs(9)).await;
    settle().await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Idle);
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert!(thread.iter().any(|m| m.kind == MessageKind::RawFallback && m.body == "the screen"));
    assert_eq!(h.rt.state().keys.len(), 1, "Enter once");
}

#[tokio::test(start_paused = true)]
async fn the_watchdog_stops_when_the_app_quits() {
    let h = harness();
    let watchdog = h.engine.run_watchdog(Duration::from_secs(1));
    h.engine.shutdown().await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(watchdog.is_finished());
}

#[tokio::test(start_paused = true)]
async fn a_tick_while_the_prompt_is_still_being_sent_leaves_the_turn_alone() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Researcher")).unwrap();
    let msg = h.engine.queue_user_message(&bot.id, "hello").unwrap();
    h.rt.state().prompt_delay = Some(Duration::from_secs(5));
    let pump = tokio::spawn(h.engine.pump(&bot.id));
    while h.rt.state().prompts.is_empty() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!pump.is_finished(), "the prompt is still being sent");

    h.engine.watchdog_tick().await;

    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread.iter().find(|m| m.id == msg.id).unwrap().status, MessageStatus::Delivered);
    assert!(!thread.iter().any(|m| m.kind == MessageKind::RawFallback));
    assert!(!codes(&h, &bot.id).contains(&"bot_blocked".to_string()));
    pump.await.unwrap();
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
}

#[tokio::test]
async fn a_message_queued_behind_a_vanished_working_bot_is_delivered() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: "w1:p1".into(), state: AgentState::Working }).await;
    h.engine.queue_user_message(&bot.id, "next").unwrap();
    h.rt.state().inspect_failures.push_back(RuntimeError::Gone);
    h.engine.watchdog_tick().await;
    settle().await;
    let s = h.rt.state();
    assert_eq!(s.starts.len(), 2, "the bot started again for the waiting message");
    let (pane, text) = s.prompts.last().unwrap();
    assert_eq!(pane, "w1:p2");
    assert!(text.ends_with("\nnext"), "{text}");
}

#[tokio::test(start_paused = true)]
async fn the_idle_clock_restarts_when_work_the_cli_picked_up_on_its_own_ends() {
    let h = harness();
    let bot = idle_bot(&h).await;
    tokio::time::sleep(Duration::from_secs(25 * 60)).await;
    h.rt.state().state = AgentState::Working;
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: "w1:p1".into(), state: AgentState::Working }).await;
    h.rt.state().state = AgentState::Idle;
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: "w1:p1".into(), state: AgentState::Idle }).await;
    settle().await;

    tokio::time::sleep(Duration::from_secs(10 * 60)).await;
    h.engine.watchdog_tick().await;
    assert!(h.rt.state().stopped.is_empty(), "35 minutes after its delivered turn, but 10 after its own work");
    tokio::time::sleep(Duration::from_secs(21 * 60)).await;
    h.engine.watchdog_tick().await;
    assert_eq!(h.rt.state().stopped, vec!["w1:p1".to_string()]);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Offline);
}

#[tokio::test(start_paused = true)]
async fn a_shell_still_running_keeps_an_idle_session_and_restarts_its_clock() {
    let h = harness();
    let bot = idle_bot(&h).await;
    h.rt.state().screen = CLAUDE_SHELL_RUNNING.into();
    tokio::time::sleep(Duration::from_secs(31 * 60)).await;
    h.engine.watchdog_tick().await;
    assert!(h.rt.state().stopped.is_empty());
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Idle);
    h.rt.state().screen = CLAUDE_NOTHING_RUNNING.into();
    h.engine.watchdog_tick().await;
    assert!(h.rt.state().stopped.is_empty());
    tokio::time::sleep(Duration::from_secs(31 * 60)).await;
    h.engine.watchdog_tick().await;
    assert_eq!(h.rt.state().stopped, vec!["w1:p1".to_string()]);
}

#[tokio::test(start_paused = true)]
async fn an_idle_session_whose_cli_is_already_working_is_not_stopped() {
    let h = harness();
    let bot = idle_bot(&h).await;
    tokio::time::sleep(Duration::from_secs(31 * 60)).await;
    h.rt.state().state = AgentState::Working;
    h.engine.watchdog_tick().await;
    assert!(h.rt.state().stopped.is_empty());
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working, "the tick opened its follow-up turn");
}

#[tokio::test(start_paused = true)]
async fn an_idle_session_whose_screen_cannot_be_read_is_not_stopped() {
    let h = harness();
    let _bot = idle_bot(&h).await;
    h.rt.state().snapshot_failures.push_back(RuntimeError::Other("herdr did not answer".into()));
    tokio::time::sleep(Duration::from_secs(31 * 60)).await;
    h.engine.watchdog_tick().await;
    assert!(h.rt.state().stopped.is_empty());
    h.engine.watchdog_tick().await;
    assert_eq!(h.rt.state().stopped, vec!["w1:p1".to_string()]);
}

#[tokio::test(start_paused = true)]
async fn a_grok_session_keeps_the_plain_idle_stop() {
    let h = harness();
    let bot = h.engine.create_bot(NewBot { provider: Provider::Grok, ..claude_bot("Scout") }).unwrap();
    h.engine.queue_user_message(&bot.id, "hello").unwrap();
    h.engine.pump(&bot.id).await;
    assert!(h.engine.handle_bridge(reply("w1:p1", "hi")).await.ok);
    finish_turn(&h, "w1:p1").await;
    h.rt.state().screen = CLAUDE_SHELL_RUNNING.into();
    tokio::time::sleep(Duration::from_secs(31 * 60)).await;
    h.engine.watchdog_tick().await;
    assert_eq!(h.rt.state().stopped, vec!["w1:p1".to_string()]);
}
