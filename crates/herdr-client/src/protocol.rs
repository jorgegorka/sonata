use serde::{Deserialize, Serialize};

pub const PINNED_PROTOCOL: u32 = 22;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadSource {
    Visible,
    Recent,
    RecentUnwrapped,
    Detection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadFormat {
    Text,
    Ansi,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pong {
    pub version: String,
    pub protocol: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceInfo {
    pub workspace_id: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TabInfo {
    pub tab_id: String,
    pub workspace_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PaneInfo {
    pub pane_id: String,
    pub tab_id: String,
    pub workspace_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Created {
    #[serde(default)]
    pub workspace: Option<WorkspaceInfo>,
    pub tab: TabInfo,
    pub root_pane: PaneInfo,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AgentSessionInfo {
    pub agent: String,
    pub kind: String,
    pub source: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentInfo {
    pub pane_id: String,
    pub tab_id: String,
    pub workspace_id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub agent_status: AgentStatus,
    #[serde(default)]
    pub agent_session: Option<AgentSessionInfo>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub interactive_ready: Option<bool>,
}

impl AgentInfo {
    pub fn session_id(&self) -> Option<&str> {
        self.agent_session.as_ref().filter(|s| s.kind == "id").map(|s| s.value.as_str())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ReadEnvelope {
    pub read: ReadResult,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ReadResult {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct ErrorBody {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponseLine {
    pub id: String,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<ErrorBody>,
}
