mod support;
use bridge_proto::ErrorCode;
use sonata_core::model::*;
use sonata_core::runtime::{RuntimeError, TrustPrompt};
use sonata_core::store::Store;
use support::*;

fn codex_bot(name: &str) -> NewBot {
    NewBot { provider: Provider::Codex, ..claude_bot(name) }
}

#[tokio::test]
async fn a_codex_session_starts_with_codex_arguments_and_no_app_chosen_id() {
    let h = harness();
    let bot = h.engine.create_bot(codex_bot("Coder")).unwrap();
    h.engine.queue_user_message(&bot.id, "hi").unwrap();
    h.engine.pump(&bot.id).await;
    let spec = h.rt.state().starts[0].clone();
    assert_eq!(spec.kind, "codex");
    assert_eq!(
        spec.args,
        ["--dangerously-bypass-approvals-and-sandbox", "--no-daemon", "-c", "check_for_update_on_startup=false"]
    );
    assert_eq!(spec.sign_in, Some("1. Sign in with ChatGPT"));
    assert_eq!(spec.trust, Some(TrustPrompt::Menu { option: "Trust and continue" }));
    assert_eq!(saved_session(&h, &bot.id).await, None, "nothing to save until codex reports its thread");
}

async fn codex_in_flight(h: &Harness) -> (Bot, String) {
    let bot = h.engine.create_bot(codex_bot("Coder")).unwrap();
    h.engine.queue_user_message(&bot.id, "hi").unwrap();
    h.engine.pump(&bot.id).await;
    let session = h.engine.sessions_of(&bot.id).unwrap().remove(0);
    let p = pane(h, &session);
    (bot, p)
}

#[tokio::test]
async fn a_codex_session_takes_its_id_from_sbot() {
    let h = harness();
    let (bot, p) = codex_in_flight(&h).await;
    let resp = h.engine.handle_bridge(with_cli_id(reply(&p, "done"), "thread-1")).await;
    assert!(resp.ok, "{resp:?}");
    assert_eq!(saved_session(&h, &bot.id).await.as_deref(), Some("thread-1"));
}

#[tokio::test]
async fn a_sbot_id_never_replaces_a_saved_one() {
    let h = harness();
    let (bot, p) = codex_in_flight(&h).await;
    h.engine.handle_bridge(with_cli_id(reply(&p, "done"), "thread-1")).await;
    h.engine.handle_bridge(with_cli_id(bots_request(&p), "thread-sub")).await;
    assert_eq!(saved_session(&h, &bot.id).await.as_deref(), Some("thread-1"));
}

#[tokio::test]
async fn a_sbot_id_fills_the_id_whatever_the_command() {
    let h = harness();
    let (bot, p) = codex_in_flight(&h).await;
    let resp = h.engine.handle_bridge(with_cli_id(bots_request(&p), "thread-1")).await;
    assert!(resp.ok, "{resp:?}");
    assert_eq!(saved_session(&h, &bot.id).await.as_deref(), Some("thread-1"));

    let h = harness();
    let (bot, p) = codex_in_flight(&h).await;
    finish_turn(&h, &p).await;
    let resp = h.engine.handle_bridge(with_cli_id(reply(&p, "late"), "thread-2")).await;
    assert_eq!(resp.error.map(|e| e.code), Some(ErrorCode::NothingToReplyTo));
    assert_eq!(saved_session(&h, &bot.id).await.as_deref(), Some("thread-2"));
}

#[tokio::test]
async fn a_sbot_id_from_an_unknown_pane_changes_nothing() {
    let h = harness();
    let (bot, _) = codex_in_flight(&h).await;
    let resp = h.engine.handle_bridge(with_cli_id(reply("w9:p9", "x"), "thread-x")).await;
    assert_eq!(resp.error.map(|e| e.code), Some(ErrorCode::NotABot));
    assert_eq!(saved_session(&h, &bot.id).await, None);
}

#[tokio::test]
async fn a_sbot_id_is_ignored_for_a_cli_that_takes_the_apps_id() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "hi").await;
    let session = h.engine.sessions_of(&bot.id).unwrap().remove(0);
    let before = saved_session(&h, &bot.id).await;
    assert!(before.is_some(), "claude's id is chosen by the app at start");
    h.engine.handle_bridge(with_cli_id(bots_request(&pane(&h, &session)), "thread-x")).await;
    assert_eq!(saved_session(&h, &bot.id).await, before);
}

#[tokio::test]
async fn a_sbot_id_after_a_switch_to_claude_is_ignored() {
    let h = harness();
    let (bot, p) = codex_in_flight(&h).await;
    h.engine.update_bot(&bot.id, NewBot { provider: Provider::Claude, ..codex_bot("Coder") }).unwrap();
    h.engine.handle_bridge(with_cli_id(bots_request(&p), "thread-1")).await;
    assert_eq!(saved_session(&h, &bot.id).await, None);
}

#[tokio::test]
async fn a_failed_resume_starts_fresh_and_the_next_sbot_id_fills_it() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(codex_bot("Coder")).unwrap();
    let session = new_session(&h, &bot.id);
    Store::open(&h.paths.db_path())
        .unwrap()
        .set_session_cli_id(&session.id, Some("gone"), bot.config_version, bot.provider)
        .unwrap();

    let next = relaunch(&h);
    next.rt.state().start_failures.push_back(RuntimeError::StartFailed("No saved session found".into()));
    next.engine.queue_user_message_to_session(&session.id, "again", &[]).unwrap();
    next.engine.pump(&bot.id).await;
    {
        let s = next.rt.state();
        assert_eq!(s.starts.len(), 2);
        assert_eq!(s.starts[0].args.first().map(String::as_str), Some("resume"));
        assert_ne!(s.starts[1].args.first().map(String::as_str), Some("resume"), "{:?}", s.starts[1].args);
    }
    let thread = next.engine.user_thread(&bot.id).unwrap();
    assert!(thread.iter().any(|m| m.system_code.as_deref() == Some("session_reset")), "{thread:?}");
    assert_eq!(saved_session(&next, &bot.id).await, None);

    let p = pane(&next, &session);
    next.engine.handle_bridge(with_cli_id(bots_request(&p), "thread-2")).await;
    assert_eq!(saved_session(&next, &bot.id).await.as_deref(), Some("thread-2"));
}
