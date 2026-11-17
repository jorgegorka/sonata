#![allow(dead_code)]
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bridge_proto::{BridgeCommand, BridgeRequest};
use sonata_core::engine::{Engine, EngineConfig, UiEvent};
use sonata_core::model::*;
use sonata_core::paths::{resolve_bots_root, AppPaths};
use sonata_core::runtime::fake::FakeRuntime;
use sonata_core::runtime::{AgentState, RuntimeEvent};
use sonata_core::store::Store;

pub struct Harness {
    pub engine: Engine,
    pub rt: Arc<FakeRuntime>,
    pub paths: AppPaths,
    _tmp: tempfile::TempDir,
}

pub fn harness() -> Harness {
    build_harness(|_| Store::open_in_memory().unwrap(), None)
}

pub fn harness_with_grace(grace: Duration) -> Harness {
    build_harness(|_| Store::open_in_memory().unwrap(), Some(grace))
}

pub fn harness_after(previous_run: impl FnOnce(&Store)) -> Harness {
    build_harness(
        |paths| {
            previous_run(&Store::open(&paths.db_path()).unwrap());
            Store::open(&paths.db_path()).unwrap()
        },
        None,
    )
}

pub fn relaunch(previous: &Harness) -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let rt = FakeRuntime::new();
    let store = Store::open(&previous.paths.db_path()).unwrap();
    let bots_root = resolve_bots_root(None, store.get_setting("bots_root").unwrap(), previous.paths.bots_root.clone());
    let paths = AppPaths { bots_root, ..previous.paths.clone() };
    let engine = Engine::new(
        store,
        rt.clone(),
        EngineConfig {
            paths: paths.clone(),
            sbot_dir: tmp.path().join("bin"),
            fallback_lines: 200,
            turn_start_grace: None,
        },
    );
    Harness { engine, rt, paths, _tmp: tmp }
}

fn build_harness(store: impl FnOnce(&AppPaths) -> Store, turn_start_grace: Option<Duration>) -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
    paths.ensure().unwrap();
    let rt = FakeRuntime::new();
    let engine = Engine::new(
        store(&paths),
        rt.clone(),
        EngineConfig { paths: paths.clone(), sbot_dir: tmp.path().join("bin"), fallback_lines: 200, turn_start_grace },
    );
    Harness { engine, rt, paths, _tmp: tmp }
}

pub fn claude_bot(name: &str) -> NewBot {
    NewBot {
        display_name: name.into(),
        color: "#3b82f6".into(),
        provider: Provider::Claude,
        model: None,
        instructions: "Be helpful.".into(),
        skill_ids: vec![],
    }
}

pub fn reply(pane: &str, body: &str) -> BridgeRequest {
    BridgeRequest {
        v: 1,
        pane_id: pane.into(),
        cli_session_id: None,
        command: BridgeCommand::Reply { body: body.into(), files: vec![] },
    }
}

pub async fn status_of(h: &Harness, bot_id: &str) -> BotStatus {
    h.engine.list_bots().await.unwrap().into_iter().find(|v| v.bot.id == bot_id).unwrap().status
}

pub async fn settle() {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}

pub async fn saved_session(h: &Harness, bot_id: &str) -> Option<String> {
    h.engine.sessions_of(bot_id).unwrap().into_iter().next().and_then(|s| s.provider_session_id)
}

pub fn latest_session(h: &Harness, bot_id: &str) -> String {
    h.engine.sessions_of(bot_id).unwrap().into_iter().next().expect("the bot has a session").id
}

pub fn new_session(h: &Harness, bot_id: &str) -> Session {
    Store::open(&h.paths.db_path()).unwrap().create_session(bot_id, None, None).unwrap()
}

pub fn agent(session: &Session) -> String {
    format!("s{}", session.id.to_lowercase())
}

pub fn pane(h: &Harness, session: &Session) -> String {
    h.rt.state().pane_of(&agent(session)).expect("the session's CLI started")
}

pub async fn bot_with_delivered(h: &Harness, body: &str) -> (Bot, Message) {
    let bot = h.engine.create_bot(claude_bot("Researcher")).unwrap();
    let msg = h.engine.queue_user_message(&bot.id, body).unwrap();
    h.engine.pump(&bot.id).await;
    (bot, msg)
}

pub fn send(pane: &str, to: &str, body: &str) -> BridgeRequest {
    BridgeRequest {
        v: 1,
        pane_id: pane.into(),
        cli_session_id: None,
        command: BridgeCommand::Send { to: to.into(), body: body.into(), files: vec![] },
    }
}

pub fn send_files(pane: &str, to: &str, body: &str, files: &[&std::path::Path]) -> BridgeRequest {
    let files = files.iter().map(|f| f.to_string_lossy().into_owned()).collect();
    BridgeRequest {
        v: 1,
        pane_id: pane.into(),
        cli_session_id: None,
        command: BridgeCommand::Send { to: to.into(), body: body.into(), files },
    }
}

pub fn with_cli_id(mut req: BridgeRequest, id: &str) -> BridgeRequest {
    req.cli_session_id = Some(id.into());
    req
}

pub fn bots_request(pane: &str) -> BridgeRequest {
    BridgeRequest { v: 1, pane_id: pane.into(), cli_session_id: None, command: BridgeCommand::Bots }
}

pub fn request(pane: &str, command: BridgeCommand) -> BridgeRequest {
    BridgeRequest { v: 1, pane_id: pane.into(), cli_session_id: None, command }
}

pub fn ask(pane: &str, body: &str) -> BridgeRequest {
    request(pane, BridgeCommand::Ask { body: body.into(), files: vec![], options: vec![] })
}

pub fn ask_with(pane: &str, body: &str, options: &[&str]) -> BridgeRequest {
    request(
        pane,
        BridgeCommand::Ask {
            body: body.into(),
            files: vec![],
            options: options.iter().map(|o| o.to_string()).collect(),
        },
    )
}

pub fn result(pane: &str, body: &str) -> BridgeRequest {
    request(pane, BridgeCommand::Result { body: body.into(), files: vec![], outcome: None })
}

pub fn result_with(pane: &str, body: &str, outcome: &str) -> BridgeRequest {
    request(pane, BridgeCommand::Result { body: body.into(), files: vec![], outcome: Some(outcome.into()) })
}

pub async fn finish_turn(h: &Harness, pane: &str) {
    for state in [AgentState::Working, AgentState::Idle] {
        h.engine.on_runtime_event(RuntimeEvent::State { pane_id: pane.into(), state }).await;
    }
    settle().await;
}

pub async fn team(h: &Harness) -> (Bot, Bot, Message) {
    let writer = h
        .engine
        .create_bot(NewBot { instructions: "Writes drafts.\nKeeps them short.".into(), ..claude_bot("Writer") })
        .unwrap();
    let (researcher, hello) = bot_with_delivered(h, "hello").await;
    (researcher, writer, hello)
}

pub fn skill(name: &str, body: &str) -> NewSkill {
    NewSkill { name: name.into(), description: format!("Use for {name}."), body: body.into() }
}

pub fn skill_folder(parent: &std::path::Path, name: &str) -> std::path::PathBuf {
    let folder = parent.join(name);
    std::fs::create_dir_all(folder.join("scripts")).unwrap();
    std::fs::write(folder.join("SKILL.md"), format!("---\nname: {name}\ndescription: Does {name}.\n---\n\n# {name}\n"))
        .unwrap();
    std::fs::write(folder.join("scripts/run.sh"), "echo hi\n").unwrap();
    folder
}

pub fn events(rx: &mut tokio::sync::broadcast::Receiver<UiEvent>) -> Vec<UiEvent> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

pub fn stored(h: &Harness, message: &Message) -> MessageStatus {
    h.engine.thread_messages(&message.thread_id).unwrap().into_iter().find(|m| m.id == message.id).unwrap().status
}

pub fn opened(h: &Harness, bot: &Bot, asker: &Session) -> Session {
    let sessions = h.engine.sessions_of(&bot.id).unwrap();
    sessions.into_iter().find(|s| s.opened_by.as_deref() == Some(asker.id.as_str())).expect("a session it opened")
}

pub fn sorted(mut items: Vec<String>) -> Vec<String> {
    items.sort();
    items
}

pub async fn chain(h: &Harness) -> ([Bot; 3], [Session; 3], [String; 3]) {
    let planner = h.engine.create_bot(claude_bot("Planner")).unwrap();
    let reviewer = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let qa = h.engine.create_bot(claude_bot("QA")).unwrap();
    let (p1, _) = h.engine.start_session_with_message(&planner.id, "Plan the release.", &[]).unwrap();
    h.engine.pump_session(&p1.id).await;
    let p_pane = pane(h, &p1);
    assert!(h.engine.handle_bridge(send(&p_pane, "@reviewer", "Review the plan.")).await.ok);
    settle().await;
    let r1 = opened(h, &reviewer, &p1);
    let r_pane = pane(h, &r1);
    assert!(h.engine.handle_bridge(send(&r_pane, "@qa", "Test the plan.")).await.ok);
    settle().await;
    let q1 = opened(h, &qa, &r1);
    let q_pane = pane(h, &q1);
    ([planner, reviewer, qa], [p1, r1, q1], [p_pane, r_pane, q_pane])
}

pub const CLAUDE_SHELL_RUNNING: &str = "\
✻ Cogitated for 23s · done 11:37 AM · 1 shell still running
────────────────────────────────────────
❯
────────────────────────────────────────
  ⏵⏵ bypass permissions on · 1 shell · ← 1 agent · ↓ to manage
";

pub const CLAUDE_NOTHING_RUNNING: &str = "\
✻ Cooked for 2s · done 11:40 AM
────────────────────────────────────────
❯
────────────────────────────────────────
  ⏵⏵ bypass permissions on (shift+tab to cycle) · ← 1 agent
";

#[derive(Clone, Default)]
pub struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

pub fn capture() -> (Captured, tracing::subscriber::DefaultGuard) {
    let log = Captured::default();
    let subscriber = tracing_subscriber::fmt().with_ansi(false).with_writer(log.clone()).finish();
    (log.clone(), tracing::subscriber::set_default(subscriber))
}

pub fn setup_team(h: &Harness, project: Option<&std::path::Path>) {
    h.engine.finish_setup(project).unwrap();
    let find = |slug: &str| h.engine.list_skills().unwrap().into_iter().find(|v| v.skill.slug == slug).map(|v| v.skill);
    let shared =
        find("sonata").unwrap_or_else(|| h.engine.create_skill(skill("sonata", "Shared example guidance.")).unwrap());
    std::fs::write(h.paths.skills_dir().join("sonata/notes.txt"), "Synthetic sample notes.\n").unwrap();
    let mut ids = std::collections::HashMap::new();
    for (handle, name) in [
        ("research", "Research"),
        ("plan", "Plan"),
        ("feature", "Feature"),
        ("debug", "Debug"),
        ("query", "Query"),
        ("interview", "Interview"),
        ("review", "Review"),
        ("testing", "Testing"),
    ] {
        let slug = format!("sonata-{}", if handle == "testing" { "test" } else { handle });
        let role = find(&slug)
            .unwrap_or_else(|| h.engine.create_skill(skill(&slug, &format!("Example {handle} guidance."))).unwrap());
        let exists = h.paths.bot_dir(handle).exists();
        if !exists {
            let instructions = format!(
                "Example {handle} role.\n\n{}\n\n{}.",
                sonata_core::bundled::checkout_paragraph(project),
                sonata_core::bundled::NEW_ASK
            );
            let bot = h
                .engine
                .create_bot(NewBot { instructions, skill_ids: vec![shared.id.clone(), role.id], ..claude_bot(name) })
                .unwrap();
            ids.insert(handle.to_string(), bot.id);
        }
    }
    if !ids.is_empty() && !h.engine.list_workflows().unwrap().iter().any(|w| w.name == "Example review workflow") {
        let definition =
            sonata_core::workflow::text::parse_text(TICKET_WORKFLOW, |handle| ids.get(handle).cloned(), None).unwrap();
        h.engine.create_workflow(definition).unwrap();
    }
}

pub const TICKET_WORKFLOW: &str = "Workflow: Example review workflow\nWhen: A request to implement and review.\n\n\
## Plan\nBot: @plan\n-> Implement\n\nPlan the request.\n\n\
## Implement\nBot: @feature\n-> Review\n\nImplement the approved plan.\n\n\
## Review\nBot: @review\napproved -> Test\nchanges requested -> Implement\n\nReview the result.\n\n\
## Test\nBot: @testing\npassed -> Open MR\nfailed -> Implement\n\nCheck the acceptance criteria.\n\n\
## Open MR\nBot: @feature\n-> Done\n\nReport the result.\n";
