use std::path::Path;
use std::process::Stdio;

use sonata_core::engine::{Engine, EngineConfig};
use sonata_core::model::{NewBot, Provider};
use sonata_core::paths::AppPaths;
use sonata_core::runtime::fake::FakeRuntime;
use sonata_core::store::Store;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

struct Setup {
    engine: Engine,
    bot_id: String,
    socket: std::path::PathBuf,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Setup {
    setup_with(Provider::Claude).await
}

#[tokio::test]
async fn orchestration_cli_starts_reads_and_cancels_real_persisted_tasks() {
    let s = setup().await;
    s.engine.finish_setup(None).unwrap();
    let master = s.engine.master_id().unwrap().unwrap();
    let (session, _) = s.engine.start_session_with_message(&master, "Coordinate two tasks", &[]).unwrap();
    s.engine.pump_session(&session.id).await;
    let body = r#"{"title":"Process","goal":"Two tasks","tasks":[{"key":"a","title":"First","instructions":"Do it","acceptance_criteria":["Checked"],"executor":{"kind":"bot","handle":"writer"}},{"key":"b","title":"Second","instructions":"Use first result","acceptance_criteria":["Checked"],"executor":{"kind":"bot","handle":"researcher"},"depends_on":["a"]}]}"#;
    let (ok, stdout, stderr) =
        sbot(&["orchestration", "start", "--key", "initial"], Some("w1:p2"), &s.socket, Some(body)).await;
    assert!(ok, "{stderr}");
    let run: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let id = run["id"].as_str().unwrap();
    assert_eq!(run["tasks"][0]["state"], "running");
    assert_eq!(run["tasks"][1]["state"], "waiting");
    let (ok, output, error) = sbot(&["orchestration", "show", id], Some("w1:p2"), &s.socket, None).await;
    assert!(ok, "{error}");
    assert_eq!(serde_json::from_str::<serde_json::Value>(&output).unwrap()["id"], id);
    let revision = run["revision"].to_string();
    let (ok, _, _) =
        sbot(&["orchestration", "finish", id, "--revision", &revision], Some("w1:p2"), &s.socket, Some("Too soon"))
            .await;
    assert!(ok, "early completion is saved until children finish");
    assert_eq!(s.engine.orchestration_view(id).unwrap().run.status, sonata_core::orchestration::Status::Active);
    let (ok, _, error) = sbot(&["orchestration", "cancel", id], Some("w1:p2"), &s.socket, None).await;
    assert!(ok, "{error}");
    assert_eq!(s.engine.orchestration_view(id).unwrap().run.status, sonata_core::orchestration::Status::Cancelled);
}

async fn setup_with(provider: Provider) -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
    paths.ensure().unwrap();
    let engine = Engine::new(
        Store::open_in_memory().unwrap(),
        FakeRuntime::new(),
        EngineConfig { paths: paths.clone(), sbot_dir: tmp.path().into(), fallback_lines: 200, turn_start_grace: None },
    );
    let bot = engine
        .create_bot(NewBot {
            display_name: "Researcher".into(),
            color: "#3b82f6".into(),
            provider,
            model: None,
            instructions: String::new(),
            skill_ids: vec![],
        })
        .unwrap();
    engine
        .create_bot(NewBot {
            display_name: "Writer".into(),
            color: "#16a34a".into(),
            provider: Provider::Claude,
            model: None,
            instructions: "Writes drafts.".into(),
            skill_ids: vec![],
        })
        .unwrap();
    engine.queue_user_message(&bot.id, "hello").unwrap();
    engine.pump(&bot.id).await;
    let socket = paths.bridge_socket();
    sonata_core::bridge::serve(&socket, engine.clone()).unwrap();
    Setup { engine, bot_id: bot.id, socket, _tmp: tmp }
}

async fn sbot(args: &[&str], pane: Option<&str>, socket: &Path, stdin: Option<&str>) -> (bool, String, String) {
    sbot_in(None, &[], args, pane, socket, stdin).await
}

async fn sbot_in(
    dir: Option<&Path>,
    env: &[(&str, &str)],
    args: &[&str],
    pane: Option<&str>,
    socket: &Path,
    stdin: Option<&str>,
) -> (bool, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sbot"));
    cmd.args(args)
        .env_remove("HERDR_PANE_ID")
        .env_remove("CODEX_THREAD_ID")
        .env("SBOT_SOCKET", socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    if let Some(p) = pane {
        cmd.env("HERDR_PANE_ID", p);
    }
    cmd.envs(env.iter().copied());
    let mut child = cmd.spawn().unwrap();
    let mut input = child.stdin.take().unwrap();
    if let Some(text) = stdin {
        input.write_all(text.as_bytes()).await.unwrap();
    }
    drop(input);
    let out = child.wait_with_output().await.unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stdout).into(), String::from_utf8_lossy(&out.stderr).into())
}

#[tokio::test]
async fn reply_reaches_the_app_and_prints_the_message_id() {
    let s = setup().await;
    let (ok, stdout, stderr) = sbot(&["reply"], Some("w1:p1"), &s.socket, Some("Hello from **sbot**\n")).await;
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.starts_with("sent to user (message "), "{stdout}");
    let thread = s.engine.user_thread(&s.bot_id).unwrap();
    assert!(thread.iter().any(|m| m.body == "Hello from **sbot**"));
}

#[tokio::test]
async fn ask_prints_that_the_answer_comes_next() {
    let s = setup().await;
    let (ok, stdout, stderr) = sbot(&["ask"], Some("w1:p1"), &s.socket, Some("Which date?\n")).await;
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.starts_with("asked the user (message "), "{stdout}");
    let thread = s.engine.user_thread(&s.bot_id).unwrap();
    assert!(thread.iter().any(|m| m.body == "Which date?"));
}

#[tokio::test]
async fn ask_passes_its_options_in_order() {
    let s = setup().await;
    let args = ["ask", "--option", "Approve", "--option", "Change the plan"];
    let (ok, stdout, stderr) = sbot(&args, Some("w1:p1"), &s.socket, Some("Plan?\n")).await;
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.starts_with("asked the user (message "), "{stdout}");
    let session = s.engine.sessions_of(&s.bot_id).unwrap().remove(0);
    assert_eq!(session.waiting_options, vec!["Approve".to_string(), "Change the plan".to_string()]);
}

#[tokio::test]
async fn result_finishes_a_standalone_task() {
    let s = setup().await;
    let (ok, _, stderr) = sbot(&["result"], Some("w1:p1"), &s.socket, Some("Done.\n")).await;
    assert!(ok, "{stderr}");
    assert_eq!(s.engine.list_work_items().await.unwrap()[0].state, "finished");
}

#[tokio::test]
async fn result_passes_its_outcome() {
    let s = setup().await;
    let args = ["result", "--outcome", "approved"];
    let (ok, _, stderr) = sbot(&args, Some("w1:p1"), &s.socket, Some("Done.\n")).await;
    assert!(!ok);
    assert!(stderr.contains("--outcome is only for a workflow step"), "{stderr}");
}

#[tokio::test]
async fn send_to_a_teammate_prints_where_it_was_queued() {
    let s = setup().await;
    let (ok, stdout, stderr) = sbot(&["send", "@writer"], Some("w1:p1"), &s.socket, Some("Draft, please.\n")).await;
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.starts_with("queued for @writer (message "), "{stdout}");
}

#[tokio::test]
async fn bots_lists_the_teammates() {
    let s = setup().await;
    let (ok, stdout, stderr) = sbot(&["bots"], Some("w1:p1"), &s.socket, None).await;
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout.trim_end(), "@writer (Writer) · claude · offline — Writes drafts.");
}

#[tokio::test]
async fn relative_file_paths_are_sent_as_absolute_paths() {
    let s = setup().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.md"), "n").unwrap();
    let (ok, stdout, stderr) = sbot_in(
        Some(dir.path()),
        &[],
        &["reply", "--file", "notes.md"],
        Some("w1:p1"),
        &s.socket,
        Some("See attached."),
    )
    .await;
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.starts_with("sent to user (message "), "{stdout}");
    let sent = s.engine.user_thread(&s.bot_id).unwrap().into_iter().find(|m| m.body == "See attached.").unwrap();
    let expected = dir.path().canonicalize().unwrap().join("notes.md");
    assert_eq!(sent.attachments[0].path, expected.to_string_lossy());
}

#[tokio::test]
async fn an_unknown_recipient_is_explained() {
    let s = setup().await;
    let (ok, _, stderr) = sbot(&["send", "@nobody"], Some("w1:p1"), &s.socket, Some("hi")).await;
    assert!(!ok);
    assert!(stderr.contains("No bot named @nobody"), "{stderr}");
}

#[tokio::test]
async fn app_errors_exit_nonzero_with_the_message() {
    let s = setup().await;
    let (ok, _, stderr) = sbot(&["reply"], Some("w9:p9"), &s.socket, Some("hi")).await;
    assert!(!ok);
    assert!(stderr.contains("not a Sonata bot"), "{stderr}");
}

#[tokio::test]
async fn empty_stdin_is_rejected_locally() {
    let s = setup().await;
    let (ok, _, stderr) = sbot(&["reply"], Some("w1:p1"), &s.socket, Some("  \n")).await;
    assert!(!ok);
    assert!(stderr.contains("the message is empty"), "{stderr}");
}

#[tokio::test]
async fn missing_socket_says_the_app_is_not_running() {
    let tmp = tempfile::tempdir().unwrap();
    let (ok, _, stderr) = sbot(&["reply"], Some("w1:p1"), &tmp.path().join("none.sock"), Some("hi")).await;
    assert!(!ok);
    assert!(stderr.contains("Sonata is not running"), "{stderr}");
}

#[tokio::test]
async fn missing_pane_id_is_reported() {
    let s = setup().await;
    let (ok, _, stderr) = sbot(&["reply"], None, &s.socket, Some("hi")).await;
    assert!(!ok);
    assert!(stderr.contains("not running inside a Sonata bot"), "{stderr}");
}

fn saved_id(s: &Setup) -> Option<String> {
    s.engine.sessions_of(&s.bot_id).unwrap().into_iter().next().and_then(|session| session.provider_session_id)
}

#[tokio::test]
async fn the_codex_thread_id_reaches_the_app() {
    let s = setup_with(Provider::Codex).await;
    let id = "01a143db-8973-7a12-837a-0d8dc3e76759";
    let (ok, _, stderr) =
        sbot_in(None, &[("CODEX_THREAD_ID", id)], &["reply"], Some("w1:p1"), &s.socket, Some("done")).await;
    assert!(ok, "stderr: {stderr}");
    assert_eq!(saved_id(&s).as_deref(), Some(id));
}

#[tokio::test]
async fn a_blank_codex_thread_id_is_not_sent() {
    let s = setup_with(Provider::Codex).await;
    let (ok, _, stderr) =
        sbot_in(None, &[("CODEX_THREAD_ID", "  ")], &["reply"], Some("w1:p1"), &s.socket, Some("done")).await;
    assert!(ok, "stderr: {stderr}");
    assert_eq!(saved_id(&s), None);
}

#[tokio::test]
async fn bot_show_prints_the_bot() {
    let s = setup().await;
    s.engine.finish_setup(Some(s._tmp.path())).unwrap();
    let master = s.engine.master_bot().unwrap().expect("Master exists");
    s.engine.queue_user_message(&master.id, "hi").unwrap();
    s.engine.pump(&master.id).await;
    let (ok, stdout, stderr) = sbot(&["bot", "show", "@master"], Some("w1:p2"), &s.socket, None).await;
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.starts_with("@master (Master) · claude ·"), "{stdout}");
}

async fn setup_with_master() -> Setup {
    let s = setup().await;
    s.engine.finish_setup(Some(s._tmp.path())).unwrap();
    let master = s.engine.master_bot().unwrap().expect("Master exists");
    s.engine.queue_user_message(&master.id, "hi").unwrap();
    s.engine.pump(&master.id).await;
    s
}

#[tokio::test]
async fn skill_copy_prints_the_draft_path() {
    let s = setup_with_master().await;
    let (ok, stdout, stderr) = sbot(&["skill", "copy", "sonata-master"], Some("w1:p2"), &s.socket, None).await;
    assert!(ok, "stderr: {stderr}");
    let draft = s._tmp.path().join("bots/master/skill-drafts/session-1/sonata-master");
    assert_eq!(stdout.trim_end(), format!("draft at {}", draft.display()));
    assert!(draft.join("SKILL.md").is_file());
}

#[tokio::test]
async fn bot_create_prints_the_handle() {
    let s = setup_with_master().await;
    let args = ["bot", "create", "--name", "SEO", "--color", "#0ea5e9"];
    let (ok, stdout, stderr) = sbot(&args, Some("w1:p2"), &s.socket, Some("Does SEO.")).await;
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout.trim_end(), "created @seo");
}

#[tokio::test]
async fn skills_end_with_the_drafts_folder() {
    let s = setup_with_master().await;
    let (ok, stdout, stderr) = sbot(&["skills"], Some("w1:p2"), &s.socket, None).await;
    assert!(ok, "stderr: {stderr}");
    let drafts = s._tmp.path().join("bots/master/skill-drafts/session-1");
    let last = stdout.trim_end().lines().last().unwrap().to_string();
    assert_eq!(last, format!("Drafts for this conversation go in {}/<name>/.", drafts.display()));
}

#[tokio::test]
async fn recreate_says_it_makes_again_what_the_user_deleted() {
    let nowhere = Path::new("/nonexistent/bridge.sock");
    let (ok, stdout, stderr) = sbot(&["bot", "create", "--help"], None, nowhere, None).await;
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.contains("Make again the bot this conversation created, after the user deleted it"), "{stdout}");
    let (ok, stdout, stderr) = sbot(&["skill", "add", "--help"], None, nowhere, None).await;
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.contains("Add again the skill this conversation added, after the user deleted it"), "{stdout}");
}
