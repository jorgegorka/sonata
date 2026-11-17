mod support;

use sonata_core::model::*;
use sonata_core::{CoreError, Refusal};
use support::*;

fn grok_bot(name: &str) -> NewBot {
    NewBot { provider: Provider::Grok, ..claude_bot(name) }
}

fn has_arg(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

#[tokio::test]
async fn an_edit_mid_turn_reaches_the_cli_with_the_next_message() {
    let h = harness();
    let (bot, first) = bot_with_delivered(&h, "first").await;
    let edit = NewBot { instructions: "Answer in French.".into(), ..claude_bot("Researcher") };
    let edited = h.engine.update_bot(&bot.id, edit).unwrap();
    assert_eq!((edited.handle.as_str(), edited.config_version), ("researcher", 2));
    h.engine.queue_user_message(&bot.id, "second").unwrap();
    h.engine.pump(&bot.id).await;
    assert!(h.rt.state().stopped.is_empty(), "the turn in progress is left alone");

    h.engine.handle_bridge(reply("w1:p1", "done")).await;
    finish_turn(&h, "w1:p1").await;
    h.engine.pump(&bot.id).await;

    {
        let s = h.rt.state();
        assert_eq!(s.stopped, vec!["w1:p1".to_string()], "the old CLI was closed");
        assert_eq!(s.starts.len(), 2);
        assert!(has_arg(&s.starts[1].args, "--resume"), "claude keeps its conversation: {:?}", s.starts[1].args);
        assert_eq!(s.prompts[1].0, "w1:p2");
        assert!(s.prompts[1].1.contains("second"));
    }
    let agents = std::fs::read_to_string(h.paths.bot_dir("researcher").join("AGENTS.md")).unwrap();
    assert!(agents.contains("Answer in French."), "{agents}");
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert_eq!(thread.iter().find(|m| m.id == first.id).unwrap().status, MessageStatus::Delivered);
}

#[tokio::test]
async fn a_colour_change_needs_no_restart() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "first").await;
    h.engine.handle_bridge(reply("w1:p1", "done")).await;
    finish_turn(&h, "w1:p1").await;
    let recolored =
        h.engine.update_bot(&bot.id, NewBot { color: "#16a34a".into(), ..claude_bot("Researcher") }).unwrap();
    assert_eq!(recolored.config_version, 1);
    h.engine.queue_user_message(&bot.id, "second").unwrap();
    h.engine.pump(&bot.id).await;
    let s = h.rt.state();
    assert!(s.stopped.is_empty());
    assert_eq!((s.starts.len(), s.prompts[1].0.as_str()), (1, "w1:p1"));
}

#[tokio::test]
async fn an_edited_grok_bot_starts_a_new_conversation_and_says_so() {
    let h = harness();
    let bot = h.engine.create_bot(grok_bot("Upper")).unwrap();
    h.engine.queue_user_message(&bot.id, "first").unwrap();
    h.engine.pump(&bot.id).await;
    h.engine.handle_bridge(reply("w1:p1", "done")).await;
    finish_turn(&h, "w1:p1").await;
    let first_session = saved_session(&h, &bot.id).await.expect("a session was saved");

    h.engine.restart_session(&latest_session(&h, &bot.id)).await.unwrap();
    h.engine.queue_user_message(&bot.id, "second").unwrap();
    h.engine.pump(&bot.id).await;
    assert!(has_arg(&h.rt.state().starts[1].args, "--resume"));
    h.engine.handle_bridge(reply("w1:p2", "done")).await;
    finish_turn(&h, "w1:p2").await;

    h.engine.update_bot(&bot.id, NewBot { instructions: "Shout.".into(), ..grok_bot("Upper") }).unwrap();
    h.engine.queue_user_message(&bot.id, "third").unwrap();
    h.engine.pump(&bot.id).await;
    let args = h.rt.state().starts[2].args.clone();
    assert!(!has_arg(&args, "--resume") && has_arg(&args, "-s"), "{args:?}");
    assert_ne!(saved_session(&h, &bot.id).await.unwrap(), first_session);
    let codes: Vec<_> = h.engine.user_thread(&bot.id).unwrap().into_iter().filter_map(|m| m.system_code).collect();
    assert_eq!(codes, vec!["config_new_session".to_string()]);
}

#[tokio::test]
async fn switching_provider_never_resumes_the_old_clis_session() {
    let h = harness();
    h.rt.state().session_id = Some("claude-session".into());
    let (bot, _) = bot_with_delivered(&h, "first").await;
    assert_eq!(saved_session(&h, &bot.id).await.as_deref(), Some("claude-session"));

    h.engine.update_bot(&bot.id, grok_bot("Researcher")).unwrap();
    assert_eq!(saved_session(&h, &bot.id).await, None, "the switch drops the old CLI's session");
    h.engine.handle_bridge(reply("w1:p1", "done")).await;
    finish_turn(&h, "w1:p1").await;
    assert_eq!(saved_session(&h, &bot.id).await, None, "the old CLI's turn end does not save it again");

    h.rt.state().session_id = None;
    h.engine.queue_user_message(&bot.id, "second").unwrap();
    h.engine.pump(&bot.id).await;
    let s = h.rt.state();
    assert_eq!(s.stopped, vec!["w1:p1".to_string()]);
    let grok = &s.starts[1];
    assert_eq!(grok.kind, "grok");
    assert!(!has_arg(&grok.args, "--resume") && has_arg(&grok.args, "-s"), "{:?}", grok.args);
}

#[tokio::test]
async fn an_edit_is_checked_like_a_new_bot() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Researcher")).unwrap();
    let err = h.engine.update_bot(&bot.id, NewBot { display_name: "  ".into(), ..claude_bot("x") }).unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::NameRequired)), "{err}");
    let err = h.engine.update_bot("missing", claude_bot("X")).unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::BotNotFound(_))), "{err}");
    let trimmed = h.engine.update_bot(&bot.id, claude_bot("  Scout  ")).unwrap();
    assert_eq!((trimmed.display_name.as_str(), trimmed.handle.as_str()), ("Scout", "researcher"));
}

#[tokio::test(start_paused = true)]
async fn switching_provider_while_the_bot_starts_never_resumes_the_old_session() {
    let h = harness();
    h.rt.state().start_delay = Some(std::time::Duration::from_secs(5));
    let bot = h.engine.create_bot(grok_bot("Researcher")).unwrap();
    h.engine.queue_user_message(&bot.id, "first").unwrap();
    let pump = tokio::spawn(h.engine.pump(&bot.id));
    while h.rt.state().starts.len() != 1 {
        tokio::task::yield_now().await;
    }
    h.engine.update_bot(&bot.id, claude_bot("Researcher")).unwrap();
    pump.await.unwrap();
    assert_eq!(saved_session(&h, &bot.id).await, None, "the grok session is not written back");

    h.rt.state().start_delay = None;
    h.engine.handle_bridge(reply("w1:p1", "done")).await;
    finish_turn(&h, "w1:p1").await;
    h.engine.queue_user_message(&bot.id, "second").unwrap();
    h.engine.pump(&bot.id).await;
    let args = h.rt.state().starts[1].args.clone();
    assert_eq!(h.rt.state().starts[1].kind, "claude");
    assert!(!has_arg(&args, "--resume"), "{args:?}");
    let codes: Vec<_> = h.engine.user_thread(&bot.id).unwrap().into_iter().filter_map(|m| m.system_code).collect();
    assert!(codes.is_empty(), "{codes:?}");
}

async fn two_sessions_after_a_turn(h: &Harness, new: NewBot) -> (Bot, [Session; 2], [String; 2]) {
    let bot = h.engine.create_bot(new).unwrap();
    let (one, _) = h.engine.start_session_with_message(&bot.id, "A", &[]).unwrap();
    let (two, _) = h.engine.start_session_with_message(&bot.id, "B", &[]).unwrap();
    h.engine.pump(&bot.id).await;
    let panes = [pane(h, &one), pane(h, &two)];
    for p in &panes {
        assert!(h.engine.handle_bridge(reply(p, "done")).await.ok);
        finish_turn(h, p).await;
    }
    let saved = |s: &Session| h.engine.sessions_of(&bot.id).unwrap().into_iter().find(|x| x.id == s.id).unwrap();
    let sessions = [saved(&one), saved(&two)];
    (bot, sessions, panes)
}

fn last_start_args(h: &Harness, session: &Session) -> Vec<String> {
    h.rt.state().starts.iter().rfind(|spec| spec.agent == agent(session)).expect("started").args.clone()
}

fn notices(h: &Harness, in_chat: &Message) -> Vec<String> {
    h.engine.thread_messages(&in_chat.thread_id).unwrap().into_iter().filter_map(|m| m.system_code).collect()
}

#[tokio::test]
async fn each_session_of_an_edited_claude_bot_restarts_with_its_next_message_and_resumes_its_own_conversation() {
    let h = harness();
    let (bot, [one, two], [p1, p2]) = two_sessions_after_a_turn(&h, claude_bot("Researcher")).await;
    let saved = [&one, &two].map(|s| s.provider_session_id.clone().expect("a CLI session was saved"));
    assert_ne!(saved[0], saved[1]);
    h.engine
        .update_bot(&bot.id, NewBot { instructions: "Answer in French.".into(), ..claude_bot("Researcher") })
        .unwrap();

    h.engine.queue_user_message_to_session(&one.id, "A again", &[]).unwrap();
    h.engine.pump_session(&one.id).await;
    assert_eq!(h.rt.state().stopped, vec![p1.clone()], "only session 1's CLI was replaced");
    assert!(last_start_args(&h, &one).ends_with(&["--resume".to_string(), saved[0].clone()]));

    h.engine.queue_user_message_to_session(&two.id, "B again", &[]).unwrap();
    h.engine.pump_session(&two.id).await;
    assert_eq!(h.rt.state().stopped, vec![p1, p2]);
    assert!(last_start_args(&h, &two).ends_with(&["--resume".to_string(), saved[1].clone()]));
    assert_eq!(h.rt.state().starts.len(), 4);
}

#[tokio::test]
async fn each_session_of_an_edited_grok_bot_starts_afresh_and_says_so_in_its_own_chat() {
    let h = harness();
    let (bot, [one, two], [p1, p2]) = two_sessions_after_a_turn(&h, grok_bot("Upper")).await;
    h.engine.update_bot(&bot.id, NewBot { instructions: "Shout.".into(), ..grok_bot("Upper") }).unwrap();

    let a = h.engine.queue_user_message_to_session(&one.id, "A again", &[]).unwrap();
    h.engine.pump_session(&one.id).await;
    let args = last_start_args(&h, &one);
    assert!(!has_arg(&args, "--resume") && has_arg(&args, "-s"), "{args:?}");
    assert_eq!(h.rt.state().stopped, vec![p1.clone()]);
    let b = h.engine.queue_user_message_to_session(&two.id, "B again", &[]).unwrap();
    assert_eq!(notices(&h, &a), vec!["config_new_session".to_string()]);
    assert!(notices(&h, &b).is_empty(), "session 2 has not started again yet");

    h.engine.pump_session(&two.id).await;
    let args = last_start_args(&h, &two);
    assert!(!has_arg(&args, "--resume") && has_arg(&args, "-s"), "{args:?}");
    assert_eq!(h.rt.state().stopped, vec![p1, p2]);
    assert_eq!(notices(&h, &b), vec!["config_new_session".to_string()]);
    assert_eq!(notices(&h, &a), vec!["config_new_session".to_string()], "one notice in each chat");
    let now = h.engine.sessions_of(&bot.id).unwrap();
    for old in [&one, &two] {
        let now = now.iter().find(|s| s.id == old.id).unwrap();
        assert_ne!(old.provider_session_id, now.provider_session_id, "a new CLI session");
    }
}

#[tokio::test]
async fn a_provider_switch_clears_every_sessions_cli_conversation_and_each_starts_afresh() {
    let h = harness();
    let (bot, [one, two], [p1, p2]) = two_sessions_after_a_turn(&h, claude_bot("Researcher")).await;
    assert!(one.provider_session_id.is_some() && two.provider_session_id.is_some());
    h.engine.update_bot(&bot.id, grok_bot("Researcher")).unwrap();
    assert!(h.engine.sessions_of(&bot.id).unwrap().iter().all(|s| s.provider_session_id.is_none()));

    let a = h.engine.queue_user_message_to_session(&one.id, "A again", &[]).unwrap();
    let b = h.engine.queue_user_message_to_session(&two.id, "B again", &[]).unwrap();
    h.engine.pump(&bot.id).await;

    let mut stopped = h.rt.state().stopped.clone();
    stopped.sort();
    assert_eq!(stopped, vec![p1, p2]);
    for session in [&one, &two] {
        let s = h.rt.state();
        let spec = s.starts.iter().rfind(|spec| spec.agent == agent(session)).unwrap();
        assert_eq!(spec.kind, "grok");
        assert!(!has_arg(&spec.args, "--resume") && has_arg(&spec.args, "-s"), "{:?}", spec.args);
    }
    assert!(notices(&h, &a).is_empty() && notices(&h, &b).is_empty(), "nothing saved, so nothing to say");
}
