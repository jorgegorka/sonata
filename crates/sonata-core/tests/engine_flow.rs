#![allow(clippy::await_holding_lock)]
mod support;

use std::collections::BTreeMap;

use bridge_proto::*;
use sonata_core::model::*;
use sonata_core::runtime::{AgentState, RuntimeEvent};
use support::*;

fn state(pane: &str, s: AgentState) -> RuntimeEvent {
    RuntimeEvent::State { pane_id: pane.into(), state: s }
}

#[tokio::test]
async fn create_bot_derives_handle_and_folder_and_rejects_bad_input() {
    let h = harness();
    let a = h.engine.create_bot(claude_bot("Investigación")).unwrap();
    let b = h.engine.create_bot(claude_bot("Investigación")).unwrap();
    assert_eq!((a.handle.as_str(), b.handle.as_str()), ("investigacion", "investigacion-2"));
    assert!(h.paths.bot_dir("investigacion").is_dir());
    assert!(h.engine.create_bot(claude_bot("   ")).is_err());
    let cursor = NewBot { provider: Provider::Cursor, ..claude_bot("Writer") };
    assert!(h.engine.create_bot(cursor).is_err());
}

#[tokio::test]
async fn a_new_bot_never_inherits_a_leftover_folder() {
    let h = harness();
    std::fs::create_dir_all(h.paths.bot_dir("Researcher")).unwrap();
    std::fs::write(h.paths.bot_dir("Researcher").join("MEMORY.md"), "someone else's notes").unwrap();
    let bot = h.engine.create_bot(claude_bot("Researcher")).unwrap();
    assert_eq!(bot.handle, "researcher-2");
    assert!(h.paths.bot_dir("researcher-2").is_dir());
    assert!(!h.paths.bot_dir("researcher-2").join("MEMORY.md").exists());
}

#[tokio::test]
async fn first_message_starts_bot_and_delivers_envelope() {
    let h = harness();
    let (bot, msg) = bot_with_delivered(&h, "hello").await;

    let s = h.rt.state();
    assert_eq!(s.starts.len(), 1);
    let spec = &s.starts[0];
    assert_eq!(spec.kind, "claude");
    assert_eq!(spec.trust, Some(sonata_core::runtime::TrustPrompt::Menu { option: "Yes, I trust this folder" }));
    assert_eq!(
        spec.args[..6],
        [
            "--dangerously-skip-permissions",
            "--settings",
            r#"{"skipDangerousModePermissionPrompt":true}"#,
            "--disallowedTools",
            "AskUserQuestion",
            "--session-id"
        ]
    );
    assert_eq!(spec.args.len(), 7);
    let chosen = uuid::Uuid::parse_str(&spec.args[6]).expect("a UUID");
    assert_eq!(chosen.get_version_num(), 4);
    assert_eq!(spec.cwd, h.paths.bot_dir("researcher"));
    assert_eq!(
        spec.env,
        BTreeMap::from([("SBOT_SOCKET".to_string(), h.paths.bridge_socket().to_string_lossy().into_owned())])
    );
    assert!(spec.cwd.join("AGENTS.md").is_file());
    assert_eq!(
        s.prompts,
        vec![("w1:p1".to_string(), "[Message from user · thread you↔user · language: English]\nhello".to_string())]
    );
    drop(s);

    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread[0].id, msg.id);
    assert_eq!(thread[0].status, MessageStatus::Delivered);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
    assert_eq!(saved_session(&h, &bot.id).await, Some(chosen.to_string()));
}

#[tokio::test]
async fn the_engine_chosen_session_is_resumed_after_an_exit() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    let chosen = saved_session(&h, &bot.id).await.expect("id stored once the start succeeded");
    h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: "w1:p1".into() }).await;
    settle().await;
    h.engine.queue_user_message(&bot.id, "again").unwrap();
    h.engine.pump(&bot.id).await;
    let s = h.rt.state();
    assert!(s.starts[1].args.ends_with(&["--resume".to_string(), chosen]));
    assert!(!s.starts[1].args.contains(&"--session-id".to_string()));
}

#[tokio::test]
async fn reply_then_idle_ends_turn_without_fallback() {
    let h = harness();
    let (bot, msg) = bot_with_delivered(&h, "hello").await;
    h.engine.on_runtime_event(state("w1:p1", AgentState::Working)).await;

    let resp = h.engine.handle_bridge(reply("w1:p1", "Hi there!\n")).await;
    assert!(resp.ok, "{resp:?}");
    h.engine.on_runtime_event(state("w1:p1", AgentState::Idle)).await;

    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread.len(), 2);
    assert_eq!(thread[1].kind, MessageKind::Text);
    assert_eq!(thread[1].body, "Hi there!");
    assert_eq!(thread[1].sender, bot.id);
    assert_eq!(thread[1].in_reply_to.as_deref(), Some(msg.id.as_str()));
    assert_eq!(Some(thread[1].id.clone()), resp.message_id);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Idle);
}

#[tokio::test]
async fn turn_without_reply_posts_raw_output_and_reminds_next_time() {
    let h = harness();
    h.rt.state().output = "Here is what I found\n".into();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    h.engine.on_runtime_event(state("w1:p1", AgentState::Working)).await;
    h.engine.on_runtime_event(state("w1:p1", AgentState::Idle)).await;

    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread[1].kind, MessageKind::RawFallback);
    assert_eq!(thread[1].body, "Here is what I found");

    h.engine.queue_user_message(&bot.id, "again").unwrap();
    h.engine.pump(&bot.id).await;
    let prompts = h.rt.state().prompts.clone();
    assert!(prompts[1].1.ends_with("Reminder: the user only sees what you send with sbot reply."));
}

#[tokio::test]
async fn blank_output_posts_no_output_notice() {
    let h = harness();
    h.rt.state().output = "   \n".into();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    h.engine.on_runtime_event(state("w1:p1", AgentState::Working)).await;
    h.engine.on_runtime_event(state("w1:p1", AgentState::Idle)).await;
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread[1].kind, MessageKind::System);
    assert_eq!(thread[1].system_code.as_deref(), Some("no_output_captured"));
}

#[tokio::test]
async fn messages_sent_while_working_are_delivered_one_at_a_time_in_order() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "one").await;
    h.engine.queue_user_message(&bot.id, "two").unwrap();
    h.engine.pump(&bot.id).await;
    h.engine.queue_user_message(&bot.id, "three").unwrap();
    h.engine.pump(&bot.id).await;
    assert_eq!(h.rt.state().prompts.len(), 1);

    for _ in 0..2 {
        h.engine.on_runtime_event(state("w1:p1", AgentState::Working)).await;
        h.engine.on_runtime_event(state("w1:p1", AgentState::Idle)).await;
        settle().await;
    }
    let s = h.rt.state();
    assert_eq!(s.starts.len(), 1);
    let bodies: Vec<&str> = s.prompts.iter().map(|(_, t)| t.lines().nth(1).unwrap()).collect();
    assert_eq!(bodies, vec!["one", "two", "three"]);
}

#[tokio::test]
async fn concurrent_pumps_start_the_bot_once() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Researcher")).unwrap();
    h.engine.queue_user_message(&bot.id, "a").unwrap();
    h.engine.queue_user_message(&bot.id, "b").unwrap();
    tokio::join!(h.engine.pump(&bot.id), h.engine.pump(&bot.id));
    let s = h.rt.state();
    assert_eq!(s.starts.len(), 1);
    assert_eq!(s.prompts.len(), 1);
}

#[tokio::test]
async fn idle_before_working_does_not_end_the_turn() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    h.engine.on_runtime_event(state("w1:p1", AgentState::Idle)).await;
    assert_eq!(h.engine.user_thread(&bot.id).unwrap().len(), 1);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
}

#[tokio::test]
async fn bridge_rejects_bad_requests_with_specific_codes() {
    let h = harness();
    let code = |r: BridgeResponse| r.error.map(|e| e.code);

    assert_eq!(code(h.engine.handle_bridge(reply("w9:p9", "x")).await), Some(ErrorCode::NotABot));

    let (bot, _) = bot_with_delivered(&h, "hello").await;
    assert_eq!(code(h.engine.handle_bridge(reply("w1:p1", "  ")).await), Some(ErrorCode::EmptyBody));
    let v2 = BridgeRequest { v: 2, ..reply("w1:p1", "x") };
    assert_eq!(code(h.engine.handle_bridge(v2).await), Some(ErrorCode::UnsupportedVersion));
    let send = BridgeRequest {
        v: 1,
        pane_id: "w1:p1".into(),
        cli_session_id: None,
        command: BridgeCommand::Send { to: "@writer".into(), body: "x".into(), files: vec![] },
    };
    assert_eq!(code(h.engine.handle_bridge(send).await), Some(ErrorCode::UnknownRecipient));

    h.engine.on_runtime_event(state("w1:p1", AgentState::Working)).await;
    h.engine.on_runtime_event(state("w1:p1", AgentState::Idle)).await;
    assert_eq!(code(h.engine.handle_bridge(reply("w1:p1", "late")).await), Some(ErrorCode::NothingToReplyTo));
    assert!(h.engine.user_thread(&bot.id).unwrap().iter().all(|m| m.body != "late"));
}

#[tokio::test]
async fn ui_events_follow_the_message_lifecycle() {
    use sonata_core::engine::UiEvent;
    let h = harness();
    let mut rx = h.engine.subscribe_ui();
    let (bot, msg) = bot_with_delivered(&h, "hello").await;

    let mut events = vec![];
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    assert!(
        matches!(&events[0], UiEvent::SessionAdded { session } if Some(&session.session.id) == msg.recipient_session.as_ref())
    );
    assert!(matches!(&events[1], UiEvent::MessageAdded { message } if message.id == msg.id));
    let statuses: Vec<BotStatus> = events
        .iter()
        .filter_map(|e| match e {
            UiEvent::BotStatus { bot_id, status } if *bot_id == bot.id => Some(*status),
            _ => None,
        })
        .collect();
    assert_eq!(statuses, vec![BotStatus::Starting, BotStatus::Idle, BotStatus::Working]);
    assert!(events.iter().any(
        |e| matches!(e, UiEvent::MessageUpdated { message } if message.id == msg.id && message.status == MessageStatus::Delivered)
    ));
}

#[tokio::test]
async fn a_grok_bot_starts_with_its_flags_an_app_chosen_session_and_its_trust_answer() {
    let h = harness();
    let bot = h
        .engine
        .create_bot(NewBot { provider: Provider::Grok, model: Some("grok-4".into()), ..claude_bot("Scout") })
        .unwrap();
    h.engine.queue_user_message(&bot.id, "hello").unwrap();
    h.engine.pump(&bot.id).await;
    let s = h.rt.state();
    let spec = &s.starts[0];
    assert_eq!(spec.kind, "grok");
    assert_eq!(spec.args[..4], ["--always-approve", "-m", "grok-4", "-s"]);
    assert!(uuid::Uuid::parse_str(&spec.args[4]).is_ok(), "{:?}", spec.args);
    assert_eq!(
        spec.trust,
        Some(sonata_core::runtime::TrustPrompt::Key {
            question: "Do you trust the contents of this directory?",
            key: "y"
        })
    );
    assert!(spec.cwd.join("AGENTS.md").is_file());
    assert!(!spec.cwd.join("CLAUDE.md").exists(), "grok reads AGENTS.md itself");
}
