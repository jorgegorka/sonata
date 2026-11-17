mod support;

use sonata_core::model::*;
use sonata_core::runtime::RuntimeEvent;
use sonata_core::{CoreError, Refusal};
use support::*;

fn opencode_bot(name: &str, model: Option<&str>) -> NewBot {
    NewBot { provider: Provider::Opencode, model: model.map(String::from), ..claude_bot(name) }
}

fn is_app_chosen_opencode_id(id: &str) -> bool {
    id.strip_prefix("ses_")
        .is_some_and(|hex| hex.len() == 32 && hex.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)))
}

async fn opencode_in_flight(h: &Harness, model: Option<&str>) -> Bot {
    let bot = h.engine.create_bot(opencode_bot("Scout", model)).unwrap();
    h.engine.queue_user_message(&bot.id, "hi").unwrap();
    h.engine.pump(&bot.id).await;
    bot
}

#[tokio::test]
async fn an_opencode_session_starts_standalone_with_its_own_id_and_config() {
    let h = harness();
    let bot = opencode_in_flight(&h, Some("opencode-go/kimi-k3")).await;
    let spec = h.rt.state().starts[0].clone();
    assert_eq!(spec.kind, "opencode");
    assert_eq!(spec.args[..3], ["--standalone", "--auto", "-s"]);
    assert_eq!(spec.args.len(), 4);
    assert!(is_app_chosen_opencode_id(&spec.args[3]), "{}", spec.args[3]);
    assert_eq!(saved_session(&h, &bot.id).await.as_deref(), Some(spec.args[3].as_str()));
    assert_eq!(spec.env["SBOT_SOCKET"], h.paths.bridge_socket().to_string_lossy());
    assert_eq!(spec.env["OPENCODE_DISABLE_AUTOUPDATE"], "1");
    let config: serde_json::Value = serde_json::from_str(&spec.env["OPENCODE_CONFIG_CONTENT"]).unwrap();
    assert_eq!(config["model"], "opencode-go/kimi-k3");
    assert_eq!((spec.trust, spec.sign_in), (None, None));
}

#[tokio::test]
async fn an_opencode_session_resumes_with_its_saved_id() {
    let h = harness();
    let bot = opencode_in_flight(&h, None).await;
    let saved = saved_session(&h, &bot.id).await.expect("saved at start");
    h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: "w1:p1".into() }).await;
    settle().await;
    h.engine.queue_user_message(&bot.id, "again").unwrap();
    h.engine.pump(&bot.id).await;
    assert_eq!(h.rt.state().starts[1].args, ["--standalone", "--auto", "-s", saved.as_str()]);
}

#[tokio::test]
async fn stopping_an_opencode_turn_presses_esc_twice_at_once() {
    let h = harness();
    let bot = opencode_in_flight(&h, None).await;
    h.engine.stop_turn(&latest_session(&h, &bot.id)).await.unwrap();
    assert_eq!(h.rt.state().keys, vec![("w1:p1".to_string(), vec!["esc".to_string(), "esc".to_string()])]);
}

#[tokio::test]
async fn a_stop_follows_the_cli_that_runs_not_the_bots_new_provider() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "first").await;
    h.engine.update_bot(&bot.id, opencode_bot(&bot.display_name, None)).unwrap();
    h.engine.stop_turn(&latest_session(&h, &bot.id)).await.unwrap();
    assert_eq!(h.rt.state().keys, vec![("w1:p1".to_string(), vec!["esc".to_string()])]);
}

#[tokio::test]
async fn a_claude_bot_switched_to_opencode_starts_fresh_with_an_opencode_id() {
    let h = harness();
    let bot = h.engine.create_bot(NewBot { model: Some("sonnet".into()), ..claude_bot("Writer") }).unwrap();
    h.engine.queue_user_message(&bot.id, "first").unwrap();
    h.engine.pump(&bot.id).await;
    let claude_id = saved_session(&h, &bot.id).await.expect("claude's id saved");
    h.engine.handle_bridge(reply("w1:p1", "done")).await;
    finish_turn(&h, "w1:p1").await;

    let err = h.engine.update_bot(&bot.id, opencode_bot("Writer", Some("sonnet"))).unwrap_err();
    assert!(matches!(&err, CoreError::Refused(Refusal::ModelFormat(m)) if m == "sonnet"), "{err}");
    h.engine.update_bot(&bot.id, opencode_bot("Writer", None)).unwrap();
    h.engine.queue_user_message(&bot.id, "second").unwrap();
    h.engine.pump(&bot.id).await;
    let s = h.rt.state();
    let args = &s.starts.last().unwrap().args;
    assert_eq!(args[..3], ["--standalone", "--auto", "-s"]);
    assert!(is_app_chosen_opencode_id(&args[3]), "{args:?}");
    assert!(!args.contains(&claude_id), "{args:?}");
}

#[tokio::test]
async fn an_opencode_model_must_name_its_provider() {
    let h = harness();
    for model in ["kimi", "opencode-go/", "/kimi", " kimi "] {
        let err = h.engine.create_bot(opencode_bot("Scout", Some(model))).unwrap_err();
        assert!(matches!(&err, CoreError::Refused(Refusal::ModelFormat(_))), "{model:?}: {err}");
    }
    for (name, model) in [("One", Some("opencode-go/kimi-k3")), ("Two", Some("  ")), ("Three", None)] {
        h.engine.create_bot(opencode_bot(name, model)).unwrap_or_else(|e| panic!("{model:?}: {e}"));
    }
    h.engine.create_bot(NewBot { model: Some("sonnet".into()), ..claude_bot("Claude") }).unwrap();
}

#[tokio::test]
async fn a_new_model_starts_the_opencode_sessions_afresh() {
    let h = harness();
    let bot = opencode_in_flight(&h, Some("opencode-go/glm-5.3-flash")).await;
    let first = saved_session(&h, &bot.id).await.expect("saved at start");
    h.engine.handle_bridge(reply("w1:p1", "done")).await;
    finish_turn(&h, "w1:p1").await;
    h.engine.update_bot(&bot.id, opencode_bot("Scout", Some("opencode-go/kimi-k3"))).unwrap();
    assert_eq!(saved_session(&h, &bot.id).await, None);
    h.engine.queue_user_message(&bot.id, "again").unwrap();
    h.engine.pump(&bot.id).await;
    let s = h.rt.state();
    let args = &s.starts.last().unwrap().args;
    assert!(is_app_chosen_opencode_id(&args[3]) && args[3] != first, "{args:?}");
}

#[tokio::test]
async fn other_edits_keep_the_opencode_conversation() {
    let h = harness();
    let bot = opencode_in_flight(&h, Some("opencode-go/glm-5.3-flash")).await;
    let first = saved_session(&h, &bot.id).await.expect("saved at start");
    let edit =
        NewBot { instructions: "Be brief.".into(), ..opencode_bot("Scout", Some(" opencode-go/glm-5.3-flash ")) };
    h.engine.update_bot(&bot.id, edit).unwrap();
    assert_eq!(saved_session(&h, &bot.id).await.as_deref(), Some(first.as_str()));
}

#[tokio::test]
async fn a_new_model_keeps_a_claude_conversation() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "first").await;
    let first = saved_session(&h, &bot.id).await.expect("saved at start");
    h.engine.update_bot(&bot.id, NewBot { model: Some("opus".into()), ..claude_bot(&bot.display_name) }).unwrap();
    assert_eq!(saved_session(&h, &bot.id).await.as_deref(), Some(first.as_str()), "claude switches models on resume");
}
