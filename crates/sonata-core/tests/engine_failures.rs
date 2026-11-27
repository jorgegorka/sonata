#![allow(clippy::await_holding_lock)]
mod support;

use sonata_core::model::*;
use sonata_core::runtime::{AgentState, RuntimeError, RuntimeEvent};
use support::*;

fn state(pane: &str, s: AgentState) -> RuntimeEvent {
    RuntimeEvent::State { pane_id: pane.into(), state: s }
}

fn codes(thread: &[Message]) -> Vec<String> {
    thread.iter().filter_map(|m| m.system_code.clone()).collect()
}

#[tokio::test]
async fn watchdog_ends_a_turn_whose_working_state_was_missed() {
    let h = harness();
    h.rt.state().output = "fast answer".into();
    let (bot, msg) = bot_with_delivered(&h, "hello").await;
    h.rt.state().state = AgentState::Idle;
    h.engine.check_stalled_turn(&latest_session(&h, &bot.id), &msg.id).await;
    h.engine.check_stalled_turn(&latest_session(&h, &bot.id), &msg.id).await;
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread[1].kind, MessageKind::RawFallback);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Idle);
    assert_eq!(h.rt.state().keys, vec![("w1:p1".to_string(), vec!["enter".to_string()])], "Enter once, not twice");
}

#[tokio::test]
async fn watchdog_presses_enter_when_the_prompt_was_typed_but_not_sent() {
    let h = harness();
    let (bot, msg) = bot_with_delivered(&h, "hello").await;
    let session = latest_session(&h, &bot.id);
    h.rt.state().state = AgentState::Idle;
    h.engine.check_stalled_turn(&session, &msg.id).await;

    assert_eq!(h.rt.state().keys, vec![("w1:p1".to_string(), vec!["enter".to_string()])]);
    assert_eq!(h.engine.user_thread(&bot.id).unwrap().len(), 1, "no fallback: the turn is still open");
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);

    h.engine.on_runtime_event(state("w1:p1", AgentState::Working)).await;
    assert!(h.engine.handle_bridge(reply("w1:p1", "hi")).await.ok);
    h.engine.on_runtime_event(state("w1:p1", AgentState::Idle)).await;
    h.engine.check_stalled_turn(&session, &msg.id).await;
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread.iter().map(|m| m.kind).collect::<Vec<_>>(), [MessageKind::Text, MessageKind::Text]);
    assert_eq!(thread[1].body, "hi");
    assert_eq!(h.rt.state().keys.len(), 1);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Idle);
}

#[tokio::test]
async fn watchdog_ends_a_replied_turn_without_pressing_enter() {
    let h = harness();
    let (bot, msg) = bot_with_delivered(&h, "hello").await;
    assert!(h.engine.handle_bridge(reply("w1:p1", "fast")).await.ok);
    h.rt.state().state = AgentState::Idle;
    h.engine.check_stalled_turn(&latest_session(&h, &bot.id), &msg.id).await;

    assert!(h.rt.state().keys.is_empty());
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Idle);
    assert_eq!(h.engine.user_thread(&bot.id).unwrap().len(), 2, "the reply, no fallback");
}

#[tokio::test]
async fn watchdog_ends_the_turn_when_enter_cannot_be_pressed() {
    let h = harness();
    h.rt.state().output = "screen".into();
    let (bot, msg) = bot_with_delivered(&h, "hello").await;
    h.rt.state().state = AgentState::Idle;
    h.rt.state().key_failures.push_back(RuntimeError::Other("agent.send_keys: herdr did not answer".into()));
    h.engine.check_stalled_turn(&latest_session(&h, &bot.id), &msg.id).await;

    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread[1].kind, MessageKind::RawFallback);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Idle);
}

#[tokio::test]
async fn watchdog_records_working_so_a_later_idle_ends_the_turn() {
    let h = harness();
    let (bot, msg) = bot_with_delivered(&h, "hello").await;
    h.rt.state().state = AgentState::Working;
    h.engine.check_stalled_turn(&latest_session(&h, &bot.id), &msg.id).await;
    assert_eq!(h.engine.user_thread(&bot.id).unwrap().len(), 1);
    h.engine.on_runtime_event(state("w1:p1", AgentState::Idle)).await;
    assert_eq!(h.engine.user_thread(&bot.id).unwrap().len(), 2);
}

#[tokio::test]
async fn watchdog_takes_a_vanished_agent_offline_and_interrupts_its_turn() {
    use sonata_core::engine::UiEvent;

    let h = harness();
    let (bot, msg) = bot_with_delivered(&h, "hello").await;
    h.engine.queue_user_message(&bot.id, "next").unwrap();
    let mut ui = h.engine.subscribe_ui();
    h.rt.state().inspect_failures.push_back(RuntimeError::Gone);
    h.engine.check_stalled_turn(&latest_session(&h, &bot.id), &msg.id).await;
    settle().await;

    let status =
        |body: &str| h.engine.user_thread(&bot.id).unwrap().into_iter().find(|m| m.body == body).unwrap().status;
    assert_eq!(status("hello"), MessageStatus::Interrupted, "maybe partly processed: not requeued");
    assert_eq!(status("next"), MessageStatus::Delivered);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
    let s = h.rt.state();
    assert_eq!(s.stopped, vec!["w1:p1".to_string()]);
    assert_eq!((s.starts.len(), s.prompts.len()), (2, 2));
    assert!(s.starts[1].args.contains(&"--resume".to_string()));
    let (pane, text) = s.prompts.last().unwrap();
    assert_eq!((pane.as_str(), text.lines().nth(1)), ("w1:p2", Some("next")));
    drop(s);
    let (mut interrupted, mut offline) = (false, false);
    while let Ok(event) = ui.try_recv() {
        match event {
            UiEvent::MessageUpdated { message } => {
                interrupted |= message.id == msg.id && message.status == MessageStatus::Interrupted
            }
            UiEvent::BotStatus { status, .. } => offline |= status == BotStatus::Offline,
            UiEvent::MessageAdded { .. } | UiEvent::Paused { .. } => {}
            UiEvent::SessionStatus { .. }
            | UiEvent::SessionAdded { .. }
            | UiEvent::SessionUpdated { .. }
            | UiEvent::SessionRemoved { .. }
            | UiEvent::ProjectsChanged
            | UiEvent::SkillsChanged
            | UiEvent::TasksChanged
            | UiEvent::WorkflowsChanged
            | UiEvent::WorkflowRunChanged { .. }
            | UiEvent::BotAdded { .. }
            | UiEvent::BotUpdated { .. }
            | UiEvent::UsageLimits { .. }
            | UiEvent::WorkItemsChanged
            | UiEvent::OrchestrationChanged { .. } => {}
        }
    }
    assert!(interrupted && offline);
    h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: "w1:p1".into() }).await;
    settle().await;
    assert_eq!(h.rt.state().starts.len(), 2);
    assert_eq!(status("next"), MessageStatus::Delivered);
    assert!(!h.engine.handle_bridge(reply("w1:p1", "late")).await.ok);
}

#[tokio::test]
async fn watchdog_leaves_the_turn_alone_when_herdr_does_not_answer() {
    let h = harness();
    let (bot, msg) = bot_with_delivered(&h, "hello").await;
    h.rt.state().inspect_failures.push_back(RuntimeError::Other("agent.get: herdr did not answer within 30s".into()));
    h.engine.check_stalled_turn(&latest_session(&h, &bot.id), &msg.id).await;
    settle().await;

    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread.iter().find(|m| m.id == msg.id).unwrap().status, MessageStatus::Delivered);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
    assert!(h.rt.state().stopped.is_empty());
    assert!(h.engine.handle_bridge(reply("w1:p1", "still here")).await.ok, "the turn is still open");
}

#[tokio::test]
async fn exit_mid_turn_interrupts_and_the_next_message_restarts() {
    let h = harness();
    let (bot, msg) = bot_with_delivered(&h, "hello").await;
    h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: "w1:p1".into() }).await;
    settle().await;
    let first = h.engine.user_thread(&bot.id).unwrap().into_iter().find(|m| m.id == msg.id).unwrap();
    assert_eq!(first.status, MessageStatus::Interrupted);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Offline);

    h.engine.queue_user_message(&bot.id, "are you back?").unwrap();
    h.engine.pump(&bot.id).await;
    let s = h.rt.state();
    assert_eq!(s.starts.len(), 2);
    assert_eq!(s.prompts.last().unwrap().0, "w1:p2");
}

async fn offline_bot_with_saved_session(h: &Harness) -> Bot {
    h.rt.state().session_id = Some("sess-1".into());
    let (bot, _) = bot_with_delivered(h, "hello").await;
    h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: "w1:p1".into() }).await;
    settle().await;
    h.rt.state().session_id = None;
    assert_eq!(saved_session(h, &bot.id).await.as_deref(), Some("sess-1"));
    bot
}

fn count(thread: &[Message], code: &str) -> usize {
    codes(thread).iter().filter(|c| *c == code).count()
}

#[tokio::test]
async fn an_exit_does_not_hold_up_the_event_loop_with_the_restart() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    h.engine.queue_user_message(&bot.id, "next").unwrap();
    h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: "w1:p1".into() }).await;
    assert_eq!(h.rt.state().starts.len(), 1, "the restart runs in its own task");
    settle().await;
    let s = h.rt.state();
    assert_eq!(s.starts.len(), 2);
    let (pane, text) = s.prompts.last().unwrap();
    assert_eq!((pane.as_str(), text.lines().nth(1)), ("w1:p2", Some("next")));
    drop(s);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
}

#[tokio::test]
async fn resume_uses_saved_session_and_falls_back_to_a_fresh_one() {
    let h = harness();
    let bot = offline_bot_with_saved_session(&h).await;

    h.rt.state().start_failures.push_back(RuntimeError::StartFailed("no such session".into()));
    h.engine.queue_user_message(&bot.id, "again").unwrap();
    h.engine.pump(&bot.id).await;

    let s = h.rt.state();
    assert_eq!(s.starts.len(), 3);
    assert!(s.starts[1].args.ends_with(&["--resume".to_string(), "sess-1".to_string()]));
    assert!(!s.starts[2].args.contains(&"--resume".to_string()));
    assert_eq!(s.prompts.len(), 2);
    let fresh = s.starts[2].args.iter().skip_while(|a| *a != "--session-id").nth(1).cloned();
    drop(s);
    assert!(fresh.is_some());
    assert_eq!(saved_session(&h, &bot.id).await, fresh);
    assert_eq!(count(&h.engine.user_thread(&bot.id).unwrap(), "session_reset"), 1);
}

#[tokio::test]
async fn a_failed_first_start_stores_no_session_id_and_a_successful_one_stores_its_own() {
    let h = harness();
    h.rt.state().start_failures.push_back(RuntimeError::StartFailed("claude not found".into()));
    let (bot, _) = bot_with_delivered(&h, "first").await;
    assert_eq!(saved_session(&h, &bot.id).await, None, "nothing to resume yet");

    h.engine.pump(&bot.id).await;
    let s = h.rt.state();
    assert_eq!(s.starts.len(), 2);
    assert!(!s.starts[1].args.contains(&"--resume".to_string()), "a fresh session again");
    let chosen = |i: usize| s.starts[i].args.iter().skip_while(|a| *a != "--session-id").nth(1).cloned();
    assert_ne!(chosen(0), chosen(1), "a new id each time");
    let second = chosen(1);
    drop(s);
    assert!(second.is_some());
    assert_eq!(saved_session(&h, &bot.id).await, second);
    assert_eq!(count(&h.engine.user_thread(&bot.id).unwrap(), "session_reset"), 0);
}

#[tokio::test]
async fn a_quit_during_the_first_start_leaves_no_session_to_resume() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Researcher")).unwrap();
    h.engine.queue_user_message(&bot.id, "first").unwrap();
    h.rt.state().hang_starts = true;
    let starting = tokio::spawn(h.engine.pump(&bot.id));
    settle().await;
    assert_eq!(h.rt.state().starts.len(), 1, "the start is under way");
    assert!(h.rt.state().starts[0].args.contains(&"--session-id".to_string()));
    assert_eq!(saved_session(&h, &bot.id).await, None);
    starting.abort();
}

#[tokio::test]
async fn shutdown_stops_pumping_so_quit_neither_restarts_nor_resets_bots() {
    let h = harness();
    h.rt.state().session_id = Some("sess-1".into());
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    h.engine.queue_user_message(&bot.id, "queued at quit").unwrap();
    for _ in 0..2 {
        h.rt.state().start_failures.push_back(RuntimeError::Other("the runtime has shut down".into()));
    }

    h.engine.shutdown().await;
    h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: "w1:p1".into() }).await;
    settle().await;
    h.engine.pump(&bot.id).await;

    assert!(h.rt.state().shutdown_called);
    assert_eq!(h.rt.state().starts.len(), 1);
    assert_eq!(saved_session(&h, &bot.id).await.as_deref(), Some("sess-1"));
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(count(&thread, "session_reset") + count(&thread, "bot_start_failed"), 0);
    assert_eq!(thread.iter().find(|m| m.body == "queued at quit").unwrap().status, MessageStatus::Queued);
}

#[tokio::test]
async fn a_broken_runtime_does_not_reset_the_saved_session() {
    let h = harness();
    let bot = offline_bot_with_saved_session(&h).await;

    h.rt.state().start_failures.push_back(RuntimeError::Other("herdr speaks protocol 19".into()));
    h.engine.queue_user_message(&bot.id, "again").unwrap();
    h.engine.pump(&bot.id).await;

    assert_eq!(h.rt.state().starts.len(), 2, "no retry without resume");
    assert_eq!(saved_session(&h, &bot.id).await.as_deref(), Some("sess-1"));
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(count(&thread, "session_reset"), 0);
    assert_eq!(count(&thread, "bot_start_failed"), 1);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::NeedsAttention);
}

#[tokio::test]
async fn saved_session_survives_when_the_fresh_start_fails_too() {
    let h = harness();
    let bot = offline_bot_with_saved_session(&h).await;

    for _ in 0..2 {
        h.rt.state().start_failures.push_back(RuntimeError::StartFailed("claude not found".into()));
    }
    h.engine.queue_user_message(&bot.id, "again").unwrap();
    h.engine.pump(&bot.id).await;

    assert_eq!(h.rt.state().starts.len(), 3);
    assert_eq!(saved_session(&h, &bot.id).await.as_deref(), Some("sess-1"));
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(count(&thread, "session_reset"), 0);
    assert_eq!(count(&thread, "bot_start_failed"), 1);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::NeedsAttention);
}

#[tokio::test]
async fn start_failure_flags_the_bot_and_the_next_message_retries() {
    let h = harness();
    h.rt.state().start_failures.push_back(RuntimeError::StartFailed("claude not found".into()));
    let (bot, first) = bot_with_delivered(&h, "first").await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::NeedsAttention);
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert!(codes(&thread).contains(&"bot_start_failed".to_string()));
    assert_eq!(thread.iter().find(|m| m.id == first.id).unwrap().status, MessageStatus::Queued);

    h.engine.queue_user_message(&bot.id, "second").unwrap();
    h.engine.pump(&bot.id).await;
    let s = h.rt.state();
    assert_eq!(s.starts.len(), 2);
    assert!(s.prompts[0].1.ends_with("\nfirst"));
}

#[tokio::test]
async fn prompt_failure_requeues_the_message() {
    let h = harness();
    h.rt.state().prompt_failures.push_back(RuntimeError::Blocked);
    h.rt.state().inspect_failures.push_back(RuntimeError::Other("not consulted".into()));
    let (bot, msg) = bot_with_delivered(&h, "hello").await;
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread.iter().find(|m| m.id == msg.id).unwrap().status, MessageStatus::Queued);
    assert!(codes(&thread).contains(&"prompt_failed".to_string()));
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::NeedsAttention);
    assert!(h.rt.state().stopped.is_empty());
    h.rt.state().inspect_failures.clear();

    h.engine.pump(&bot.id).await;
    assert_eq!(h.rt.state().prompts.len(), 2);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
}

#[tokio::test]
async fn prompt_to_a_vanished_agent_takes_the_bot_offline_until_the_next_message() {
    let h = harness();
    let gone = || RuntimeError::Other("herdr error agent_not_found: agent target w1:p1 not found".into());
    h.rt.state().prompt_failures.push_back(gone());
    h.rt.state().inspect_failures.push_back(RuntimeError::Gone);
    let (bot, msg) = bot_with_delivered(&h, "hello").await;
    let saved = saved_session(&h, &bot.id).await.expect("session id");

    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread.iter().find(|m| m.id == msg.id).unwrap().status, MessageStatus::Queued);
    assert_eq!(count(&thread, "prompt_failed"), 1);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Offline);
    assert_eq!(h.rt.state().stopped, vec!["w1:p1".to_string()]);

    h.engine.queue_user_message(&bot.id, "are you there?").unwrap();
    h.engine.pump(&bot.id).await;
    let s = h.rt.state();
    assert_eq!(s.starts.len(), 2);
    assert!(s.starts[1].args.ends_with(&["--resume".to_string(), saved]));
    let (pane, text) = s.prompts.last().unwrap();
    assert_eq!((pane.as_str(), text.lines().nth(1)), ("w1:p2", Some("hello")));
}

#[tokio::test]
async fn a_prompt_failure_takes_the_bot_offline_only_if_the_agent_is_gone() {
    let h = harness();
    let hiccup = || RuntimeError::Other("agent.get: herdr did not answer within 30s".into());
    h.rt.state().prompt_failures.push_back(hiccup());
    h.rt.state().inspect_failures.push_back(hiccup());
    let (bot, msg) = bot_with_delivered(&h, "hello").await;

    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread.iter().find(|m| m.id == msg.id).unwrap().status, MessageStatus::Queued);
    assert_eq!(count(&thread, "prompt_failed"), 1);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::NeedsAttention);
    assert!(h.rt.state().stopped.is_empty());

    h.engine.pump(&bot.id).await;
    let s = h.rt.state();
    assert_eq!(s.starts.len(), 1, "no restart");
    assert_eq!(s.prompts.last().unwrap().0, "w1:p1");
}

#[tokio::test]
async fn prompt_failure_with_a_live_agent_keeps_the_bot_running() {
    let h = harness();
    h.rt.state().prompt_failures.push_back(RuntimeError::Other("herdr hiccup".into()));
    let (bot, msg) = bot_with_delivered(&h, "hello").await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::NeedsAttention);
    assert!(h.rt.state().stopped.is_empty());

    h.engine.pump(&bot.id).await;
    let s = h.rt.state();
    assert_eq!(s.starts.len(), 1);
    assert_eq!(s.prompts.len(), 2);
    assert_eq!(s.prompts[1].0, "w1:p1");
    drop(s);
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread.iter().find(|m| m.id == msg.id).unwrap().status, MessageStatus::Delivered);
}

#[tokio::test]
async fn relaunch_interrupts_cut_off_turns_and_delivers_the_queue() {
    use sonata_core::engine::UiEvent;
    use sonata_core::ids::new_id;

    let ids = std::sync::Mutex::new(Vec::new());
    let h = harness_after(|store| {
        let bot = store.insert_bot(&claude_bot("Researcher"), "researcher").unwrap();
        let session = store.create_session(&bot.id, None, None).unwrap();
        let to_bot = |body: &str, status| NewMessage {
            sender: USER.into(),
            recipient: bot.id.clone(),
            sender_session: None,
            recipient_session: Some(session.id.clone()),
            kind: MessageKind::Text,
            body: body.into(),
            attachments: vec![],
            system_code: None,
            system_params: None,
            status,
            chain_id: new_id(),
            hop: 0,
            in_reply_to: None,
        };
        let answered = store.insert_message(&to_bot("answered", MessageStatus::Delivered)).unwrap();
        store
            .insert_message(&NewMessage {
                sender: bot.id.clone(),
                recipient: USER.into(),
                sender_session: Some(session.id.clone()),
                recipient_session: None,
                in_reply_to: Some(answered.id.clone()),
                ..to_bot("the answer", MessageStatus::Delivered)
            })
            .unwrap();
        let cut = store.insert_message(&to_bot("cut off by the quit", MessageStatus::Delivered)).unwrap();
        let waiting = store.insert_message(&to_bot("still waiting", MessageStatus::Queued)).unwrap();
        *ids.lock().unwrap() = vec![bot.id, answered.id, cut.id, waiting.id];
    });
    let [bot_id, answered, cut, waiting] = ids.into_inner().unwrap().try_into().unwrap();
    let mut ui = h.engine.subscribe_ui();

    h.engine.recover().await.unwrap();

    let status = |id: &str| h.engine.user_thread(&bot_id).unwrap().into_iter().find(|m| m.id == id).unwrap().status;
    assert_eq!(status(&answered), MessageStatus::Delivered);
    assert_eq!(status(&cut), MessageStatus::Interrupted);
    assert_eq!(status(&waiting), MessageStatus::Delivered);
    let s = h.rt.state();
    assert_eq!(s.starts.len(), 1);
    assert_eq!(s.prompts.len(), 1);
    assert_eq!(s.prompts[0].1.lines().nth(1), Some("still waiting"));
    drop(s);
    let mut interrupted_event = false;
    while let Ok(event) = ui.try_recv() {
        interrupted_event |= matches!(event, UiEvent::MessageUpdated { message }
            if message.id == cut && message.status == MessageStatus::Interrupted);
    }
    assert!(interrupted_event);
    assert_eq!(status_of(&h, &bot_id).await, BotStatus::Working);
}

#[tokio::test]
async fn blocked_flags_attention_once_and_working_restores() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    h.engine.on_runtime_event(state("w1:p1", AgentState::Blocked)).await;
    h.engine.on_runtime_event(state("w1:p1", AgentState::Blocked)).await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::NeedsAttention);
    let blocked = codes(&h.engine.user_thread(&bot.id).unwrap()).iter().filter(|c| *c == "bot_blocked").count();
    assert_eq!(blocked, 1);
    h.engine.on_runtime_event(state("w1:p1", AgentState::Working)).await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
}

#[tokio::test]
async fn session_id_reported_later_is_saved_at_turn_end() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    h.rt.state().session_id = Some("later".into());
    h.engine.on_runtime_event(state("w1:p1", AgentState::Working)).await;
    h.engine.on_runtime_event(state("w1:p1", AgentState::Idle)).await;
    h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: "w1:p1".into() }).await;
    settle().await;
    h.engine.queue_user_message(&bot.id, "again").unwrap();
    h.engine.pump(&bot.id).await;
    assert!(h.rt.state().starts[1].args.ends_with(&["--resume".to_string(), "later".to_string()]));
}

#[tokio::test]
async fn stale_watchdog_does_not_end_the_next_messages_turn() {
    let h = harness();
    let (bot, first) = bot_with_delivered(&h, "one").await;
    h.engine.queue_user_message(&bot.id, "two").unwrap();
    h.engine.on_runtime_event(state("w1:p1", AgentState::Working)).await;
    h.engine.on_runtime_event(state("w1:p1", AgentState::Idle)).await;
    settle().await;
    assert_eq!(h.rt.state().prompts.len(), 2);
    let len_before = h.engine.user_thread(&bot.id).unwrap().len();

    h.rt.state().state = AgentState::Idle;
    h.engine.check_stalled_turn(&latest_session(&h, &bot.id), &first.id).await;

    assert_eq!(h.engine.user_thread(&bot.id).unwrap().len(), len_before);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
    assert!(h.engine.handle_bridge(reply("w1:p1", "ok")).await.ok);
}

#[tokio::test(start_paused = true)]
async fn a_stalled_prompt_is_retried_once_when_the_agent_stayed_idle() {
    let h = harness();
    h.rt.state().prompt_failures.push_back(RuntimeError::Stalled);
    let (bot, msg) = bot_with_delivered(&h, "hello").await;
    assert_eq!(h.rt.state().prompts.len(), 2, "retried after the pause");
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread.iter().find(|m| m.id == msg.id).unwrap().status, MessageStatus::Delivered);
}

#[tokio::test(start_paused = true)]
async fn a_stalled_prompt_the_cli_took_is_not_sent_again() {
    for state in [AgentState::Working, AgentState::Blocked, AgentState::Unknown] {
        let h = harness();
        {
            let mut s = h.rt.state();
            s.prompt_failures.push_back(RuntimeError::Stalled);
            s.state = state;
        }
        let (bot, msg) = bot_with_delivered(&h, "hello").await;
        assert_eq!(h.rt.state().prompts.len(), 1, "{state:?}: sent once");
        let thread = h.engine.user_thread(&bot.id).unwrap();
        assert_eq!(thread.iter().find(|m| m.id == msg.id).unwrap().status, MessageStatus::Delivered, "{state:?}");
        assert!(!codes(&thread).contains(&"prompt_failed".to_string()), "{state:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_prompt_that_stalls_twice_goes_back_to_the_queue() {
    let h = harness();
    for _ in 0..2 {
        h.rt.state().prompt_failures.push_back(RuntimeError::Stalled);
    }
    let (bot, msg) = bot_with_delivered(&h, "hello").await;
    assert_eq!(h.rt.state().prompts.len(), 2);
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread.iter().find(|m| m.id == msg.id).unwrap().status, MessageStatus::Queued);
    assert!(codes(&thread).contains(&"prompt_failed".to_string()));
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::NeedsAttention);
}

#[tokio::test(start_paused = true)]
async fn a_stalled_prompt_whose_state_cannot_be_read_stays_delivered() {
    let h = harness();
    {
        let mut s = h.rt.state();
        s.prompt_failures.push_back(RuntimeError::Stalled);
        s.inspect_failures.push_back(RuntimeError::Other("herdr did not answer".into()));
    }
    let (bot, msg) = bot_with_delivered(&h, "hello").await;
    settle().await;
    assert_eq!(h.rt.state().prompts.len(), 1, "sent once");
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread.iter().find(|m| m.id == msg.id).unwrap().status, MessageStatus::Delivered);
    assert!(!codes(&thread).contains(&"prompt_failed".to_string()));
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
}

#[tokio::test(start_paused = true)]
async fn a_stalled_prompt_to_a_vanished_agent_goes_back_to_the_queue_and_the_bot_offline() {
    let h = harness();
    {
        let mut s = h.rt.state();
        s.prompt_failures.push_back(RuntimeError::Stalled);
        s.inspect_failures.extend([RuntimeError::Gone, RuntimeError::Gone]);
    }
    let (bot, msg) = bot_with_delivered(&h, "hello").await;
    settle().await;
    let s = h.rt.state();
    assert_eq!(s.prompts.len(), 1, "not re-sent to a gone agent");
    assert_eq!(s.stopped, vec!["w1:p1".to_string()], "the leftover shell is closed");
    drop(s);
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread.iter().find(|m| m.id == msg.id).unwrap().status, MessageStatus::Queued);
    assert!(codes(&thread).contains(&"prompt_failed".to_string()));
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Offline);
}

#[tokio::test(start_paused = true)]
async fn a_reply_during_the_stall_pause_proves_the_prompt_landed() {
    let h = harness();
    h.rt.state().prompt_failures.push_back(RuntimeError::Stalled);
    let bot = h.engine.create_bot(claude_bot("Researcher")).unwrap();
    let msg = h.engine.queue_user_message(&bot.id, "hello").unwrap();
    let pump = tokio::spawn(h.engine.pump(&bot.id));
    while h.rt.state().prompts.is_empty() {
        tokio::task::yield_now().await;
    }
    settle().await;
    assert!(h.engine.handle_bridge(reply("w1:p1", "on it")).await.ok);
    pump.await.unwrap();
    assert_eq!(h.rt.state().prompts.len(), 1, "not sent again");
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread.iter().find(|m| m.id == msg.id).unwrap().status, MessageStatus::Delivered);
    assert!(!codes(&thread).contains(&"prompt_failed".to_string()));
}
