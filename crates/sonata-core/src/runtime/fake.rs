use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::broadcast;

use super::*;

pub struct FakeState {
    pub health: Result<RuntimeHealth, RuntimeError>,
    pub starts: Vec<LaunchSpec>,
    pub prompts: Vec<(String, String)>,
    pub keys: Vec<(String, Vec<String>)>,
    pub stopped: Vec<String>,
    pub output: String,
    pub screen: String,
    pub ansi_screen: String,
    pub state: AgentState,
    pub session_id: Option<String>,
    pub pane_states: HashMap<String, AgentState>,
    pub pane_session_ids: HashMap<String, String>,
    pub pane_outputs: HashMap<String, String>,
    agent_panes: HashMap<String, String>,
    pub start_failures: VecDeque<RuntimeError>,
    pub prompt_failures: VecDeque<RuntimeError>,
    pub inspect_failures: VecDeque<RuntimeError>,
    pub key_failures: VecDeque<RuntimeError>,
    pub snapshot_failures: VecDeque<RuntimeError>,
    pub hang_starts: bool,
    pub start_delay: Option<Duration>,
    pub prompt_delay: Option<Duration>,
    pub snapshot_delay: Option<Duration>,
    pub snapshots: u32,
    pub shutdown_called: bool,
    pub shutdown_error: Option<RuntimeError>,
    next_pane: u32,
}

pub struct FakeRuntime {
    state: Mutex<FakeState>,
    tx: broadcast::Sender<RuntimeEvent>,
}

impl FakeState {
    pub fn pane_of(&self, agent: &str) -> Option<String> {
        self.agent_panes.get(agent).cloned()
    }

    fn session_of(&self, pane_id: &str) -> Option<String> {
        self.pane_session_ids.get(pane_id).cloned().or_else(|| self.session_id.clone())
    }
}

impl FakeRuntime {
    pub fn new() -> Arc<Self> {
        let (tx, _) = broadcast::channel(256);
        Arc::new(Self {
            state: Mutex::new(FakeState {
                health: Ok(RuntimeHealth { version: "0.9.5".into(), protocol: herdr_client::PINNED_PROTOCOL }),
                starts: vec![],
                prompts: vec![],
                keys: vec![],
                stopped: vec![],
                output: String::new(),
                screen: String::new(),
                ansi_screen: String::new(),
                state: AgentState::Idle,
                session_id: None,
                pane_states: HashMap::new(),
                pane_session_ids: HashMap::new(),
                pane_outputs: HashMap::new(),
                agent_panes: HashMap::new(),
                start_failures: VecDeque::new(),
                prompt_failures: VecDeque::new(),
                inspect_failures: VecDeque::new(),
                key_failures: VecDeque::new(),
                snapshot_failures: VecDeque::new(),
                hang_starts: false,
                start_delay: None,
                prompt_delay: None,
                snapshot_delay: None,
                snapshots: 0,
                shutdown_called: false,
                shutdown_error: None,
                next_pane: 0,
            }),
            tx,
        })
    }

    pub fn state(&self) -> MutexGuard<'_, FakeState> {
        self.state.lock().unwrap()
    }

    pub fn emit(&self, event: RuntimeEvent) {
        let _ = self.tx.send(event);
    }
}

#[async_trait]
impl BotRuntime for FakeRuntime {
    async fn health(&self) -> Result<RuntimeHealth, RuntimeError> {
        self.state().health.clone()
    }

    async fn start_bot(&self, spec: &LaunchSpec) -> Result<RuntimeBot, RuntimeError> {
        let (hang, delay) = {
            let mut s = self.state();
            s.starts.push(spec.clone());
            (s.hang_starts, s.start_delay)
        };
        if hang {
            std::future::pending::<()>().await;
        }
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        let mut s = self.state();
        if let Some(err) = s.start_failures.pop_front() {
            return Err(err);
        }
        s.next_pane += 1;
        let pane_id = format!("w1:p{}", s.next_pane);
        s.agent_panes.insert(spec.agent.clone(), pane_id.clone());
        Ok(RuntimeBot {
            handle: spec.handle.clone(),
            tab_id: format!("w1:t{}", s.next_pane),
            session_id: s.session_of(&pane_id),
            pane_id,
        })
    }

    async fn prompt(&self, bot: &RuntimeBot, text: &str) -> Result<(), RuntimeError> {
        let (delay, failure) = {
            let mut s = self.state();
            s.prompts.push((bot.pane_id.clone(), text.to_string()));
            (s.prompt_delay, s.prompt_failures.pop_front())
        };
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        match failure {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    async fn inspect(&self, bot: &RuntimeBot) -> Result<Inspection, RuntimeError> {
        let mut s = self.state();
        if let Some(err) = s.inspect_failures.pop_front() {
            return Err(err);
        }
        let state = s.pane_states.get(&bot.pane_id).copied().unwrap_or(s.state);
        Ok(Inspection { state, session_id: s.session_of(&bot.pane_id) })
    }

    async fn read_output(&self, bot: &RuntimeBot, _lines: u32) -> Result<String, RuntimeError> {
        let s = self.state();
        Ok(s.pane_outputs.get(&bot.pane_id).unwrap_or(&s.output).clone())
    }

    async fn snapshot(&self, _bot: &RuntimeBot, _lines: u32) -> Result<String, RuntimeError> {
        let delay = {
            let mut state = self.state();
            state.snapshots += 1;
            state.snapshot_delay
        };
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        let mut s = self.state();
        match s.snapshot_failures.pop_front() {
            Some(err) => Err(err),
            None => Ok(s.screen.clone()),
        }
    }

    async fn terminal(&self, _bot: &RuntimeBot, _lines: u32) -> Result<String, RuntimeError> {
        Ok(self.state().ansi_screen.clone())
    }

    async fn send_keys(&self, bot: &RuntimeBot, keys: &[&str]) -> Result<(), RuntimeError> {
        let mut s = self.state();
        s.keys.push((bot.pane_id.clone(), keys.iter().map(|k| k.to_string()).collect()));
        match s.key_failures.pop_front() {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    async fn stop_bot(&self, bot: &RuntimeBot) -> Result<(), RuntimeError> {
        self.state().stopped.push(bot.pane_id.clone());
        Ok(())
    }

    async fn shutdown(&self) {
        self.state().shutdown_called = true;
    }

    async fn shutdown_checked(&self) -> Result<(), RuntimeError> {
        self.shutdown().await;
        match self.state().shutdown_error.clone() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(args: &[&str]) -> LaunchSpec {
        LaunchSpec {
            handle: "a".into(),
            agent: "a".into(),
            label: "a-1".into(),
            kind: "claude".into(),
            cwd: "/tmp/a".into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: Default::default(),
            path_prefix: "/tmp/bin".into(),
            trust: None,
            sign_in: None,
        }
    }

    #[tokio::test]
    async fn records_calls_and_numbers_panes() {
        let rt = FakeRuntime::new();
        rt.state().session_id = Some("s1".into());
        let a = rt.start_bot(&spec(&[])).await.unwrap();
        let b = rt.start_bot(&spec(&[])).await.unwrap();
        assert_eq!((a.pane_id.as_str(), b.pane_id.as_str()), ("w1:p1", "w1:p2"));
        assert_eq!(a.session_id.as_deref(), Some("s1"));
        rt.prompt(&a, "hello").await.unwrap();
        assert_eq!(rt.state().prompts, vec![("w1:p1".to_string(), "hello".to_string())]);
    }

    #[tokio::test]
    async fn scripted_failures_are_consumed_in_order() {
        let rt = FakeRuntime::new();
        rt.state().start_failures.push_back(RuntimeError::StartFailed("boom".into()));
        assert!(rt.start_bot(&spec(&[])).await.is_err());
        assert!(rt.start_bot(&spec(&[])).await.is_ok());
        assert_eq!(rt.state().starts.len(), 2);
    }

    #[tokio::test]
    async fn hung_starts_are_recorded_and_never_finish() {
        let rt = FakeRuntime::new();
        rt.state().hang_starts = true;
        let spec = spec(&[]);
        let start = rt.start_bot(&spec);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(50), start).await.is_err());
        assert_eq!(rt.state().starts.len(), 1);
    }

    #[tokio::test]
    async fn the_snapshot_is_the_scripted_screen() {
        let rt = FakeRuntime::new();
        let bot = rt.start_bot(&spec(&[])).await.unwrap();
        rt.state().screen = "Allow? ❯ Yes".into();
        assert_eq!(rt.snapshot(&bot, 60).await.unwrap(), "Allow? ❯ Yes");
    }

    #[tokio::test]
    async fn scripted_snapshot_failures_are_consumed_in_order() {
        let rt = FakeRuntime::new();
        let bot = rt.start_bot(&spec(&[])).await.unwrap();
        rt.state().screen = "❯".into();
        rt.state().snapshot_failures.push_back(RuntimeError::Other("no answer".into()));
        assert!(rt.snapshot(&bot, 60).await.is_err());
        assert_eq!(rt.snapshot(&bot, 60).await.unwrap(), "❯");
    }

    #[tokio::test]
    async fn the_terminal_is_the_scripted_ansi_screen() {
        let rt = FakeRuntime::new();
        let bot = rt.start_bot(&spec(&[])).await.unwrap();
        rt.state().ansi_screen = "\u{1b}[1m❯\u{1b}[0m".into();
        assert_eq!(rt.terminal(&bot, 40).await.unwrap(), "\u{1b}[1m❯\u{1b}[0m");
    }

    #[tokio::test]
    async fn emitted_events_reach_subscribers() {
        let rt = FakeRuntime::new();
        let mut rx = rt.subscribe();
        rt.emit(RuntimeEvent::Exited { pane_id: "w1:p1".into() });
        assert_eq!(rx.recv().await.unwrap(), RuntimeEvent::Exited { pane_id: "w1:p1".into() });
    }

    #[tokio::test]
    async fn each_pane_can_be_scripted_on_its_own() {
        let rt = FakeRuntime::new();
        let mut one = spec(&[]);
        one.agent = "a-1".into();
        let mut two = spec(&[]);
        two.agent = "a-2".into();
        rt.state().session_id = Some("global".into());
        let a = rt.start_bot(&one).await.unwrap();
        let b = rt.start_bot(&two).await.unwrap();
        assert_eq!(rt.state().pane_of("a-2"), Some(b.pane_id.clone()));
        assert_eq!(rt.state().pane_of("nope"), None);
        {
            let mut s = rt.state();
            s.state = AgentState::Idle;
            s.pane_states.insert(b.pane_id.clone(), AgentState::Working);
            s.pane_session_ids.insert(b.pane_id.clone(), "s-b".into());
            s.pane_outputs.insert(b.pane_id.clone(), "from b".into());
            s.output = "global out".into();
        }
        let ia = rt.inspect(&a).await.unwrap();
        let ib = rt.inspect(&b).await.unwrap();
        assert_eq!((ia.state, ia.session_id.as_deref()), (AgentState::Idle, Some("global")));
        assert_eq!((ib.state, ib.session_id.as_deref()), (AgentState::Working, Some("s-b")));
        assert_eq!(rt.read_output(&a, 10).await.unwrap(), "global out");
        assert_eq!(rt.read_output(&b, 10).await.unwrap(), "from b");
        assert_eq!(rt.state().starts[1].agent, "a-2");
    }

    #[tokio::test]
    async fn a_pane_session_id_set_before_the_start_is_returned_by_it() {
        let rt = FakeRuntime::new();
        rt.state().pane_session_ids.insert("w1:p1".into(), "s-1".into());
        let a = rt.start_bot(&spec(&[])).await.unwrap();
        assert_eq!(a.session_id.as_deref(), Some("s-1"));
    }
}
