use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use herdr_client::{
    delete_session, session_socket_path, AgentInfo, AgentStatus, Event, HerdrClient, HerdrError, HerdrServer,
    ReadFormat, ReadSource, Subscription, PINNED_PROTOCOL,
};
use tokio::sync::{broadcast, Mutex};

use super::*;

const WORKSPACE_LABEL: &str = "Sonata";
const READY_MARKER: &str = "__SONATA_READY__";
const START_TIMEOUT_MS: u64 = 60_000;
const SHELL_READY_TIMEOUT_MS: u64 = 30_000;
const PANE_BUSY_RETRIES: u32 = 10;
const PANE_BUSY_PAUSE: Duration = Duration::from_millis(200);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);

pub type EnvSource = Pin<Box<dyn Future<Output = Vec<(String, String)>> + Send + 'static>>;

pub struct HerdrRuntime {
    bin: PathBuf,
    session: String,
    env: tokio::sync::OnceCell<Vec<(String, String)>>,
    env_source: Mutex<Option<EnvSource>>,
    server: Mutex<Option<(HerdrServer, String)>>,
    closed: AtomicBool,
    tx: broadcast::Sender<RuntimeEvent>,
}

fn map_status(status: AgentStatus) -> AgentState {
    match status {
        AgentStatus::Idle | AgentStatus::Done => AgentState::Idle,
        AgentStatus::Working => AgentState::Working,
        AgentStatus::Blocked => AgentState::Blocked,
        AgentStatus::Unknown => AgentState::Unknown,
    }
}

fn other(e: HerdrError) -> RuntimeError {
    RuntimeError::Other(e.to_string())
}

fn agent_error(e: HerdrError) -> RuntimeError {
    match e.code() {
        Some("agent_not_found" | "pane_not_found") => RuntimeError::Gone,
        _ => other(e),
    }
}

fn inspection(info: AgentInfo) -> Result<Inspection, RuntimeError> {
    if info.agent.is_none() {
        return Err(RuntimeError::Gone);
    }
    Ok(Inspection { state: map_status(info.agent_status), session_id: info.session_id().map(str::to_string) })
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

const SCREEN_SHOWS: &str = "; the screen shows:\n";

fn startup_blocked(reason: &str, screen: &str) -> HerdrError {
    HerdrError::Protocol(format!("{reason}{SCREEN_SHOWS}{screen}"))
}

const SHELL_GUARD_HOLD_SECS: u32 = 5;

pub(crate) fn shell_setup_line(path_prefix: &str) -> String {
    let guard = format!(
        "[ -n \"$ZSH_VERSION$BASH_VERSION\" ] && eval '__sonata_n=0; __sonata_guard() {{ \
         __sonata_n=$((__sonata_n+1)); if [ \"$__sonata_n\" -ge 2 ]; then \
         exec /bin/sleep {SHELL_GUARD_HOLD_SECS} || exit; fi; }}; \
         if [ -n \"$ZSH_VERSION\" ]; then precmd_functions=(__sonata_guard $precmd_functions); \
         else PROMPT_COMMAND=\"__sonata_guard${{PROMPT_COMMAND:+;$PROMPT_COMMAND}}\"; fi'"
    );
    format!(
        " unset HISTFILE; export PATH={}:\"$PATH\"; {guard} \
         || [ -z \"$ZSH_VERSION$BASH_VERSION\" ] && printf '__SONATA_%s__\\n' READY",
        shell_quote(path_prefix)
    )
}

async fn accept_folder_trust(c: &HerdrClient, agent: &str, option: &str) -> Result<(), HerdrError> {
    for _ in 0..3 {
        let screen = c.agent_read(agent, ReadSource::Visible, 60).await?;
        if !screen.contains(option) {
            return Err(startup_blocked("agent blocked during startup", &screen));
        }
        if screen.lines().any(|l| (l.contains('❯') || l.contains('›')) && l.contains(option)) {
            c.agent_send_keys(agent, &["enter"]).await?;
            for _ in 0..60 {
                tokio::time::sleep(Duration::from_millis(500)).await;
                match c.agent_get(agent).await?.agent_status {
                    AgentStatus::Idle | AgentStatus::Done => return Ok(()),
                    AgentStatus::Blocked => {
                        let screen = c.agent_read(agent, ReadSource::Visible, 60).await?;
                        if !screen.contains(option) {
                            return Err(startup_blocked("agent blocked after accepting folder trust", &screen));
                        }
                    }
                    AgentStatus::Working | AgentStatus::Unknown => {}
                }
            }
            let screen = c
                .agent_read(agent, ReadSource::Visible, 60)
                .await
                .unwrap_or_else(|e| format!("(screen unavailable: {e})"));
            return Err(startup_blocked("agent not ready 30 s after accepting folder trust", &screen));
        }
        c.agent_send_keys(agent, &["down"]).await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let screen =
        c.agent_read(agent, ReadSource::Visible, 60).await.unwrap_or_else(|e| format!("(screen unavailable: {e})"));
    Err(startup_blocked("could not select the folder-trust option", &screen))
}

async fn refuse_sign_in(c: &HerdrClient, agent: &str, text: &str) -> Result<(), HerdrError> {
    let screen = c.agent_read(agent, ReadSource::Visible, 60).await?;
    let is_menu_line = |line: &str| {
        let line = line.trim();
        let line = line.strip_prefix(['>', '›', '❯']).map_or(line, str::trim_start);
        line.starts_with(text)
    };
    if screen.lines().any(is_menu_line) {
        return Err(startup_blocked("the CLI is not signed in", &screen));
    }
    Ok(())
}

async fn check_signed_in(c: &HerdrClient, spec: &LaunchSpec) -> Result<(), HerdrError> {
    match spec.sign_in {
        Some(text) => refuse_sign_in(c, &spec.agent, text).await,
        None => Ok(()),
    }
}

async fn answer_key_prompt(
    c: &HerdrClient,
    agent: &str,
    question: &str,
    key: &str,
    required: bool,
) -> Result<bool, HerdrError> {
    let screen = c.agent_read(agent, ReadSource::Visible, 60).await?;
    if !screen.contains(question) {
        return if required { Err(startup_blocked("agent blocked during startup", &screen)) } else { Ok(false) };
    }
    c.agent_send_keys(agent, &[key]).await?;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if !c.agent_read(agent, ReadSource::Visible, 60).await?.contains(question) {
            return Ok(true);
        }
    }
    let screen =
        c.agent_read(agent, ReadSource::Visible, 60).await.unwrap_or_else(|e| format!("(screen unavailable: {e})"));
    Err(startup_blocked("the folder-trust question was still showing 30 s after answering it", &screen))
}

async fn answer_blocked(c: &HerdrClient, agent: &str, trust: Option<&TrustPrompt>) -> Result<(), HerdrError> {
    match trust {
        Some(TrustPrompt::Menu { option }) => accept_folder_trust(c, agent, option).await,
        Some(TrustPrompt::Key { question, key }) => answer_key_prompt(c, agent, question, key, true).await.map(|_| ()),
        None => {
            let screen = c.agent_read(agent, ReadSource::Visible, 60).await?;
            Err(startup_blocked("agent blocked during startup", &screen))
        }
    }
}

async fn wait_until_ready(
    c: &HerdrClient,
    agent: &str,
    trust: Option<&TrustPrompt>,
    last_screen: &mut Option<String>,
) -> Result<AgentInfo, HerdrError> {
    for _ in 0..60 {
        let info = c.agent_get(agent).await?;
        match info.agent_status {
            AgentStatus::Idle | AgentStatus::Done => return Ok(info),
            AgentStatus::Blocked => {
                answer_blocked(c, agent, trust).await?;
                return c.agent_get(agent).await;
            }
            AgentStatus::Working | AgentStatus::Unknown => {
                if let Ok(screen) = c.agent_read(agent, ReadSource::Visible, 60).await {
                    *last_screen = Some(screen);
                }
                tokio::time::sleep(Duration::from_millis(500)).await
            }
        }
    }
    let screen =
        c.agent_read(agent, ReadSource::Visible, 60).await.unwrap_or_else(|e| format!("(screen unavailable: {e})"));
    Err(startup_blocked("agent not ready 30 s after start", &screen))
}

struct ReadyTiming {
    recheck: Duration,
    poll: Duration,
    settle: Duration,
}

const READY_TIMING: ReadyTiming = ReadyTiming {
    recheck: Duration::from_millis(1500),
    poll: Duration::from_millis(500),
    settle: Duration::from_secs(30),
};

fn ready_for_input(info: &AgentInfo) -> bool {
    matches!(info.agent_status, AgentStatus::Idle | AgentStatus::Done)
        && info.agent.is_some()
        && info.interactive_ready != Some(false)
}

enum Seen {
    Gone(String),
    Busy(AgentInfo),
    Ready(AgentInfo),
}

fn seen(got: Result<AgentInfo, HerdrError>) -> Seen {
    match got {
        Err(e) => Seen::Gone(e.to_string()),
        Ok(info) if info.agent.is_none() => Seen::Gone("herdr no longer detects the CLI".into()),
        Ok(info) if ready_for_input(&info) => Seen::Ready(info),
        Ok(info) => Seen::Busy(info),
    }
}

async fn confirm_ready(
    c: &HerdrClient,
    agent: &str,
    first: AgentInfo,
    t: &ReadyTiming,
) -> Result<AgentInfo, HerdrError> {
    let deadline = tokio::time::Instant::now() + t.settle;
    let mut state = seen(Ok(first));
    loop {
        state = match state {
            Seen::Gone(why) => {
                return Err(HerdrError::Protocol(format!("agent exited right after it became ready ({why})")))
            }
            Seen::Ready(_) => {
                tokio::time::sleep(t.recheck).await;
                match seen(c.agent_get(agent).await) {
                    Seen::Ready(info) => return Ok(info),
                    other => other,
                }
            }
            Seen::Busy(info) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(HerdrError::Protocol(format!(
                        "agent not ready within {:?} after start (status {:?}, interactive {:?})",
                        t.settle, info.agent_status, info.interactive_ready
                    )));
                }
                tokio::time::sleep(t.poll).await;
                seen(c.agent_get(agent).await)
            }
        };
    }
}

impl HerdrRuntime {
    pub fn new(bin: PathBuf, session: impl Into<String>) -> Arc<Self> {
        Self::new_with_env(bin, session, Vec::new())
    }

    pub fn new_with_env(bin: PathBuf, session: impl Into<String>, env: Vec<(String, String)>) -> Arc<Self> {
        Self::new_with_env_source(bin, session, Box::pin(async move { env }))
    }

    pub fn new_with_env_source(bin: PathBuf, session: impl Into<String>, source: EnvSource) -> Arc<Self> {
        let (tx, _) = broadcast::channel(256);
        Arc::new(Self {
            bin,
            session: session.into(),
            env: tokio::sync::OnceCell::new(),
            env_source: Mutex::new(Some(source)),
            server: Mutex::new(None),
            closed: AtomicBool::new(false),
            tx,
        })
    }

    async fn env(&self) -> Vec<(String, String)> {
        self.env
            .get_or_init(|| async {
                let mut slot = self.env_source.lock().await;
                let Some(source) = slot.as_mut() else { return Vec::new() };
                let env = source.await;
                *slot = None;
                env
            })
            .await
            .clone()
    }

    async fn client(&self) -> Result<(HerdrClient, String), RuntimeError> {
        self.connect(false).await
    }

    async fn connect(&self, check: bool) -> Result<(HerdrClient, String), RuntimeError> {
        let closed = || RuntimeError::Other("the runtime has shut down".into());
        if self.closed.load(Ordering::SeqCst) {
            return Err(closed());
        }
        let mut guard = self.server.lock().await;
        if self.closed.load(Ordering::SeqCst) {
            return Err(closed());
        }
        if let Some((server, ws)) = guard.as_ref() {
            let probe = server.client().clone().with_timeout(HEALTH_TIMEOUT);
            if !check || probe.ping().await.is_ok() {
                return Ok((server.client().clone(), ws.clone()));
            }
        }
        if guard.is_some() {
            tracing::warn!(session = %self.session, "herdr stopped answering; starting a new session");
        }
        if let Some((dead, _)) = guard.take() {
            let _ = dead.stop().await;
            if self.closed.load(Ordering::SeqCst) {
                return Err(closed());
            }
        }
        let leftover = HerdrClient::new(session_socket_path(&self.session)).with_timeout(HEALTH_TIMEOUT);
        if leftover.ping().await.is_ok() {
            let _ = leftover.server_stop().await;
            for _ in 0..50 {
                if leftover.ping().await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        let _ = delete_session(&self.bin, &self.session).await;
        let env = self.env().await;
        if self.closed.load(Ordering::SeqCst) {
            return Err(closed());
        }
        let server = HerdrServer::start_with_env(&self.bin, &self.session, &env).await.map_err(other)?;
        let client = server.client().clone();
        let pong = client.ping().await.map_err(other)?;
        if pong.protocol != PINNED_PROTOCOL {
            let _ = server.stop().await;
            return Err(RuntimeError::Other(format!(
                "herdr {} speaks protocol {}, Sonata needs {PINNED_PROTOCOL}",
                pong.version, pong.protocol
            )));
        }
        let created =
            client.workspace_create(WORKSPACE_LABEL, &dirs::home_dir().unwrap_or_default()).await.map_err(other)?;
        let ws = created
            .workspace
            .map(|w| w.workspace_id)
            .ok_or_else(|| RuntimeError::Other("workspace.create returned no workspace".into()))?;
        *guard = Some((server, ws.clone()));
        tracing::info!(session = %self.session, version = %pong.version, "herdr session started");
        Ok((client, ws))
    }

    async fn watch(&self, client: &HerdrClient, pane: &str) -> Result<(), RuntimeError> {
        let mut stream =
            client.subscribe(&[Subscription::agent_status(pane), Subscription::pane_exited()]).await.map_err(other)?;
        let tx = self.tx.clone();
        let pane = pane.to_string();
        tokio::spawn(async move {
            loop {
                match stream.next().await {
                    Ok(Some(Event::PaneAgentStatusChanged { pane_id, agent_status })) if pane_id == pane => {
                        let _ = tx.send(RuntimeEvent::State { pane_id, state: map_status(agent_status) });
                    }
                    Ok(Some(Event::PaneExited { pane_id })) if pane_id == pane => break,
                    Ok(Some(_)) => {}
                    Err(HerdrError::Protocol(_)) => {}
                    Ok(None) | Err(_) => break,
                }
            }
            let _ = tx.send(RuntimeEvent::Exited { pane_id: pane });
        });
        Ok(())
    }
}

#[async_trait]
impl BotRuntime for HerdrRuntime {
    async fn health(&self) -> Result<RuntimeHealth, RuntimeError> {
        let (c, _) = self.connect(true).await?;
        let pong = c.ping().await.map_err(other)?;
        Ok(RuntimeHealth { version: pong.version, protocol: pong.protocol })
    }

    async fn start_bot(&self, spec: &LaunchSpec) -> Result<RuntimeBot, RuntimeError> {
        let (c, ws) = self.connect(true).await?;
        let created = c
            .tab_create(&ws, &spec.label, &spec.cwd, &spec.env)
            .await
            .map_err(|e| RuntimeError::StartFailed(e.to_string()))?;
        let pane = created.root_pane.pane_id.clone();
        let tab = created.tab.tab_id.clone();
        let mut last_screen = None;
        let started = async {
            c.pane_send_input(&pane, &shell_setup_line(&spec.path_prefix.to_string_lossy()), &["enter"]).await?;
            c.pane_wait_for_substring(&pane, READY_MARKER, SHELL_READY_TIMEOUT_MS).await?;
            let mut busy = 0;
            let agent_started = loop {
                match c.agent_start(&spec.agent, &spec.kind, &pane, &spec.args, START_TIMEOUT_MS).await {
                    Err(e) if e.code() == Some("agent_pane_busy") && busy < PANE_BUSY_RETRIES => {
                        busy += 1;
                        tokio::time::sleep(PANE_BUSY_PAUSE).await;
                    }
                    other => break other,
                }
            };
            let ready = match agent_started {
                Err(e) if e.code() == Some("agent_not_ready") => {
                    answer_blocked(&c, &spec.agent, spec.trust.as_ref()).await?;
                    c.agent_get(&spec.agent).await
                }
                Ok(_) => wait_until_ready(&c, &spec.agent, spec.trust.as_ref(), &mut last_screen).await,
                Err(e) => Err(e),
            }?;
            let key = match spec.trust {
                Some(TrustPrompt::Key { question, key }) => Some((question, key)),
                _ => None,
            };
            let ready = match key {
                Some((question, key)) if answer_key_prompt(&c, &spec.agent, question, key, false).await? => {
                    c.agent_get(&spec.agent).await?
                }
                _ => ready,
            };
            let info = confirm_ready(&c, &spec.agent, ready, &READY_TIMING).await?;
            let Some((question, key)) = key else {
                check_signed_in(&c, spec).await?;
                return Ok(info);
            };
            if !answer_key_prompt(&c, &spec.agent, question, key, false).await? {
                check_signed_in(&c, spec).await?;
                return Ok(info);
            }
            let after = c.agent_get(&spec.agent).await?;
            let info = confirm_ready(&c, &spec.agent, after, &READY_TIMING).await?;
            check_signed_in(&c, spec).await?;
            Ok::<_, HerdrError>(info)
        }
        .await;
        let info = match started {
            Ok(info) => info,
            Err(e) => {
                let mut message = e.to_string();
                if !message.contains(SCREEN_SHOWS) {
                    let screen = match c.pane_read(&pane, ReadSource::Visible, 60).await {
                        Ok(screen) => screen,
                        Err(e) => last_screen.unwrap_or_else(|| format!("(screen unavailable: {e})")),
                    };
                    message = format!("{message}{SCREEN_SHOWS}{}", screen.trim_end());
                }
                let _ = c.tab_close(&tab).await;
                return Err(RuntimeError::StartFailed(message));
            }
        };
        if let Err(e) = self.watch(&c, &pane).await {
            let _ = c.tab_close(&tab).await;
            return Err(e);
        }
        Ok(RuntimeBot {
            handle: spec.handle.clone(),
            pane_id: pane,
            tab_id: tab,
            session_id: info.session_id().map(str::to_string),
        })
    }

    async fn prompt(&self, bot: &RuntimeBot, text: &str) -> Result<(), RuntimeError> {
        let (c, _) = self.client().await?;
        c.agent_prompt(&bot.pane_id, text).await.map_err(|e| match e.code() {
            Some("agent_blocked") => RuntimeError::Blocked,
            Some("agent_prompt_stalled") => RuntimeError::Stalled,
            _ => other(e),
        })
    }

    async fn inspect(&self, bot: &RuntimeBot) -> Result<Inspection, RuntimeError> {
        let (c, _) = self.client().await?;
        inspection(c.agent_get(&bot.pane_id).await.map_err(agent_error)?)
    }

    async fn read_output(&self, bot: &RuntimeBot, lines: u32) -> Result<String, RuntimeError> {
        let (c, _) = self.client().await?;
        let text = c.agent_read(&bot.pane_id, ReadSource::RecentUnwrapped, lines).await.map_err(other)?;
        if !text.trim().is_empty() {
            return Ok(text);
        }
        c.agent_read(&bot.pane_id, ReadSource::Visible, lines).await.map_err(other)
    }

    async fn snapshot(&self, bot: &RuntimeBot, lines: u32) -> Result<String, RuntimeError> {
        let (c, _) = self.client().await?;
        c.agent_read(&bot.pane_id, ReadSource::Visible, lines).await.map_err(agent_error)
    }

    async fn terminal(&self, bot: &RuntimeBot, lines: u32) -> Result<String, RuntimeError> {
        let (c, _) = self.client().await?;
        c.agent_read_as(&bot.pane_id, ReadSource::Visible, ReadFormat::Ansi, lines).await.map_err(agent_error)
    }

    async fn send_keys(&self, bot: &RuntimeBot, keys: &[&str]) -> Result<(), RuntimeError> {
        let (c, _) = self.client().await?;
        c.agent_send_keys(&bot.pane_id, keys).await.map_err(other)
    }

    async fn stop_bot(&self, bot: &RuntimeBot) -> Result<(), RuntimeError> {
        let (c, _) = self.client().await?;
        c.tab_close(&bot.tab_id).await.map_err(other)
    }

    async fn shutdown(&self) {
        tracing::info!(session = %self.session, "stopping the herdr session");
        self.closed.store(true, Ordering::SeqCst);
        if let Some((server, _)) = self.server.lock().await.take() {
            let _ = server.stop().await;
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_session() -> String {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        format!("sonata-test-{}-{nanos}-{n}", std::process::id())
    }

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

    #[tokio::test]
    async fn a_dead_session_is_replaced_on_the_next_start() {
        let Some(bin) = std::env::var_os("SONATA_HERDR_BIN").map(PathBuf::from).filter(|p| p.is_file()) else {
            eprintln!("SONATA_HERDR_BIN is not set; skipping live test");
            return;
        };
        let session = test_session();
        let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
        let rt = HerdrRuntime::new(bin, session);

        let (first, _) = rt.connect(true).await.unwrap();
        first.server_stop().await.unwrap();
        for _ in 0..50 {
            if first.ping().await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let (unchecked, _) = rt.connect(false).await.unwrap();
        assert!(unchecked.ping().await.is_err(), "steady-state calls do not ping first");

        let (fresh, ws) = rt.connect(true).await.unwrap();
        assert_eq!(fresh.ping().await.unwrap().protocol, PINNED_PROTOCOL);
        assert!(fresh.workspace_list().await.unwrap().iter().any(|w| w.workspace_id == ws));
        rt.shutdown().await;
        assert!(fresh.ping().await.is_err(), "shutdown stopped the replacement");
    }

    #[tokio::test]
    async fn health_starts_the_session_and_reports_the_pinned_protocol() {
        let Some(bin) = std::env::var_os("SONATA_HERDR_BIN").map(PathBuf::from).filter(|p| p.is_file()) else {
            eprintln!("SONATA_HERDR_BIN is not set; skipping live test");
            return;
        };
        let session = test_session();
        let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
        let rt = HerdrRuntime::new(bin, session);
        let health = rt.health().await.unwrap();
        assert!(health.version.contains("0.9.3"), "{health:?}");
        assert_eq!(health.protocol, PINNED_PROTOCOL);
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn the_runtime_waits_for_the_env_source_once() {
        let Some(bin) = std::env::var_os("SONATA_HERDR_BIN").map(PathBuf::from).filter(|p| p.is_file()) else {
            eprintln!("SONATA_HERDR_BIN is not set; skipping live test");
            return;
        };
        let session = test_session();
        let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
        let runs = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counted = runs.clone();
        let source: EnvSource = Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            counted.fetch_add(1, Ordering::SeqCst);
            vec![("SONATA_ENV_PROBE".to_string(), "late".to_string())]
        });
        let rt = HerdrRuntime::new_with_env_source(bin, session, source);
        let started = std::time::Instant::now();
        rt.health().await.unwrap();
        assert!(started.elapsed() >= Duration::from_millis(300), "the first start waited for the probe");
        rt.health().await.unwrap();
        assert_eq!(runs.load(Ordering::SeqCst), 1, "the probe ran once");
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn a_cancelled_first_start_keeps_the_env_source() {
        let Some(bin) = std::env::var_os("SONATA_HERDR_BIN").map(PathBuf::from).filter(|p| p.is_file()) else {
            eprintln!("SONATA_HERDR_BIN is not set; skipping live test");
            return;
        };
        let session = test_session();
        let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
        let runs = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counted = runs.clone();
        let (began_tx, began_rx) = tokio::sync::oneshot::channel();
        let source: EnvSource = Box::pin(async move {
            let _ = began_tx.send(());
            tokio::time::sleep(Duration::from_millis(300)).await;
            counted.fetch_add(1, Ordering::SeqCst);
            vec![("SONATA_ENV_PROBE".to_string(), "late".to_string())]
        });
        let rt = HerdrRuntime::new_with_env_source(bin, session, source);
        tokio::select! {
            _ = rt.health() => panic!("the first start finished before it was cancelled"),
            began = began_rx => began.expect("the source began"),
        }
        assert_eq!(runs.load(Ordering::SeqCst), 0, "cancelled mid-probe");
        rt.health().await.unwrap();
        assert_eq!(runs.load(Ordering::SeqCst), 1, "the probe ran once, resumed by the second start");
        let env = rt.env.get().expect("the env is set");
        assert_eq!(
            env,
            &vec![("SONATA_ENV_PROBE".to_string(), "late".to_string())],
            "the probe's env reached the runtime"
        );
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn a_quit_during_the_env_source_starts_no_session() {
        let Some(bin) = std::env::var_os("SONATA_HERDR_BIN").map(PathBuf::from).filter(|p| p.is_file()) else {
            eprintln!("SONATA_HERDR_BIN is not set; skipping live test");
            return;
        };
        let session = test_session();
        let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
        let (began_tx, began_rx) = tokio::sync::oneshot::channel();
        let (answer_tx, answer_rx) = tokio::sync::oneshot::channel::<()>();
        let source: EnvSource = Box::pin(async move {
            let _ = began_tx.send(());
            let _ = answer_rx.await;
            Vec::new()
        });
        let rt = HerdrRuntime::new_with_env_source(bin, session.clone(), source);
        let starting = tokio::spawn({
            let rt = rt.clone();
            async move { rt.health().await }
        });
        began_rx.await.expect("the source began");
        let quitting = tokio::spawn({
            let rt = rt.clone();
            async move { rt.shutdown().await }
        });
        for _ in 0..100 {
            if rt.closed.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(rt.closed.load(Ordering::SeqCst), "shutdown began");
        answer_tx.send(()).unwrap();
        match starting.await.unwrap() {
            Err(RuntimeError::Other(m)) => assert!(m.contains("shut down"), "{m}"),
            other => panic!("expected a shut-down error, got {other:?}"),
        }
        quitting.await.unwrap();
        let socket = HerdrClient::new(session_socket_path(&session)).with_timeout(HEALTH_TIMEOUT);
        assert!(socket.ping().await.is_err(), "no session answers");
    }

    #[tokio::test]
    async fn the_guarded_shell_exits_after_the_next_command() {
        let Some(bin) = std::env::var_os("SONATA_HERDR_BIN").map(PathBuf::from).filter(|p| p.is_file()) else {
            eprintln!("SONATA_HERDR_BIN is not set; skipping live test");
            return;
        };
        let session = test_session();
        let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
        let rt = HerdrRuntime::new(bin, session);
        let (c, ws) = rt.connect(true).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let created = c.tab_create(&ws, "guard", dir.path(), &Default::default()).await.unwrap();
        let pane = created.root_pane.pane_id;
        let mut events = c.subscribe(&[Subscription::pane_exited()]).await.unwrap();

        c.pane_send_input(&pane, &shell_setup_line("/nonexistent"), &["enter"]).await.unwrap();
        c.pane_wait_for_substring(&pane, READY_MARKER, SHELL_READY_TIMEOUT_MS).await.unwrap();
        c.pane_send_input(&pane, "true", &["enter"]).await.unwrap();
        let sent = std::time::Instant::now();
        tokio::time::sleep(Duration::from_millis(300)).await;
        c.pane_send_input(&pane, "printf 'C1_%s\\n' LEAKED", &["enter"]).await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
        let screen = c.pane_read(&pane, ReadSource::Visible, 60).await.unwrap();
        assert!(screen.contains("C1_%s"), "the late input was typed: {screen}");
        assert!(!screen.contains("C1_LEAKED"), "the shell ran the late input: {screen}");

        let exited = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match events.next().await {
                    Ok(Some(Event::PaneExited { pane_id })) if pane_id == pane => return,
                    Ok(Some(_)) | Err(HerdrError::Protocol(_)) => {}
                    other => panic!("event stream ended: {other:?}"),
                }
            }
        })
        .await;
        assert!(exited.is_ok(), "no pane.exited within 10 s");
        println!("pane exited {:?} after `true`", sent.elapsed());
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn a_guard_that_cannot_be_armed_prints_no_marker() {
        let Some(bin) = std::env::var_os("SONATA_HERDR_BIN").map(PathBuf::from).filter(|p| p.is_file()) else {
            eprintln!("SONATA_HERDR_BIN is not set; skipping live test");
            return;
        };
        let session = test_session();
        let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
        let rt = HerdrRuntime::new(bin, session);
        let (c, ws) = rt.connect(true).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let created = c.tab_create(&ws, "guard", dir.path(), &Default::default()).await.unwrap();
        let pane = created.root_pane.pane_id;

        c.pane_send_input(&pane, " readonly PROMPT_COMMAND precmd_functions", &["enter"]).await.unwrap();
        c.pane_send_input(&pane, &shell_setup_line("/nonexistent"), &["enter"]).await.unwrap();
        let marker = c.pane_wait_for_substring(&pane, READY_MARKER, 4_000).await;
        let screen = c.pane_read(&pane, ReadSource::Visible, 60).await.unwrap();
        assert!(marker.is_err(), "marker printed without a guard: {screen}");
        assert!(screen.contains("read"), "the shell reported the read-only variable: {screen}");
        rt.shutdown().await;
    }

    fn agent_json(status: &str, cli: Option<&str>, interactive: Option<bool>) -> serde_json::Value {
        serde_json::json!({
            "pane_id": "w1:p2", "tab_id": "w1:t2", "workspace_id": "w1",
            "agent_status": status, "agent": cli, "interactive_ready": interactive,
        })
    }

    fn info(status: &str, cli: Option<&str>, interactive: Option<bool>) -> AgentInfo {
        serde_json::from_value(agent_json(status, cli, interactive)).unwrap()
    }

    fn got(status: &str, cli: Option<&str>, interactive: Option<bool>) -> serde_json::Value {
        serde_json::json!({"result": {"type": "agent", "agent": agent_json(status, cli, interactive)}})
    }

    fn not_found() -> serde_json::Value {
        serde_json::json!({"error": {"code": "agent_not_found", "message": "agent target b not found"}})
    }

    struct Scripted {
        client: HerdrClient,
        calls: Arc<std::sync::atomic::AtomicUsize>,
        _dir: tempfile::TempDir,
    }

    fn scripted(replies: Vec<serde_json::Value>) -> Scripted {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("h.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = calls.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let n = count.fetch_add(1, Ordering::SeqCst);
                let mut reply = replies[n.min(replies.len() - 1)].clone();
                tokio::spawn(async move {
                    let (r, mut w) = stream.into_split();
                    let mut line = String::new();
                    BufReader::new(r).read_line(&mut line).await.unwrap();
                    let req: serde_json::Value = serde_json::from_str(&line).unwrap();
                    reply["id"] = req["id"].clone();
                    let mut out = serde_json::to_vec(&reply).unwrap();
                    out.push(b'\n');
                    let _ = w.write_all(&out).await;
                });
            }
        });
        Scripted { client: HerdrClient::new(&socket), calls, _dir: dir }
    }

    fn screen(text: &str) -> serde_json::Value {
        serde_json::json!({"result": {"type": "agent_read", "read": {"text": text}}})
    }

    fn sent() -> serde_json::Value {
        serde_json::json!({"result": {"type": "ok"}})
    }

    const QUESTION: &str = "Do you trust the contents of this directory?";

    #[tokio::test]
    async fn a_key_question_on_screen_is_answered_and_waited_out() {
        let h = scripted(vec![screen(&format!("{QUESTION} (y/n)")), sent(), screen("> ")]);
        assert!(answer_key_prompt(&h.client, "b", QUESTION, "y", false).await.unwrap());
        assert_eq!(h.calls.load(Ordering::SeqCst), 3, "read (question), send y, read (gone)");
    }

    #[tokio::test]
    async fn a_missing_key_question_fails_only_when_one_is_required() {
        let h = scripted(vec![screen("> ")]);
        assert!(!answer_key_prompt(&h.client, "b", QUESTION, "y", false).await.unwrap());
        assert_eq!(h.calls.load(Ordering::SeqCst), 1, "nothing was sent");

        let h = scripted(vec![screen("Allow network access? ❯ Yes")]);
        let err = answer_key_prompt(&h.client, "b", QUESTION, "y", true).await.unwrap_err().to_string();
        assert!(err.contains("agent blocked during startup") && err.contains("Allow network access?"), "{err}");
        assert_eq!(h.calls.load(Ordering::SeqCst), 1, "no key for a dialog it does not know");
    }

    const FAST: ReadyTiming = ReadyTiming {
        recheck: Duration::from_millis(10),
        poll: Duration::from_millis(10),
        settle: Duration::from_millis(300),
    };

    #[tokio::test]
    async fn a_start_is_confirmed_when_the_agent_is_still_ready() {
        let h = scripted(vec![got("idle", Some("claude"), Some(true))]);
        let first = info("idle", Some("claude"), Some(true));
        let confirmed = confirm_ready(&h.client, "b", first, &FAST).await.unwrap();
        assert_eq!(confirmed.agent_status, AgentStatus::Idle);
        assert_eq!(h.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_agent_that_is_alive_but_busy_is_waited_for_and_confirmed_again() {
        let h = scripted(vec![
            got("working", Some("claude"), Some(true)),
            got("blocked", Some("claude"), None),
            got("idle", Some("claude"), Some(false)),
            got("idle", Some("claude"), Some(true)),
        ]);
        let first = info("idle", Some("claude"), Some(true));
        let confirmed = confirm_ready(&h.client, "b", first, &FAST).await.unwrap();
        assert_eq!(confirmed.interactive_ready, Some(true));
        assert_eq!(h.calls.load(Ordering::SeqCst), 5);

        let h = scripted(vec![got("idle", Some("claude"), Some(true))]);
        let first = info("working", Some("claude"), Some(true));
        assert!(confirm_ready(&h.client, "b", first, &FAST).await.is_ok());
        assert_eq!(h.calls.load(Ordering::SeqCst), 2, "poll (ready), recheck (ready)");
    }

    #[tokio::test]
    async fn an_agent_that_exits_after_looking_ready_fails_the_start() {
        for gone in [not_found(), got("idle", None, Some(true))] {
            let h = scripted(vec![gone.clone()]);
            let first = info("idle", Some("claude"), Some(true));
            let err = confirm_ready(&h.client, "b", first, &FAST).await.unwrap_err().to_string();
            assert!(err.contains("exited right after it became ready"), "{gone}: {err}");
            assert_eq!(h.calls.load(Ordering::SeqCst), 1, "no waiting for a gone agent");
        }
    }

    #[tokio::test]
    async fn an_agent_that_stays_busy_fails_once_the_bound_expires() {
        let h = scripted(vec![got("blocked", Some("claude"), None)]);
        let started = std::time::Instant::now();
        let first = info("idle", Some("claude"), Some(true));
        let err = confirm_ready(&h.client, "b", first, &FAST).await.unwrap_err().to_string();
        assert!(err.contains("not ready within"), "{err}");
        assert!(started.elapsed() >= FAST.settle);
    }

    #[test]
    fn inspect_reports_a_gone_agent_only_when_herdr_says_so() {
        let server = |code: &str| HerdrError::Server { code: code.into(), message: "x".into() };
        assert_eq!(agent_error(server("agent_not_found")), RuntimeError::Gone);
        assert_eq!(agent_error(server("pane_not_found")), RuntimeError::Gone);
        assert!(matches!(agent_error(server("internal_error")), RuntimeError::Other(_)));
        let timeout = std::io::Error::new(std::io::ErrorKind::TimedOut, "no answer");
        assert!(matches!(agent_error(HerdrError::Io(timeout)), RuntimeError::Other(_)));

        assert_eq!(inspection(info("idle", None, Some(true))), Err(RuntimeError::Gone));
        let alive = inspection(info("working", Some("claude"), None)).unwrap();
        assert_eq!(alive.state, AgentState::Working);
    }

    #[test]
    fn ready_means_an_idle_cli_that_herdr_still_detects() {
        assert!(ready_for_input(&info("idle", Some("claude"), Some(true))));
        assert!(ready_for_input(&info("done", Some("claude"), Some(true))));
        assert!(ready_for_input(&info("idle", Some("claude"), None)), "older herdr: no field");
        assert!(!ready_for_input(&info("idle", Some("claude"), Some(false))));
        assert!(!ready_for_input(&info("idle", None, Some(true))));
        for status in ["working", "blocked", "unknown"] {
            assert!(!ready_for_input(&info(status, Some("claude"), Some(true))), "{status}");
        }
    }

    #[test]
    fn shell_setup_line_stays_out_of_shell_history_and_guards_the_shell() {
        assert_eq!(
            shell_setup_line("/Apps/Sonata/bin"),
            concat!(
                r#" unset HISTFILE; export PATH='/Apps/Sonata/bin':"$PATH"; "#,
                r#"[ -n "$ZSH_VERSION$BASH_VERSION" ] && eval '__sonata_n=0; __sonata_guard() { "#,
                r#"__sonata_n=$((__sonata_n+1)); if [ "$__sonata_n" -ge 2 ]; then exec /bin/sleep 5 || exit; fi; }; "#,
                r#"if [ -n "$ZSH_VERSION" ]; then precmd_functions=(__sonata_guard $precmd_functions); "#,
                r#"else PROMPT_COMMAND="__sonata_guard${PROMPT_COMMAND:+;$PROMPT_COMMAND}"; fi' "#,
                r#"|| [ -z "$ZSH_VERSION$BASH_VERSION" ] && printf '__SONATA_%s__\n' READY"#
            )
        );
        assert!(!shell_setup_line("/x").contains(READY_MARKER), "only the output has the marker");
    }

    #[tokio::test]
    async fn closed_runtime_does_not_start_a_session() {
        let rt = HerdrRuntime::new(PathBuf::from("/nonexistent/herdr"), "x");
        rt.shutdown().await;
        rt.shutdown().await;
        let spec = LaunchSpec {
            handle: "b".into(),
            agent: "b".into(),
            label: "b-1".into(),
            kind: "claude".into(),
            cwd: PathBuf::from("/tmp"),
            args: vec![],
            env: Default::default(),
            path_prefix: PathBuf::from("/tmp"),
            trust: None,
            sign_in: None,
        };
        match rt.start_bot(&spec).await {
            Err(RuntimeError::Other(m)) => assert!(m.contains("shut down"), "{m}"),
            other => panic!("expected shut-down error, got {other:?}"),
        }
    }

    const CODEX_TRUST: &str = "  Trust this folder? Codex can read, edit, and run files here…\n› 1. Trust and continue\n  2. Quit\n  enter continue · esc quit";

    #[tokio::test]
    async fn a_codex_trust_menu_is_accepted_at_its_marker() {
        let h = scripted(vec![screen(CODEX_TRUST), sent(), got("idle", Some("codex"), Some(true))]);
        accept_folder_trust(&h.client, "b", "Trust and continue").await.unwrap();
        assert_eq!(h.calls.load(Ordering::SeqCst), 3, "read (selected), enter, get (idle): no down key");
    }

    const SIGN_IN: &str = "  Welcome to Codex, OpenAI's command-line coding agent\n> 1. Sign in with ChatGPT\n  2. Sign in with Device Code\n  Press enter to continue";

    #[tokio::test]
    async fn a_sign_in_screen_fails_the_start_and_gets_no_input() {
        let h = scripted(vec![screen(SIGN_IN)]);
        let err = refuse_sign_in(&h.client, "b", "1. Sign in with ChatGPT").await.unwrap_err().to_string();
        assert!(err.contains("not signed in") && err.contains("Sign in with ChatGPT"), "{err}");
        assert_eq!(h.calls.load(Ordering::SeqCst), 1, "one read, no keys");
    }

    #[tokio::test]
    async fn a_resumed_transcript_that_mentions_sign_in_starts() {
        let h = scripted(vec![screen(
            "› why does it say Sign in with ChatGPT?\n• The start failed because codex showed \"Sign in with ChatGPT\".\n› Ask Codex to do anything",
        )]);
        refuse_sign_in(&h.client, "b", "1. Sign in with ChatGPT").await.unwrap();
    }

    #[tokio::test]
    async fn a_cli_past_its_sign_in_starts() {
        let h = scripted(vec![screen("› Ask Codex to do anything")]);
        refuse_sign_in(&h.client, "b", "1. Sign in with ChatGPT").await.unwrap();
    }
}
