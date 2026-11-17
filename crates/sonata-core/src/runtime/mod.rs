use std::collections::BTreeMap;
use std::path::PathBuf;

use async_trait::async_trait;
use tokio::sync::broadcast;

#[cfg(any(test, feature = "test-util"))]
pub mod fake;
pub mod herdr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Idle,
    Working,
    Blocked,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustPrompt {
    Menu { option: &'static str },
    Key { question: &'static str, key: &'static str },
}

#[derive(Debug, Clone, PartialEq)]
pub struct LaunchSpec {
    pub handle: String,
    pub agent: String,
    pub label: String,
    pub kind: String,
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub path_prefix: PathBuf,
    pub trust: Option<TrustPrompt>,
    pub sign_in: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeBot {
    pub handle: String,
    pub pane_id: String,
    pub tab_id: String,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Inspection {
    pub state: AgentState,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeEvent {
    State { pane_id: String, state: AgentState },
    Exited { pane_id: String },
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RuntimeError {
    #[error("the bot is showing a dialog")]
    Blocked,
    #[error("the bot did not react to the prompt")]
    Stalled,
    #[error("could not start the bot: {0}")]
    StartFailed(String),
    #[error("the bot's CLI is no longer running")]
    Gone,
    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RuntimeHealth {
    pub version: String,
    pub protocol: u32,
}

#[async_trait]
pub trait BotRuntime: Send + Sync + 'static {
    async fn health(&self) -> Result<RuntimeHealth, RuntimeError>;
    async fn start_bot(&self, spec: &LaunchSpec) -> Result<RuntimeBot, RuntimeError>;
    async fn prompt(&self, bot: &RuntimeBot, text: &str) -> Result<(), RuntimeError>;
    async fn inspect(&self, bot: &RuntimeBot) -> Result<Inspection, RuntimeError>;
    async fn read_output(&self, bot: &RuntimeBot, lines: u32) -> Result<String, RuntimeError>;
    async fn snapshot(&self, bot: &RuntimeBot, lines: u32) -> Result<String, RuntimeError>;
    async fn terminal(&self, bot: &RuntimeBot, lines: u32) -> Result<String, RuntimeError>;
    async fn send_keys(&self, bot: &RuntimeBot, keys: &[&str]) -> Result<(), RuntimeError>;
    async fn stop_bot(&self, bot: &RuntimeBot) -> Result<(), RuntimeError>;
    async fn shutdown(&self);
    fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent>;
}
