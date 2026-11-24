use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sonata_core::engine::{Engine, EngineConfig};
use sonata_core::model::*;
use sonata_core::paths::AppPaths;
use sonata_core::provider::{ClaudeAdapter, CodexAdapter, GrokAdapter, ProviderAdapter};
use sonata_core::runtime::herdr::HerdrRuntime;
use sonata_core::runtime::{BotRuntime, Inspection, LaunchSpec, RuntimeBot, RuntimeError, RuntimeEvent, RuntimeHealth};
use sonata_core::store::Store;

struct Cleanup {
    bin: PathBuf,
    session: String,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::process::Command::new(&self.bin).args(["--session", &self.session, "server", "stop"]).output();
        let _ = std::process::Command::new(&self.bin).args(["session", "delete", &self.session]).output();
        let _ = std::fs::remove_file(herdr_client::server_config_path(&self.session));
    }
}

struct OpencodeSessions(PathBuf);

impl Drop for OpencodeSessions {
    fn drop(&mut self) {
        let Ok(store) = Store::open(&self.0) else { return };
        let ids: Vec<String> = store
            .list_bots()
            .unwrap_or_default()
            .iter()
            .flat_map(|bot| store.sessions_of(&bot.id).unwrap_or_default())
            .filter_map(|session| session.provider_session_id)
            .filter(|id| id.starts_with("ses_"))
            .collect();
        for id in ids {
            let deleted =
                std::process::Command::new("opencode").args(["session", "delete", &id, "--standalone"]).output();
            println!("deleted opencode session {id}: {:?}", deleted.map(|o| o.status));
        }
    }
}

fn live_model(provider: Provider) -> Option<String> {
    (provider == Provider::Opencode).then(|| std::env::var("SONATA_LIVE_OPENCODE_MODEL").ok()).flatten()
}

fn saved_cli_id(db: &Path, bot_id: &str) -> Option<String> {
    Store::open(db).unwrap().latest_session(bot_id).unwrap().and_then(|s| s.provider_session_id)
}

fn save_cli_id(db: &Path, bot: &Bot, cli_id: &str) {
    let store = Store::open(db).unwrap();
    let session = match store.latest_session(&bot.id).unwrap() {
        Some(session) => session,
        None => store.create_session(&bot.id, None, None).unwrap(),
    };
    store.set_session_cli_id(&session.id, Some(cli_id), bot.config_version, bot.provider).unwrap();
}

fn sbot_dir() -> PathBuf {
    std::env::current_exe()
        .unwrap()
        .ancestors()
        .skip(1)
        .find(|directory| directory.join("sbot").is_file())
        .expect("build sbot first: cargo build -p sbot")
        .to_path_buf()
}

#[tokio::test]
#[ignore = "needs herdr and a logged-in claude; run with --ignored"]
async fn claude_bot_answers_through_sbot() {
    let bin = herdr_client::find_herdr_bin().expect("herdr installed");
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    let session = format!("sonata-test-{}-{nanos}", std::process::id());
    let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };

    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
    paths.ensure().unwrap();
    let runtime = HerdrRuntime::new(bin, session);
    let engine = Engine::new(
        Store::open_in_memory().unwrap(),
        runtime,
        EngineConfig {
            paths: paths.clone(),
            sbot_dir: sbot_dir(),
            fallback_lines: 200,
            turn_start_grace: Some(Duration::from_secs(10)),
        },
    );
    engine.run_event_loop();
    sonata_core::bridge::serve(&paths.bridge_socket(), engine.clone()).unwrap();

    let bot = engine
        .create_bot(NewBot {
            display_name: "Live Test".into(),
            color: "#3b82f6".into(),
            provider: Provider::Claude,
            model: None,
            instructions: "Answer in one short sentence.".into(),
            skill_ids: vec![],
        })
        .unwrap();
    engine.queue_user_message(&bot.id, "What is 2 + 2?").unwrap();
    engine.pump(&bot.id).await;

    let mut answer = None;
    for _ in 0..180 {
        let thread = engine.user_thread(&bot.id).unwrap();
        if let Some(m) = thread.iter().find(|m| m.sender == bot.id && m.kind != MessageKind::System) {
            answer = Some(m.clone());
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let answer = answer.expect("bot answered within 180 s");
    println!("{:?}: {}", answer.kind, answer.body);
    assert_eq!(answer.kind, MessageKind::Text, "bot fell back to raw output instead of sbot reply");
    assert!(answer.body.contains('4'));
    engine.shutdown().await;
}

async fn wait_for_bot_text(engine: &Engine, bot_id: &str, after_ids: &std::collections::HashSet<String>) -> Message {
    for _ in 0..180 {
        let thread = engine.user_thread(bot_id).unwrap();
        if let Some(m) = thread
            .iter()
            .rev()
            .find(|m| m.sender == bot_id && m.kind != MessageKind::System && !after_ids.contains(&m.id))
        {
            return m.clone();
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let thread: Vec<String> = engine
        .user_thread(bot_id)
        .unwrap()
        .iter()
        .map(|m| format!("{:?} {:?} {:?} {:?} {}", m.kind, m.status, m.system_code, m.system_params, m.body))
        .collect();
    panic!("bot did not answer within 180 s; thread:\n{}", thread.join("\n"));
}

struct WithoutUserSettings(Arc<HerdrRuntime>);

#[async_trait::async_trait]
impl BotRuntime for WithoutUserSettings {
    async fn start_bot(&self, spec: &LaunchSpec) -> Result<RuntimeBot, RuntimeError> {
        let mut spec = spec.clone();
        spec.args.extend(["--setting-sources".to_string(), "project,local".to_string()]);
        self.0.start_bot(&spec).await
    }
    async fn prompt(&self, bot: &RuntimeBot, text: &str) -> Result<(), RuntimeError> {
        self.0.prompt(bot, text).await
    }
    async fn inspect(&self, bot: &RuntimeBot) -> Result<Inspection, RuntimeError> {
        self.0.inspect(bot).await
    }
    async fn read_output(&self, bot: &RuntimeBot, lines: u32) -> Result<String, RuntimeError> {
        self.0.read_output(bot, lines).await
    }
    async fn snapshot(&self, bot: &RuntimeBot, lines: u32) -> Result<String, RuntimeError> {
        self.0.snapshot(bot, lines).await
    }
    async fn terminal(&self, bot: &RuntimeBot, lines: u32) -> Result<String, RuntimeError> {
        self.0.terminal(bot, lines).await
    }
    async fn send_keys(&self, bot: &RuntimeBot, keys: &[&str]) -> Result<(), RuntimeError> {
        self.0.send_keys(bot, keys).await
    }
    async fn stop_bot(&self, bot: &RuntimeBot) -> Result<(), RuntimeError> {
        self.0.stop_bot(bot).await
    }
    async fn shutdown(&self) {
        self.0.shutdown().await
    }
    async fn health(&self) -> Result<RuntimeHealth, RuntimeError> {
        self.0.health().await
    }
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<RuntimeEvent> {
        self.0.subscribe()
    }
}

struct WithoutHerdrSessionIds(Arc<HerdrRuntime>);

#[async_trait::async_trait]
impl BotRuntime for WithoutHerdrSessionIds {
    async fn start_bot(&self, spec: &LaunchSpec) -> Result<RuntimeBot, RuntimeError> {
        let mut bot = self.0.start_bot(spec).await?;
        bot.session_id = None;
        Ok(bot)
    }
    async fn prompt(&self, bot: &RuntimeBot, text: &str) -> Result<(), RuntimeError> {
        self.0.prompt(bot, text).await
    }
    async fn inspect(&self, bot: &RuntimeBot) -> Result<Inspection, RuntimeError> {
        let mut inspection = self.0.inspect(bot).await?;
        inspection.session_id = None;
        Ok(inspection)
    }
    async fn read_output(&self, bot: &RuntimeBot, lines: u32) -> Result<String, RuntimeError> {
        self.0.read_output(bot, lines).await
    }
    async fn snapshot(&self, bot: &RuntimeBot, lines: u32) -> Result<String, RuntimeError> {
        self.0.snapshot(bot, lines).await
    }
    async fn terminal(&self, bot: &RuntimeBot, lines: u32) -> Result<String, RuntimeError> {
        self.0.terminal(bot, lines).await
    }
    async fn send_keys(&self, bot: &RuntimeBot, keys: &[&str]) -> Result<(), RuntimeError> {
        self.0.send_keys(bot, keys).await
    }
    async fn stop_bot(&self, bot: &RuntimeBot) -> Result<(), RuntimeError> {
        self.0.stop_bot(bot).await
    }
    async fn shutdown(&self) {
        self.0.shutdown().await
    }
    async fn health(&self) -> Result<RuntimeHealth, RuntimeError> {
        self.0.health().await
    }
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<RuntimeEvent> {
        self.0.subscribe()
    }
}

type Wrap = fn(Arc<HerdrRuntime>) -> Arc<dyn BotRuntime>;

fn as_is(rt: Arc<HerdrRuntime>) -> Arc<dyn BotRuntime> {
    rt
}

fn without_user_settings(rt: Arc<HerdrRuntime>) -> Arc<dyn BotRuntime> {
    Arc::new(WithoutUserSettings(rt))
}

fn without_herdr_session_ids(rt: Arc<HerdrRuntime>) -> Arc<dyn BotRuntime> {
    Arc::new(WithoutHerdrSessionIds(rt))
}

fn new_engine(db: &std::path::Path, bin: &Path, session: &str, paths: &AppPaths) -> Engine {
    new_engine_with(db, bin, session, paths, as_is)
}

fn new_engine_with(db: &std::path::Path, bin: &Path, session: &str, paths: &AppPaths, wrap: Wrap) -> Engine {
    let engine = Engine::new(
        Store::open(db).unwrap(),
        wrap(HerdrRuntime::new(bin.to_path_buf(), session.to_string())),
        EngineConfig {
            paths: paths.clone(),
            sbot_dir: sbot_dir(),
            fallback_lines: 200,
            turn_start_grace: Some(Duration::from_secs(10)),
        },
    );
    engine.run_event_loop();
    engine
}

#[tokio::test]
#[ignore = "needs herdr and a logged-in claude; run with --ignored"]
async fn claude_bot_resumes_after_restart() {
    let bin = herdr_client::find_herdr_bin().expect("herdr installed");
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    let session = format!("sonata-test-{}-{nanos}", std::process::id());
    let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };

    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
    paths.ensure().unwrap();
    let db = paths.db_path();

    let engine = new_engine(&db, &bin, &session, &paths);
    let bridge = sonata_core::bridge::serve(&paths.bridge_socket(), engine.clone()).unwrap();
    let bot = engine
        .create_bot(NewBot {
            display_name: "Resume Test".into(),
            color: "#3b82f6".into(),
            provider: Provider::Claude,
            model: None,
            instructions: "Answer in one short sentence.".into(),
            skill_ids: vec![],
        })
        .unwrap();
    engine.queue_user_message(&bot.id, "Remember this secret word: PELICAN. Just reply OK.").unwrap();
    engine.pump(&bot.id).await;
    let first = wait_for_bot_text(&engine, &bot.id, &Default::default()).await;
    println!("first answer {:?}: {}", first.kind, first.body);
    assert_eq!(first.kind, MessageKind::Text, "first answer was not via sbot reply");

    engine.shutdown().await;
    bridge.abort();
    let _ = bridge.await;
    drop(engine);

    let engine = new_engine(&db, &bin, &session, &paths);
    let mut bridge2 = None;
    for _ in 0..50 {
        match sonata_core::bridge::serve(&paths.bridge_socket(), engine.clone()) {
            Ok(h) => {
                bridge2 = Some(h);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    let _bridge2 = bridge2.expect("bridge re-bound after restart");

    let stored = Store::open(&db).unwrap();
    assert!(stored.list_bots().unwrap().iter().any(|b| b.id == bot.id), "bot persisted");
    let saved = saved_cli_id(&db, &bot.id);
    println!("provider_session_id: {saved:?}");
    assert!(saved.is_some(), "provider_session_id was not recorded");

    let seen: std::collections::HashSet<String> =
        engine.user_thread(&bot.id).unwrap().into_iter().map(|m| m.id).collect();
    engine
        .queue_user_message(&bot.id, "What secret word did I ask you to remember? Reply with just the word.")
        .unwrap();
    engine.pump(&bot.id).await;
    let second = wait_for_bot_text(&engine, &bot.id, &seen).await;
    println!("second answer {:?}: {}", second.kind, second.body);
    assert_eq!(second.kind, MessageKind::Text);
    assert!(second.body.to_uppercase().contains("PELICAN"), "bot did not remember the word: {}", second.body);
    engine.shutdown().await;
}

#[tokio::test]
#[ignore = "needs herdr and a logged-in claude; run with --ignored"]
async fn claude_bot_with_a_lost_session_starts_fresh() {
    let bin = herdr_client::find_herdr_bin().expect("herdr installed");
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    let session = format!("sonata-test-{}-{nanos}", std::process::id());
    let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };

    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
    paths.ensure().unwrap();
    let db = paths.db_path();

    let engine = new_engine(&db, &bin, &session, &paths);
    let bridge = sonata_core::bridge::serve(&paths.bridge_socket(), engine.clone()).unwrap();
    let bot = engine
        .create_bot(NewBot {
            display_name: "Lost Session".into(),
            color: "#3b82f6".into(),
            provider: Provider::Claude,
            model: None,
            instructions: "Answer in one short sentence.".into(),
            skill_ids: vec![],
        })
        .unwrap();
    engine.queue_user_message(&bot.id, "Just reply OK.").unwrap();
    engine.pump(&bot.id).await;
    let first = wait_for_bot_text(&engine, &bot.id, &Default::default()).await;
    println!("first answer {:?}: {}", first.kind, first.body);
    engine.shutdown().await;
    bridge.abort();
    let _ = bridge.await;
    drop(engine);

    let lost = "00000000-0000-4000-8000-000000000000";
    save_cli_id(&db, &bot, lost);

    let runtime = HerdrRuntime::new(bin.clone(), session.clone());
    let engine = Engine::new(
        Store::open(&db).unwrap(),
        runtime.clone(),
        EngineConfig {
            paths: paths.clone(),
            sbot_dir: sbot_dir(),
            fallback_lines: 200,
            turn_start_grace: Some(Duration::from_secs(10)),
        },
    );
    engine.run_event_loop();
    let mut bridge2 = None;
    for _ in 0..50 {
        match sonata_core::bridge::serve(&paths.bridge_socket(), engine.clone()) {
            Ok(h) => {
                bridge2 = Some(h);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    let _bridge2 = bridge2.expect("bridge re-bound after restart");
    let seen: std::collections::HashSet<String> =
        engine.user_thread(&bot.id).unwrap().into_iter().map(|m| m.id).collect();
    engine.queue_user_message(&bot.id, "What is 3 + 4? Reply with just the number.").unwrap();
    engine.pump(&bot.id).await;
    let answer = wait_for_bot_text(&engine, &bot.id, &seen).await;
    println!("answer {:?}: {}", answer.kind, answer.body);
    assert_eq!(answer.kind, MessageKind::Text);
    assert!(answer.body.contains('7'));
    let resets = engine
        .user_thread(&bot.id)
        .unwrap()
        .iter()
        .filter(|m| m.system_code.as_deref() == Some("session_reset"))
        .count();
    assert_eq!(resets, 1, "one session_reset notice");
    let stored = saved_cli_id(&db, &bot.id);
    println!("provider_session_id after the reset: {stored:?}");
    let stored = stored.expect("the fresh session id was stored");
    assert_ne!(stored, lost);
    assert_eq!(stored.len(), 36, "a UUID");

    let failed = runtime
        .start_bot(&LaunchSpec {
            handle: "lost-probe".into(),
            agent: "lost-probe".into(),
            label: "lost-probe".into(),
            kind: "claude".into(),
            cwd: paths.bot_dir(&bot.handle),
            args: vec![
                "--dangerously-skip-permissions".into(),
                "--settings".into(),
                r#"{"skipDangerousModePermissionPrompt":true}"#.into(),
                "--resume".into(),
                lost.into(),
            ],
            env: Default::default(),
            path_prefix: sbot_dir(),
            trust: ClaudeAdapter.trust_prompt(),
            sign_in: None,
        })
        .await;
    match failed {
        Err(RuntimeError::StartFailed(message)) => {
            println!("start failure: {message}");
            assert!(message.contains("the screen shows:"), "{message}");
            assert!(message.contains("No conversation found"), "{message}");
        }
        other => panic!("expected StartFailed, got {other:?}"),
    }
    engine.shutdown().await;
}

#[tokio::test]
#[ignore = "needs herdr and a logged-in claude; run with --ignored"]
async fn untrusted_folder_with_a_lost_session_recovers_without_touching_the_shell() {
    let bin = herdr_client::find_herdr_bin().expect("herdr installed");
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    let session = format!("sonata-test-{}-{nanos}", std::process::id());
    let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };

    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
    paths.ensure().unwrap();
    let db = paths.db_path();
    let runtime = HerdrRuntime::new(bin.clone(), session.clone());
    let engine = Engine::new(
        Store::open(&db).unwrap(),
        runtime.clone(),
        EngineConfig {
            paths: paths.clone(),
            sbot_dir: sbot_dir(),
            fallback_lines: 200,
            turn_start_grace: Some(Duration::from_secs(10)),
        },
    );
    engine.run_event_loop();
    let _bridge = sonata_core::bridge::serve(&paths.bridge_socket(), engine.clone()).unwrap();
    let bot = engine
        .create_bot(NewBot {
            display_name: "Untrusted Lost".into(),
            color: "#3b82f6".into(),
            provider: Provider::Claude,
            model: None,
            instructions: "Answer in one short sentence.".into(),
            skill_ids: vec![],
        })
        .unwrap();
    let lost = uuid::Uuid::new_v4().to_string();
    save_cli_id(&db, &bot, &lost);

    engine.queue_user_message(&bot.id, "What is 3 + 4? Reply with just the number.").unwrap();
    engine.pump(&bot.id).await;
    let answer = wait_for_bot_text(&engine, &bot.id, &Default::default()).await;
    println!("answer {:?}: {}", answer.kind, answer.body);
    assert_eq!(answer.kind, MessageKind::Text);
    assert!(answer.body.contains('7'), "{}", answer.body);

    let thread = engine.user_thread(&bot.id).unwrap();
    for m in &thread {
        println!("{:?} {:?} {:?} {}", m.kind, m.status, m.system_code, m.body);
    }
    let resets = thread.iter().filter(|m| m.system_code.as_deref() == Some("session_reset")).count();
    assert_eq!(resets, 1, "one session_reset notice");
    assert!(thread.iter().all(|m| m.status != MessageStatus::Interrupted), "no turn was cut off");
    let unanswered = Store::open(&db).unwrap().unanswered_deliveries().unwrap();
    assert!(unanswered.is_empty(), "delivered without an answer: {unanswered:?}");
    let stored = saved_cli_id(&db, &bot.id).expect("the fresh session id was stored");
    assert_ne!(stored, lost);

    let probe_dir = tmp.path().join("untrusted-probe");
    std::fs::create_dir_all(&probe_dir).unwrap();
    let failed = runtime
        .start_bot(&LaunchSpec {
            handle: "untrusted-probe".into(),
            agent: "untrusted-probe".into(),
            label: "untrusted-probe".into(),
            kind: "claude".into(),
            cwd: probe_dir,
            args: vec![
                "--dangerously-skip-permissions".into(),
                "--settings".into(),
                r#"{"skipDangerousModePermissionPrompt":true}"#.into(),
                "--resume".into(),
                lost,
            ],
            env: Default::default(),
            path_prefix: sbot_dir(),
            trust: ClaudeAdapter.trust_prompt(),
            sign_in: None,
        })
        .await;
    match failed {
        Err(RuntimeError::StartFailed(message)) => {
            println!("start failure: {message}");
            assert!(message.contains("No conversation found"), "{message}");
        }
        other => panic!("expected StartFailed, got {other:?}"),
    }
    engine.shutdown().await;
}

async fn answers_and_resumes(provider: Provider, check_resume: bool) {
    answers_and_resumes_with(provider, check_resume, as_is).await;
}

async fn answers_and_resumes_with(provider: Provider, check_resume: bool, wrap: Wrap) {
    assert!(std::env::var_os("SONATA_HERDR_BIN").is_some(), "set SONATA_HERDR_BIN to the pinned herdr");
    let bin = herdr_client::find_herdr_bin().expect("herdr installed");
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    let session = format!("sonata-test-{}-{nanos}", std::process::id());
    let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
    paths.ensure().unwrap();
    let db = paths.db_path();
    let _opencode = OpencodeSessions(db.clone());

    let engine = new_engine_with(&db, &bin, &session, &paths, wrap);
    let bridge = sonata_core::bridge::serve(&paths.bridge_socket(), engine.clone()).unwrap();
    let bot = engine
        .create_bot(NewBot {
            display_name: format!("{} Live", provider.as_str()),
            color: "#3b82f6".into(),
            provider,
            model: live_model(provider),
            instructions: "Answer in one short sentence.".into(),
            skill_ids: vec![],
        })
        .unwrap();
    engine.queue_user_message(&bot.id, "Remember this secret word: PELICAN. Then tell me: what is 2 + 2?").unwrap();
    engine.pump(&bot.id).await;
    let first = wait_for_bot_text(&engine, &bot.id, &Default::default()).await;
    println!("{} first answer {:?}: {}", provider.as_str(), first.kind, first.body);
    if first.kind == MessageKind::Text {
        assert!(first.body.contains('4'), "{}", first.body);
    } else {
        println!("WARNING: the first answer is raw output, not sbot reply");
    }
    if !check_resume {
        engine.shutdown().await;
        return;
    }

    engine.shutdown().await;
    bridge.abort();
    let _ = bridge.await;
    drop(engine);
    let session_before = saved_cli_id(&db, &bot.id);
    assert!(session_before.is_some(), "no provider session id was recorded");

    let engine = new_engine_with(&db, &bin, &session, &paths, wrap);
    let mut bridge2 = None;
    for _ in 0..50 {
        match sonata_core::bridge::serve(&paths.bridge_socket(), engine.clone()) {
            Ok(h) => {
                bridge2 = Some(h);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    let _bridge2 = bridge2.expect("bridge re-bound after restart");
    let seen: std::collections::HashSet<String> =
        engine.user_thread(&bot.id).unwrap().into_iter().map(|m| m.id).collect();
    engine
        .queue_user_message(&bot.id, "What secret word did I ask you to remember? Reply with just the word.")
        .unwrap();
    engine.pump(&bot.id).await;
    let second = wait_for_bot_text(&engine, &bot.id, &seen).await;
    println!("{} second answer {:?}: {}", provider.as_str(), second.kind, second.body);
    if second.kind == MessageKind::Text {
        assert!(second.body.to_uppercase().contains("PELICAN"), "the bot did not remember the word: {}", second.body);
    } else {
        println!("WARNING: the second answer is raw output, not sbot reply");
    }
    let session_after = saved_cli_id(&db, &bot.id);
    assert_eq!(session_after, session_before, "the session was replaced, not resumed");
    let resets = engine
        .user_thread(&bot.id)
        .unwrap()
        .iter()
        .filter(|m| m.system_code.as_deref() == Some("session_reset"))
        .count();
    assert_eq!(resets, 0, "the engine fell back to a fresh session");
    engine.shutdown().await;
}

#[tokio::test]
#[ignore = "needs herdr and a logged-in grok; run with --ignored"]
async fn grok_bot_answers_and_resumes_after_restart() {
    answers_and_resumes(Provider::Grok, true).await;
}

#[tokio::test]
#[ignore = "needs herdr and a logged-in grok; run with --ignored"]
async fn grok_idle_background_work_is_detected_until_the_command_finishes() {
    assert!(std::env::var_os("SONATA_HERDR_BIN").is_some(), "set SONATA_HERDR_BIN to the pinned herdr");
    let bin = herdr_client::find_herdr_bin().expect("herdr installed");
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    let session = format!("sonata-bg-{}-{nanos}", std::process::id());
    let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
    let tmp = tempfile::tempdir().unwrap();
    let runtime = HerdrRuntime::new(bin, &session);
    let bot = runtime
        .start_bot(&LaunchSpec {
            handle: "background-check".into(),
            agent: "grok".into(),
            label: "Grok background check".into(),
            kind: "grok".into(),
            cwd: tmp.path().to_path_buf(),
            args: vec!["--always-approve".into()],
            env: Default::default(),
            path_prefix: sbot_dir(),
            trust: GrokAdapter.trust_prompt(),
            sign_in: None,
        })
        .await
        .unwrap();
    runtime.prompt(&bot, "Start `sleep 45; printf done > background-complete` as a Grok-managed background shell command, using your background execution option, not shell '&' or nohup. Return immediately once started. Do not wait for it, poll it, or cancel it. No other commands.").await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    let mut saw_idle_background = false;
    let mut finished = false;
    let mut last_screen = String::new();
    let mut checks = 0;
    while tokio::time::Instant::now() < deadline {
        last_screen = runtime.snapshot(&bot, 80).await.unwrap();
        let idle = runtime.inspect(&bot).await.unwrap().state == sonata_core::runtime::AgentState::Idle;
        let background = GrokAdapter.background_work(&last_screen);
        if checks % 10 == 0 {
            println!("Grok background check (idle={idle}, background={background}):\n{last_screen}");
        }
        checks += 1;
        if idle && background && !saw_idle_background {
            println!("Grok idle with background work:\n{last_screen}");
            saw_idle_background = true;
        }
        if saw_idle_background && idle && !background && tmp.path().join("background-complete").exists() {
            println!("Grok idle after background work:\n{last_screen}");
            finished = true;
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    runtime.shutdown().await;
    assert!(saw_idle_background, "never detected Grok's idle background work:\n{last_screen}");
    assert!(finished, "background command did not finish and clear its indicator:\n{last_screen}");
}

#[tokio::test]
#[ignore = "needs herdr and logged-in claude and grok; run with --ignored"]
async fn three_bots_on_claude_and_grok_pass_work_from_one_request() {
    assert!(std::env::var_os("SONATA_HERDR_BIN").is_some(), "set SONATA_HERDR_BIN to the pinned herdr");
    let bin = herdr_client::find_herdr_bin().expect("herdr installed");
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    let session = format!("sonata-test-{}-{nanos}", std::process::id());
    let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
    paths.ensure().unwrap();
    let engine = new_engine(&paths.db_path(), &bin, &session, &paths);
    let _bridge = sonata_core::bridge::serve(&paths.bridge_socket(), engine.clone()).unwrap();
    let _watchdog = engine.run_watchdog(Duration::from_secs(15));

    let bot = |name: &str, provider: Provider, instructions: &str| {
        engine
            .create_bot(NewBot {
                display_name: name.into(),
                color: "#3b82f6".into(),
                provider,
                model: None,
                instructions: instructions.into(),
                skill_ids: vec![],
            })
            .unwrap()
    };
    let lead = bot(
        "Lead",
        Provider::Claude,
        "You coordinate @upper and @mirror. When the user gives you a word, send the word to @upper \
         and to @mirror with `sbot send`, reply to the user that you asked them, and end your turn. \
         When both have answered, send the user one message in the form \
         `UPPER: <answer> MIRROR: <answer>`.",
    );
    let upper = bot(
        "Upper",
        Provider::Grok,
        "When a bot sends you a word, reply with the word in capital letters and nothing else.",
    );
    let mirror = bot(
        "Mirror",
        Provider::Claude,
        "When a bot sends you a word, reply with the word spelled backwards and nothing else.",
    );
    engine.queue_user_message(&lead.id, "The word is pelican.").unwrap();
    engine.pump(&lead.id).await;

    let mut combined = None;
    for _ in 0..360 {
        let thread = engine.user_thread(&lead.id).unwrap();
        if let Some(m) = thread.iter().find(|m| {
            m.sender == lead.id
                && m.kind == MessageKind::Text
                && m.body.contains("PELICAN")
                && m.body.to_lowercase().contains("nacilep")
        }) {
            combined = Some(m.clone());
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let name = |id: &str| {
        [&lead, &upper, &mirror].iter().find(|b| b.id == id).map_or(id.to_string(), |b| format!("@{}", b.handle))
    };
    let dump = || {
        for t in engine.bot_threads().unwrap() {
            println!("thread {} ↔ {}:", name(&t.participant_a), name(&t.participant_b));
            for m in engine.thread_messages(&t.id).unwrap() {
                let reply = if m.in_reply_to.is_some() { " (answer)" } else { "" };
                println!(
                    "  {:?} {:?} {} → {}{reply}: {}",
                    m.kind,
                    m.status,
                    name(&m.sender),
                    name(&m.recipient),
                    m.body
                );
            }
        }
        for b in [&lead, &upper, &mirror] {
            println!("user thread with @{}:", b.handle);
            for m in engine.user_thread(&b.id).unwrap() {
                let text = m.system_code.clone().unwrap_or(m.body);
                println!("  {:?} {:?} {} → {}: {text}", m.kind, m.status, name(&m.sender), name(&m.recipient));
            }
        }
    };
    let Some(combined) = combined else {
        dump();
        panic!("no combined answer within 6 minutes");
    };
    println!("final answer: {}", combined.body);

    let bot_messages = || -> Vec<Message> {
        engine.bot_threads().unwrap().iter().flat_map(|t| engine.thread_messages(&t.id).unwrap()).collect()
    };
    let (mut quiet, mut last_count, mut settled) = (0, usize::MAX, false);
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let messages = bot_messages();
        let busy = engine
            .list_bots()
            .await
            .unwrap()
            .iter()
            .any(|v| matches!(v.status, BotStatus::Working | BotStatus::Starting));
        let idle = !busy && messages.iter().all(|m| m.status != MessageStatus::Queued) && messages.len() == last_count;
        quiet = if idle { quiet + 1 } else { 0 };
        last_count = messages.len();
        if quiet >= 10 {
            settled = true;
            break;
        }
    }
    dump();
    let limit_notices = [&lead, &upper, &mirror]
        .iter()
        .flat_map(|b| engine.user_thread(&b.id).unwrap())
        .filter(|m| m.system_code.as_deref() == Some("chain_limit_reached"))
        .count();
    let threads = engine.bot_threads().unwrap();
    assert!(settled, "the bot threads did not settle within 60 s");
    assert_eq!(limit_notices, 0, "no chain reached the hop limit");
    assert!(bot_messages().iter().all(|m| m.status != MessageStatus::Queued), "nothing is left queued between bots");
    assert_eq!(threads.len(), 2, "lead↔upper and lead↔mirror");
    for t in &threads {
        let messages = engine.thread_messages(&t.id).unwrap();
        assert!(messages.iter().any(|m| m.sender == lead.id), "the lead wrote in every thread");
        let answer = messages.iter().find(|m| m.sender != lead.id).expect("every teammate answered");
        println!("teammate {} answered with {:?}", answer.sender, answer.kind);
    }
    engine.shutdown().await;
}

#[tokio::test]
#[ignore = "needs herdr and a logged-in claude; run with --ignored"]
async fn claude_bot_answers_and_resumes_without_the_users_settings() {
    answers_and_resumes_with(Provider::Claude, true, without_user_settings).await;
}

#[tokio::test]
#[ignore = "needs herdr and a logged-in claude; run with --ignored"]
async fn claude_bot_answers_under_finder_conditions() {
    answers_under_finder_conditions(Provider::Claude).await;
}

#[tokio::test]
#[ignore = "needs herdr and opencode; run with --ignored"]
async fn opencode_bot_answers_under_finder_conditions() {
    answers_under_finder_conditions(Provider::Opencode).await;
}

async fn answers_under_finder_conditions(provider: Provider) {
    assert!(std::env::var_os("SONATA_HERDR_BIN").is_some(), "set SONATA_HERDR_BIN to the pinned herdr");
    let bin = herdr_client::find_herdr_bin().expect("herdr installed");
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    let session = format!("sonata-test-{}-{nanos}", std::process::id());
    let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
    paths.ensure().unwrap();
    let _opencode = OpencodeSessions(paths.db_path());
    let bin_dir = tmp.path().join("Sonata bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    std::fs::copy(sbot_dir().join("sbot"), bin_dir.join("sbot")).unwrap();
    let home = dirs::home_dir().unwrap();
    let search_path = sonata_core::detect::search_path(None, &home, Some("/usr/bin:/bin:/usr/sbin:/sbin"));
    let executable = sonata_core::provider::adapter_for(provider).unwrap().executable();
    assert!(
        sonata_core::detect::find_executable(executable, &search_path).is_some(),
        "{executable} is not in one of the usual install folders; the app would not find it from Finder either: \
         {search_path}"
    );
    let apple_locale = sonata_core::detect::apple_locale().await;
    let lang = sonata_core::detect::utf8_lang(None, apple_locale.as_deref());
    let env = vec![
        ("PATH".to_string(), search_path),
        ("SHELL".to_string(), sonata_core::detect::user_shell().display().to_string()),
        ("LANG".to_string(), lang),
    ];
    let engine = Engine::new(
        Store::open(&paths.db_path()).unwrap(),
        HerdrRuntime::new_with_env(bin, session, env),
        EngineConfig {
            paths: paths.clone(),
            sbot_dir: bin_dir,
            fallback_lines: 200,
            turn_start_grace: Some(Duration::from_secs(10)),
        },
    );
    engine.run_event_loop();
    let _bridge = sonata_core::bridge::serve(&paths.bridge_socket(), engine.clone()).unwrap();
    let bot = engine
        .create_bot(NewBot {
            display_name: "Finder Test".into(),
            color: "#3b82f6".into(),
            provider,
            model: live_model(provider),
            instructions: "Answer in one short sentence.".into(),
            skill_ids: vec![],
        })
        .unwrap();
    engine.queue_user_message(&bot.id, "¿Cuánto es 2 + 2? Responde con el número y la palabra «cuatro».").unwrap();
    engine.pump(&bot.id).await;
    let answer = wait_for_bot_text(&engine, &bot.id, &Default::default()).await;
    println!("finder conditions answer {:?}: {}", answer.kind, answer.body);
    assert_eq!(answer.kind, MessageKind::Text, "bot fell back to raw output instead of sbot reply");
    assert!(answer.body.contains('4') || answer.body.to_lowercase().contains("cuatro"), "{}", answer.body);
    let session = engine.sessions_of(&bot.id).unwrap().remove(0);
    let terminal = engine.terminal(&session.id).await.unwrap().expect("the bot is running");
    let head = String::from_utf8_lossy(&terminal.as_bytes()[..terminal.len().min(200)]);
    println!("finder conditions terminal (first 200 bytes): {head:?}");
    assert!(terminal.contains("\u{1b}["), "terminal text has no ANSI escapes: {terminal:?}");
    engine.shutdown().await;
}

async fn uses_an_attached_skill(provider: Provider) {
    assert!(std::env::var_os("SONATA_HERDR_BIN").is_some(), "set SONATA_HERDR_BIN to the pinned herdr");
    let bin = herdr_client::find_herdr_bin().expect("herdr installed");
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    let session = format!("sonata-test-{}-{nanos}", std::process::id());
    let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
    paths.ensure().unwrap();
    let _opencode = OpencodeSessions(paths.db_path());
    let engine = new_engine(&paths.db_path(), &bin, &session, &paths);
    let _bridge = sonata_core::bridge::serve(&paths.bridge_socket(), engine.clone()).unwrap();

    let skill = engine
        .create_skill(NewSkill {
            name: "Secret word".into(),
            description: "Tells the secret word when asked.".into(),
            body: "The secret word is PELICAN. When asked for the secret word, reply with just the word.".into(),
        })
        .unwrap();
    let bot = engine
        .create_bot(NewBot {
            display_name: format!("{} Skill", provider.as_str()),
            color: "#3b82f6".into(),
            provider,
            model: live_model(provider),
            instructions: "Answer in one short sentence.".into(),
            skill_ids: vec![skill.id.clone()],
        })
        .unwrap();
    engine.queue_user_message(&bot.id, "What is the secret word? Reply with just the word.").unwrap();
    engine.pump(&bot.id).await;
    let answer = wait_for_bot_text(&engine, &bot.id, &Default::default()).await;
    println!("{} skill answer {:?}: {}", provider.as_str(), answer.kind, answer.body);

    let skill_dir = match provider {
        Provider::Claude => ".claude/skills",
        _ => ".agents/skills",
    };
    let dir = paths.bot_dir(&bot.handle);
    let copy = dir.join(skill_dir).join(&skill.slug).join("SKILL.md");
    assert!(copy.is_file(), "no skill copy at {}", copy.display());
    let agents = std::fs::read_to_string(dir.join("AGENTS.md")).unwrap();
    let index_line = format!("`{skill_dir}/{}/SKILL.md`", skill.slug);
    assert!(agents.contains("## Your skills") && agents.contains(&index_line), "{agents}");
    assert_eq!(answer.kind, MessageKind::Text, "bot fell back to raw output instead of sbot reply");
    assert!(answer.body.to_uppercase().contains("PELICAN"), "the bot did not use its skill: {}", answer.body);
    engine.shutdown().await;
}

#[tokio::test]
#[ignore = "needs herdr and a logged-in claude; run with --ignored"]
async fn claude_bot_uses_an_attached_skill() {
    uses_an_attached_skill(Provider::Claude).await;
}

#[tokio::test]
#[ignore = "needs herdr and a logged-in grok; run with --ignored"]
async fn grok_bot_uses_an_attached_skill() {
    uses_an_attached_skill(Provider::Grok).await;
}

fn session_cli_id(db: &Path, session_id: &str) -> Option<String> {
    Store::open(db).unwrap().get_session(session_id).unwrap().and_then(|s| s.provider_session_id)
}

async fn wait_for_session_text(
    engine: &Engine,
    session: &Session,
    seen: &std::collections::HashSet<String>,
) -> Message {
    for _ in 0..180 {
        let chat = engine.session_messages(&session.id).unwrap();
        if let Some(m) =
            chat.iter().find(|m| m.sender == session.bot_id && m.kind != MessageKind::System && !seen.contains(&m.id))
        {
            return m.clone();
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let chat: Vec<String> = engine
        .session_messages(&session.id)
        .unwrap()
        .iter()
        .map(|m| format!("{:?} {:?} {:?} {:?} {}", m.kind, m.status, m.system_code, m.system_params, m.body))
        .collect();
    panic!("session {} did not answer within 180 s; chat:\n{}", session.number, chat.join("\n"));
}

fn assert_in_its_own_chat(engine: &Engine, session: &Session, answer: &Message) {
    assert_eq!(answer.kind, MessageKind::Text, "session {} fell back to raw output: {}", session.number, answer.body);
    assert_eq!(answer.sender_session.as_deref(), Some(session.id.as_str()), "answered from another session");
    assert_eq!(answer.recipient, USER);
    for m in engine.session_messages(&session.id).unwrap() {
        let end = if m.sender == USER { &m.recipient_session } else { &m.sender_session };
        assert_eq!(end.as_deref(), Some(session.id.as_str()), "a message of another session: {}", m.body);
    }
}

async fn wait_until_not_busy(engine: &Engine, session: &Session) {
    for _ in 0..720 {
        let status = engine.find_session_view(&session.id).await.unwrap().status;
        if !matches!(status, BotStatus::Working | BotStatus::Starting) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("session {} was still busy after 180 s", session.number);
}

async fn send_to_both(engine: &Engine, sessions: [&Session; 2], bodies: [&str; 2]) -> ([Message; 2], bool) {
    wait_until_not_busy(engine, sessions[0]).await;
    wait_until_not_busy(engine, sessions[1]).await;
    let seen = |s: &Session| -> std::collections::HashSet<String> {
        engine.session_messages(&s.id).unwrap().into_iter().map(|m| m.id).collect()
    };
    let (seen_a, seen_b) = (seen(sessions[0]), seen(sessions[1]));
    let sent_a = engine.queue_user_message_to_session(&sessions[0].id, bodies[0], &[]).unwrap();
    let sent_b = engine.queue_user_message_to_session(&sessions[1].id, bodies[1], &[]).unwrap();
    tokio::join!(engine.pump_session(&sessions[0].id), engine.pump_session(&sessions[1].id));
    let (a, b) = tokio::join!(
        wait_for_session_text(engine, sessions[0], &seen_a),
        wait_for_session_text(engine, sessions[1], &seen_b)
    );
    let mut delivered = Vec::new();
    for (session, sent, answer) in [(sessions[0], &sent_a, &a), (sessions[1], &sent_b, &b)] {
        assert_in_its_own_chat(engine, session, answer);
        let chat = engine.session_messages(&session.id).unwrap();
        let sent = chat.into_iter().find(|m| m.id == sent.id).expect("the message is in its session's chat");
        let at = sent.delivered_at.expect("the message was delivered");
        println!("session {}: delivered at {at}, answered at {}", session.number, answer.created_at);
        delivered.push(at);
    }
    let overlapped = delivered.iter().max() < [&a.created_at, &b.created_at].into_iter().min();
    ([a, b], overlapped)
}

fn assert_each_knows_only_its_word(provider: Provider, step: &str, answers: &[Message; 2], words: [&str; 2]) {
    for (i, answer) in answers.iter().enumerate() {
        let (own, other) = (words[i], words[1 - i]);
        println!("{} {step}, session given {own:?}: {}", provider.as_str(), answer.body);
        let body = answer.body.to_lowercase();
        assert!(body.contains(own), "the session given {own:?} does not know it: {}", answer.body);
        assert!(!body.contains(other), "the session given {own:?} knows the other session's word: {}", answer.body);
    }
}

async fn two_sessions_keep_and_resume_their_own_conversations(provider: Provider) {
    assert!(std::env::var_os("SONATA_HERDR_BIN").is_some(), "set SONATA_HERDR_BIN to the pinned herdr");
    let bin = herdr_client::find_herdr_bin().expect("herdr installed");
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    let session = format!("sonata-test-{}-{nanos}", std::process::id());
    let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
    paths.ensure().unwrap();
    let db = paths.db_path();
    let _opencode = OpencodeSessions(db.clone());

    let engine = new_engine(&db, &bin, &session, &paths);
    let bridge = sonata_core::bridge::serve(&paths.bridge_socket(), engine.clone()).unwrap();
    let bot = engine
        .create_bot(NewBot {
            display_name: format!("{} Sessions", provider.as_str()),
            color: "#3b82f6".into(),
            provider,
            model: live_model(provider),
            instructions: "Answer in one short sentence. Never write to MEMORY.md or to any other file.".into(),
            skill_ids: vec![],
        })
        .unwrap();
    let (a, _) = engine.start_session_with_message(&bot.id, "Just reply OK.", &[]).unwrap();
    let (b, _) = engine.start_session_with_message(&bot.id, "Just reply OK.", &[]).unwrap();
    assert_ne!(a.id, b.id);
    tokio::join!(engine.pump_session(&a.id), engine.pump_session(&b.id));
    let none = Default::default();
    let (ok_a, ok_b) =
        tokio::join!(wait_for_session_text(&engine, &a, &none), wait_for_session_text(&engine, &b, &none));
    for (session, answer) in [(&a, &ok_a), (&b, &ok_b)] {
        println!("{} session {} started: {}", provider.as_str(), session.number, answer.body);
        assert_in_its_own_chat(&engine, session, answer);
    }

    let words = ["apple", "banana"];
    let tell =
        |word: &str| format!("The word for this conversation is {word}. Do not write it to any file. Just reply OK.");
    let (told, overlapped) = send_to_both(&engine, [&a, &b], [&tell(words[0]), &tell(words[1])]).await;
    println!("{} told: {:?} / {:?}", provider.as_str(), told[0].body, told[1].body);
    assert!(overlapped, "the two sessions were not told their words at the same time");
    let question = "Which word did I give you in this conversation, and do you know any other word I gave you? \
                    Answer in one short sentence.";
    let (answers, overlapped) = send_to_both(&engine, [&a, &b], [question, question]).await;
    assert_each_knows_only_its_word(provider, "answer", &answers, words);
    assert!(overlapped, "the two sessions were not asked at the same time");

    engine.shutdown().await;
    bridge.abort();
    let _ = bridge.await;
    drop(engine);
    let memory = std::fs::read_to_string(paths.bot_dir(&bot.handle).join("MEMORY.md")).unwrap_or_default();
    println!("{} MEMORY.md: {memory:?}", provider.as_str());
    let before = [session_cli_id(&db, &a.id), session_cli_id(&db, &b.id)];
    println!("{} CLI sessions: {before:?}", provider.as_str());
    assert!(before.iter().all(Option::is_some), "a session saved no CLI session id");
    assert_ne!(before[0], before[1], "the two sessions share one CLI session");

    let engine = new_engine(&db, &bin, &session, &paths);
    let mut bridge2 = None;
    for _ in 0..50 {
        match sonata_core::bridge::serve(&paths.bridge_socket(), engine.clone()) {
            Ok(h) => {
                bridge2 = Some(h);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    let _bridge2 = bridge2.expect("bridge re-bound after restart");
    let (answers, overlapped) = send_to_both(&engine, [&a, &b], [question, question]).await;
    println!("{} turns after the restart overlapped: {overlapped}", provider.as_str());
    assert_each_knows_only_its_word(provider, "answer after the restart", &answers, words);
    let after = [session_cli_id(&db, &a.id), session_cli_id(&db, &b.id)];
    assert_eq!(after, before, "a session's CLI session was replaced, not resumed");
    for s in [&a, &b] {
        let resets = engine
            .session_messages(&s.id)
            .unwrap()
            .iter()
            .filter(|m| m.system_code.as_deref() == Some("session_reset"))
            .count();
        assert_eq!(resets, 0, "session {} fell back to a fresh CLI session", s.number);
    }
    engine.shutdown().await;
}

#[tokio::test]
#[ignore = "needs herdr and a logged-in claude; run with --ignored"]
async fn two_claude_sessions_of_one_bot_keep_and_resume_their_own_conversations() {
    two_sessions_keep_and_resume_their_own_conversations(Provider::Claude).await;
}

#[tokio::test]
#[ignore = "needs herdr and a logged-in grok; run with --ignored"]
async fn two_grok_sessions_of_one_bot_keep_and_resume_their_own_conversations() {
    two_sessions_keep_and_resume_their_own_conversations(Provider::Grok).await;
}

#[tokio::test]
#[ignore = "needs herdr and a logged-in claude; run with --ignored"]
async fn claude_bot_reports_when_its_background_work_ends() {
    let bin = herdr_client::find_herdr_bin().expect("herdr installed");
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    let session = format!("sonata-test-{}-{nanos}", std::process::id());
    let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };

    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
    paths.ensure().unwrap();
    let engine = new_engine(&tmp.path().join("db.sqlite"), &bin, &session, &paths);
    sonata_core::bridge::serve(&paths.bridge_socket(), engine.clone()).unwrap();

    let bot = engine
        .create_bot(NewBot {
            display_name: "Live Test".into(),
            color: "#3b82f6".into(),
            provider: Provider::Claude,
            model: None,
            instructions: "Follow the user's steps exactly.".into(),
            skill_ids: vec![],
        })
        .unwrap();
    engine
        .queue_user_message(
            &bot.id,
            "Do these steps in order:\n\
             1. With your Bash tool and run_in_background set to true, run: sleep 40 && echo finished-sleeping\n\
             2. Send me the word started with sbot reply.\n\
             3. End your turn at once. Do not wait for the background command.",
        )
        .unwrap();
    engine.pump(&bot.id).await;

    let started = wait_for_bot_text(&engine, &bot.id, &std::collections::HashSet::new()).await;
    println!("{:?}: {}", started.kind, started.body);
    let report = wait_for_bot_text(&engine, &bot.id, &std::collections::HashSet::from([started.id.clone()])).await;
    println!("{:?}: {}", report.kind, report.body);
    assert!(matches!(report.kind, MessageKind::Text | MessageKind::RawFallback), "{report:?}");
    engine.shutdown().await;
}

#[tokio::test]
#[ignore = "needs herdr and a signed-in codex; run with --ignored"]
async fn codex_bot_answers_and_resumes_after_restart() {
    answers_and_resumes(Provider::Codex, true).await;
}

#[tokio::test]
#[ignore = "needs herdr and a signed-in codex; run with --ignored"]
async fn codex_bot_resumes_with_only_the_id_sbot_reported() {
    answers_and_resumes_with(Provider::Codex, true, without_herdr_session_ids).await;
}

#[tokio::test]
#[ignore = "needs herdr and a signed-in codex; run with --ignored"]
async fn codex_bot_uses_an_attached_skill() {
    uses_an_attached_skill(Provider::Codex).await;
}

#[tokio::test]
#[ignore = "needs herdr and a signed-in codex; run with --ignored"]
async fn two_codex_sessions_of_one_bot_keep_and_resume_their_own_conversations() {
    two_sessions_keep_and_resume_their_own_conversations(Provider::Codex).await;
}

async fn start_codex(
    runtime: &HerdrRuntime,
    cwd: PathBuf,
    env: std::collections::BTreeMap<String, String>,
) -> Result<RuntimeBot, RuntimeError> {
    let bot = Bot {
        project_id: "default".into(),
        id: "b-live".into(),
        handle: "codex-probe".into(),
        display_name: "Codex Probe".into(),
        color: "#3b82f6".into(),
        provider: Provider::Codex,
        model: None,
        instructions: String::new(),
        config_version: 1,
        skill_ids: vec![],
        created_at: String::new(),
        updated_at: String::new(),
    };
    runtime
        .start_bot(&LaunchSpec {
            handle: "codex-probe".into(),
            agent: "codex-probe".into(),
            label: "codex-probe".into(),
            kind: "codex".into(),
            cwd,
            args: CodexAdapter.launch_args(&bot, None),
            env,
            path_prefix: sbot_dir(),
            trust: CodexAdapter.trust_prompt(),
            sign_in: CodexAdapter.sign_in_text(),
        })
        .await
}

#[tokio::test]
#[ignore = "needs herdr and a signed-in codex; run with --ignored"]
async fn codex_answers_its_trust_menu_in_a_folder_with_a_codex_folder() {
    assert!(std::env::var_os("SONATA_HERDR_BIN").is_some(), "set SONATA_HERDR_BIN to the pinned herdr");
    let bin = herdr_client::find_herdr_bin().expect("herdr installed");
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    let session = format!("sonata-test-{}-{nanos}", std::process::id());
    let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().join("bot");
    std::fs::create_dir_all(cwd.join(".codex")).unwrap();
    let runtime = HerdrRuntime::new(bin, session);
    let started = start_codex(&runtime, cwd, Default::default()).await;
    println!("start result: {started:?}");
    let bot = started.expect("the start answered codex's trust menu and reached its prompt");
    runtime.stop_bot(&bot).await.unwrap();
    runtime.shutdown().await;
}

#[tokio::test]
#[ignore = "needs herdr and codex installed; run with --ignored"]
async fn a_signed_out_codex_fails_its_start_with_the_sign_in_screen() {
    assert!(std::env::var_os("SONATA_HERDR_BIN").is_some(), "set SONATA_HERDR_BIN to the pinned herdr");
    let bin = herdr_client::find_herdr_bin().expect("herdr installed");
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    let session = format!("sonata-test-{}-{nanos}", std::process::id());
    let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().join("bot");
    let empty_home = tmp.path().join("codex-home");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&empty_home).unwrap();
    let env = [("CODEX_HOME".to_string(), empty_home.display().to_string())].into();
    let runtime = HerdrRuntime::new(bin, session);
    let failed = start_codex(&runtime, cwd, env).await;
    match failed {
        Err(RuntimeError::StartFailed(message)) => {
            println!("start failure: {message}");
            assert!(message.contains("Sign in with ChatGPT"), "{message}");
        }
        other => panic!("expected StartFailed, got {other:?}"),
    }
    runtime.shutdown().await;
}

async fn wait_until<T>(what: &str, dump: &dyn Fn() -> String, mut check: impl FnMut() -> Option<T>) -> T {
    for _ in 0..300 {
        if let Some(v) = check() {
            return v;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    panic!("{what} within 5 minutes; chats:\n{}", dump());
}

#[tokio::test]
#[ignore = "needs herdr and a logged-in claude; run with --ignored"]
async fn a_delegated_bot_asks_the_user_and_returns_its_result() {
    assert!(std::env::var_os("SONATA_HERDR_BIN").is_some(), "set SONATA_HERDR_BIN to the pinned herdr");
    let bin = herdr_client::find_herdr_bin().expect("herdr installed");
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    let session = format!("sonata-test-{}-{nanos}", std::process::id());
    let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
    paths.ensure().unwrap();
    let db = paths.db_path();
    let engine = new_engine(&db, &bin, &session, &paths);
    let _bridge = sonata_core::bridge::serve(&paths.bridge_socket(), engine.clone()).unwrap();
    let _watchdog = engine.run_watchdog(Duration::from_secs(15));

    let bot = |name: &str, instructions: &str| {
        engine
            .create_bot(NewBot {
                display_name: name.into(),
                color: "#3b82f6".into(),
                provider: Provider::Claude,
                model: None,
                instructions: instructions.into(),
                skill_ids: vec![],
            })
            .unwrap()
    };
    let lead = bot(
        "Lead",
        "When the user gives you a word, ask @helper with `sbot send` for a rhyme of it and tell the user \
         with `sbot reply` that you asked, then end your turn. Answer a short progress note from @helper \
         with `sbot reply` and the one word ok, then end your turn. When @helper's final result arrives, ask \
         @helper once with `sbot send` to give the rhyme in capital letters. When that answer arrives, \
         send it to the user with `sbot send user`, \
         the rhyme alone, in capital letters.",
    );
    let helper = bot(
        "Helper",
        "When a bot asks you for a rhyme, first ask the user with `sbot ask` whether the rhyme should be \
         one syllable, and end your turn. After the user answers, send that bot a short progress note \
         with `sbot send`, then the rhyme with `sbot result`. When asked to change a result, answer with \
         `sbot reply`.",
    );

    let dump = || -> String {
        let mut out = String::new();
        for b in [&lead, &helper] {
            for s in engine.sessions_of(&b.id).unwrap() {
                out += &format!(
                    "session @{}-{} waiting={:?} open={:?}\n",
                    b.handle, s.number, s.waiting_question_id, s.open_request_id
                );
                for m in engine.session_messages(&s.id).unwrap() {
                    let who = if m.sender == USER { "user".to_string() } else { format!("@{}", b.handle) };
                    out += &format!("  {:?} {:?} {who} → {}: {}\n", m.kind, m.status, m.recipient, m.body);
                }
            }
        }
        for t in engine.bot_threads().unwrap() {
            out += "bot thread:\n";
            for m in engine.thread_messages(&t.id).unwrap() {
                out += &format!("  {:?} {:?} {} → {}: {}\n", m.kind, m.status, m.sender, m.recipient, m.body);
            }
        }
        out
    };

    let (lead_session, _) = engine.start_session_with_message(&lead.id, "The word is moon.", &[]).unwrap();
    engine.pump_session(&lead_session.id).await;

    let question = wait_until("no question from Helper", &dump, || {
        engine.sessions_of(&helper.id).unwrap().into_iter().find_map(|s| {
            engine
                .session_messages(&s.id)
                .unwrap()
                .into_iter()
                .find(|m| m.sender == helper.id && m.recipient == USER && m.kind == MessageKind::Text)
                .map(|m| (s, m))
        })
    })
    .await;
    let (helper_session, question) = question;
    println!("helper asked: {}", question.body);
    let saved = Store::open(&db).unwrap().get_session(&helper_session.id).unwrap().unwrap();
    assert!(saved.waiting_question_id.is_some(), "Helper's session waits on its question\n{}", dump());

    engine.queue_user_message_to_session(&helper_session.id, "yes", &[]).unwrap();
    engine.pump_session(&helper_session.id).await;

    let last = wait_until("no final message from Lead", &dump, || {
        let texts: Vec<Message> = engine
            .session_messages(&lead_session.id)
            .unwrap()
            .into_iter()
            .filter(|m| m.sender == lead.id && m.recipient == USER && m.kind != MessageKind::System)
            .collect();
        (texts.len() >= 2).then(|| texts[texts.len() - 1].clone())
    })
    .await;
    println!("final answer: {}\n{}", last.body, dump());
    assert_eq!(last.kind, MessageKind::Text, "Lead's final message is a sbot message");
    assert!(
        last.body
            .split(|c: char| !c.is_alphabetic())
            .any(|w| w.chars().count() >= 3 && w.chars().all(char::is_uppercase)),
        "Lead's final message holds the rhyme in capital letters: {}",
        last.body
    );

    let store = Store::open(&db).unwrap();
    for b in [&lead, &helper] {
        for s in engine.sessions_of(&b.id).unwrap() {
            assert!(
                engine.session_messages(&s.id).unwrap().iter().all(|m| m.kind != MessageKind::RawFallback),
                "a raw fallback in @{}-{}\n{}",
                b.handle,
                s.number,
                dump()
            );
        }
    }
    for t in engine.bot_threads().unwrap() {
        assert!(
            engine.thread_messages(&t.id).unwrap().iter().all(|m| m.kind != MessageKind::RawFallback),
            "a raw fallback between bots\n{}",
            dump()
        );
    }
    let helper_now = store.get_session(&helper_session.id).unwrap().unwrap();
    assert_eq!(helper_now.open_request_id, None, "Helper's result closed its request\n{}", dump());
    engine.shutdown().await;
}

#[tokio::test]
#[ignore = "needs herdr and a logged-in claude; run with --ignored"]
async fn a_workflow_step_picks_an_outcome_and_the_next_step_gets_its_result() {
    assert!(std::env::var_os("SONATA_HERDR_BIN").is_some(), "set SONATA_HERDR_BIN to the pinned herdr");
    let bin = herdr_client::find_herdr_bin().expect("herdr installed");
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    let session = format!("sonata-test-{}-{nanos}", std::process::id());
    let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
    paths.ensure().unwrap();
    let db = paths.db_path();
    let engine = new_engine(&db, &bin, &session, &paths);
    let _bridge = sonata_core::bridge::serve(&paths.bridge_socket(), engine.clone()).unwrap();
    let _watchdog = engine.run_watchdog(Duration::from_secs(15));

    let bot = |name: &str| {
        engine
            .create_bot(NewBot {
                display_name: name.into(),
                color: "#3b82f6".into(),
                provider: Provider::Claude,
                model: None,
                instructions: "You take part in workflow steps. Do what the step says.".into(),
                skill_ids: vec![],
            })
            .unwrap()
    };
    let first = bot("Chooser");
    let second = bot("Finisher");
    let step = |id: &str, name: &str, bot: &Bot, instructions: &str, arrows: Vec<Arrow>| Step {
        id: id.into(),
        name: name.into(),
        bot_id: bot.id.clone(),
        instructions: instructions.into(),
        arrows,
        pos: None,
    };
    let workflow = engine
        .create_workflow(NewWorkflow {
            name: "Pick".into(),
            when_to_use: String::new(),
            def: WorkflowDef {
                version: WORKFLOW_FORMAT,
                start: Some("s1".into()),
                steps: vec![
                    step(
                        "s1",
                        "Choose",
                        &first,
                        "Reply with sbot result --outcome yes and the text: green.",
                        vec![
                            Arrow { to: "s2".into(), outcome: Some("yes".into()) },
                            Arrow { to: "s2".into(), outcome: Some("no".into()) },
                        ],
                    ),
                    step(
                        "s2",
                        "Finish",
                        &second,
                        "Reply to the user with sbot reply and the one word ok, then send the result with sbot result.",
                        vec![Arrow { to: DONE.into(), outcome: None }],
                    ),
                ],
                start_pos: None,
                done_pos: None,
            },
        })
        .unwrap();

    let dump = || -> String {
        let mut out = String::new();
        for b in [&first, &second] {
            for s in engine.sessions_of(&b.id).unwrap() {
                out += &format!("session @{}-{}\n", b.handle, s.number);
                for m in engine.session_messages(&s.id).unwrap() {
                    out += &format!("  {:?} {:?} {} → {}: {}\n", m.kind, m.status, m.sender, m.recipient, m.body);
                }
            }
        }
        out
    };

    let opened = engine.start_workflow(&workflow.id, "The request is plain.", &[]).unwrap();
    engine.pump_session(&opened.id).await;
    let store = Store::open(&db).unwrap();
    let run = store.latest_run_of_session(&opened.id).unwrap().expect("the run's card session has a run");

    let second_visit = wait_until("no second visit", &dump, || {
        store.run_visits(&run.id).unwrap().into_iter().find(|v| v.ord == 2 && v.session_id.is_some())
    })
    .await;
    let first_visit = store.run_visits(&run.id).unwrap().into_iter().find(|v| v.ord == 1).unwrap();
    assert_eq!(first_visit.outcome.as_deref(), Some("yes"), "step 1 picked its outcome\n{}", dump());
    let step2_session = second_visit.session_id.unwrap();
    engine.pump_session(&step2_session).await;

    let prompt = wait_until("no prompt with the outcome in step 2's chat", &dump, || {
        engine
            .session_messages(&step2_session)
            .unwrap()
            .into_iter()
            .find(|m| m.body.contains("outcome \"yes\"") && m.body.contains("green"))
    })
    .await;
    println!("step 2's prompt: {}\n{}", prompt.body, dump());
    assert!(prompt.body.contains("outcome \"yes\""), "{}", prompt.body);
    assert!(prompt.body.contains("green"), "{}", prompt.body);
    engine.shutdown().await;
}

#[tokio::test]
#[ignore = "needs herdr and opencode; run with --ignored"]
async fn opencode_bot_answers_and_resumes_after_restart() {
    answers_and_resumes(Provider::Opencode, true).await;
}

#[tokio::test]
#[ignore = "needs herdr and opencode; run with --ignored"]
async fn opencode_bot_uses_an_attached_skill() {
    uses_an_attached_skill(Provider::Opencode).await;
}

#[tokio::test]
#[ignore = "needs herdr and opencode; run with --ignored"]
async fn two_opencode_sessions_of_one_bot_keep_and_resume_their_own_conversations() {
    two_sessions_keep_and_resume_their_own_conversations(Provider::Opencode).await;
}

struct Held {
    bridge: tokio::task::JoinHandle<()>,
    _herdr: Cleanup,
    _opencode: OpencodeSessions,
    _tmp: tempfile::TempDir,
}

impl Drop for Held {
    fn drop(&mut self) {
        self.bridge.abort();
    }
}

async fn opencode_bot_with(body: &str) -> (Engine, Bot, Session, Held) {
    assert!(std::env::var_os("SONATA_HERDR_BIN").is_some(), "set SONATA_HERDR_BIN to the pinned herdr");
    let bin = herdr_client::find_herdr_bin().expect("herdr installed");
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    let session = format!("sonata-test-{}-{nanos}", std::process::id());
    let cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
    paths.ensure().unwrap();
    let opencode = OpencodeSessions(paths.db_path());
    let engine = new_engine(&paths.db_path(), &bin, &session, &paths);
    let bridge = sonata_core::bridge::serve(&paths.bridge_socket(), engine.clone()).unwrap();
    let bot = engine
        .create_bot(NewBot {
            display_name: "Opencode Live".into(),
            color: "#3b82f6".into(),
            provider: Provider::Opencode,
            model: live_model(Provider::Opencode),
            instructions: "Answer in one short sentence.".into(),
            skill_ids: vec![],
        })
        .unwrap();
    engine.queue_user_message(&bot.id, body).unwrap();
    engine.pump(&bot.id).await;
    let session = engine.sessions_of(&bot.id).unwrap().remove(0);
    (engine, bot, session, Held { bridge, _herdr: cleanup, _opencode: opencode, _tmp: tmp })
}

async fn screen_of(engine: &Engine, session: &Session) -> String {
    match engine.terminal(&session.id).await {
        Ok(Some(screen)) => screen,
        other => format!("(no screen: {other:?})"),
    }
}

#[tokio::test]
#[ignore = "needs herdr and opencode; run with --ignored"]
async fn opencode_bot_stops_its_turn_when_cancelled() {
    let (engine, _bot, session, _held) = opencode_bot_with(
        "Run this shell command in the foreground and wait for it to finish: sleep 60. Then reply with sbot: slept.",
    )
    .await;
    let mut shown = false;
    for _ in 0..240 {
        if screen_of(&engine, &session).await.contains("sleep 60") {
            shown = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(shown, "the bot never ran sleep 60:\n{}", screen_of(&engine, &session).await);
    engine.stop_turn(&session.id).await.unwrap();
    let stopped = std::time::Instant::now();
    wait_until_not_busy(&engine, &session).await;
    println!("the turn ended {:?} after the stop", stopped.elapsed());
    assert!(
        stopped.elapsed() < Duration::from_secs(20),
        "the turn ran on after the stop:\n{}",
        screen_of(&engine, &session).await
    );
    engine.shutdown().await;
}

async fn answer_without_a_dialog(
    engine: &Engine,
    bot: &Bot,
    session: &Session,
    seen: &std::collections::HashSet<String>,
) -> Message {
    for _ in 0..480 {
        let status = engine.find_session_view(&session.id).await.unwrap().status;
        assert_ne!(
            status,
            BotStatus::NeedsAttention,
            "the bot waits on a dialog:\n{}",
            screen_of(engine, session).await
        );
        let thread = engine.user_thread(&bot.id).unwrap();
        if let Some(answer) =
            thread.iter().find(|m| m.sender == bot.id && m.kind == MessageKind::Text && !seen.contains(&m.id))
        {
            return answer.clone();
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("no answer within 240 s:\n{}", screen_of(engine, session).await);
}

#[tokio::test]
#[ignore = "needs herdr and opencode; run with --ignored"]
async fn opencode_bot_never_waits_on_a_dialog() {
    let (engine, bot, session, _held) = opencode_bot_with(
        "Use your web search tool to find the latest stable version of the Rust programming language. Do not ask \
         me anything. Then reply with sbot with just the version number.",
    )
    .await;
    let searched = answer_without_a_dialog(&engine, &bot, &session, &Default::default()).await;
    println!("web search answer: {}", searched.body);
    assert!(searched.body.chars().any(|c| c.is_ascii_digit()), "no version in the answer: {}", searched.body);
    let screen = screen_of(&engine, &session).await;
    assert!(
        screen.lines().any(|line| line.contains("Explored: ") && line.contains(" search")),
        "the bot never searched the web:\n{screen}"
    );

    let seen = engine.user_thread(&bot.id).unwrap().into_iter().map(|m| m.id).collect();
    engine
        .queue_user_message(
            &bot.id,
            "Now use your question tool to ask me to pick red or blue. If you have no question tool, reply with sbot: \
             no question tool.",
        )
        .unwrap();
    engine.pump(&bot.id).await;
    let asked = answer_without_a_dialog(&engine, &bot, &session, &seen).await;
    println!("question answer: {}", asked.body);
    engine.shutdown().await;
}
