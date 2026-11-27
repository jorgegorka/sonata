use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::new_id;

pub const USER: &str = "user";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Claude,
    Cursor,
    Codex,
    Grok,
    Opencode,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Cursor => "cursor",
            Provider::Codex => "codex",
            Provider::Grok => "grok",
            Provider::Opencode => "opencode",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(Provider::Claude),
            "cursor" => Some(Provider::Cursor),
            "codex" => Some(Provider::Codex),
            "grok" => Some(Provider::Grok),
            "opencode" => Some(Provider::Opencode),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub default_directory: Option<String>,
    pub default_provider: Provider,
    pub master_id: Option<String>,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectInput {
    pub name: String,
    pub default_directory: Option<String>,
    pub default_provider: Provider,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bot {
    pub project_id: String,
    pub id: String,
    pub handle: String,
    pub display_name: String,
    pub color: String,
    pub provider: Provider,
    pub model: Option<String>,
    pub instructions: String,
    pub config_version: i64,
    #[serde(default)]
    pub skill_ids: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub project_id: String,
    pub id: String,
    pub bot_id: String,
    pub number: i64,
    pub title: Option<String>,
    pub provider_session_id: Option<String>,
    pub session_config_version: Option<i64>,
    pub opened_by: Option<String>,
    pub created_at: String,
    pub last_activity_at: String,
    pub archived_at: Option<String>,
    pub waiting_question_id: Option<String>,
    pub waiting_since: Option<String>,
    pub open_request_id: Option<String>,
    pub seen_at: Option<String>,
    #[serde(default)]
    pub waiting_options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewBot {
    pub display_name: String,
    pub color: String,
    pub provider: Provider,
    pub model: Option<String>,
    pub instructions: String,
    #[serde(default)]
    pub skill_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    Written,
    Imported,
}

impl SkillSource {
    pub fn as_str(self) -> &'static str {
        match self {
            SkillSource::Written => "written",
            SkillSource::Imported => "imported",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "written" => Some(SkillSource::Written),
            "imported" => Some(SkillSource::Imported),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub source: SkillSource,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SkillView {
    #[serde(flatten)]
    pub skill: Skill,
    pub used_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NewSkill {
    pub name: String,
    pub description: String,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Text,
    RawFallback,
    System,
}

impl MessageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MessageKind::Text => "text",
            MessageKind::RawFallback => "raw_fallback",
            MessageKind::System => "system",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "text" => Some(MessageKind::Text),
            "raw_fallback" => Some(MessageKind::RawFallback),
            "system" => Some(MessageKind::System),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    Queued,
    Delivered,
    Cancelled,
    Interrupted,
    Failed,
    Retried,
}

impl MessageStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MessageStatus::Queued => "queued",
            MessageStatus::Delivered => "delivered",
            MessageStatus::Cancelled => "cancelled",
            MessageStatus::Interrupted => "interrupted",
            MessageStatus::Failed => "failed",
            MessageStatus::Retried => "retried",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(MessageStatus::Queued),
            "delivered" => Some(MessageStatus::Delivered),
            "cancelled" => Some(MessageStatus::Cancelled),
            "interrupted" => Some(MessageStatus::Interrupted),
            "failed" => Some(MessageStatus::Failed),
            "retried" => Some(MessageStatus::Retried),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub thread_id: String,
    pub sender: String,
    pub recipient: String,
    pub sender_session: Option<String>,
    pub recipient_session: Option<String>,
    pub kind: MessageKind,
    pub body: String,
    pub attachments: Vec<Attachment>,
    pub system_code: Option<String>,
    pub system_params: Option<Value>,
    pub status: MessageStatus,
    pub chain_id: String,
    pub hop: i64,
    pub in_reply_to: Option<String>,
    pub created_at: String,
    pub delivered_at: Option<String>,
    pub task_id: Option<String>,
    pub task_name: Option<String>,
    pub task_since: Option<String>,
    pub workflow_mark: Option<WorkflowMark>,
    #[serde(default)]
    pub workflow_request: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewMessage {
    pub sender: String,
    pub recipient: String,
    pub sender_session: Option<String>,
    pub recipient_session: Option<String>,
    pub kind: MessageKind,
    pub body: String,
    pub attachments: Vec<Attachment>,
    pub system_code: Option<String>,
    pub system_params: Option<Value>,
    pub status: MessageStatus,
    pub chain_id: String,
    pub hop: i64,
    pub in_reply_to: Option<String>,
}

impl NewMessage {
    pub fn system(session: &Session, code: &str, params: Value) -> Self {
        Self {
            sender: session.bot_id.clone(),
            recipient: USER.to_string(),
            sender_session: Some(session.id.clone()),
            recipient_session: None,
            kind: MessageKind::System,
            body: String::new(),
            attachments: Vec::new(),
            system_code: Some(code.to_string()),
            system_params: Some(params),
            status: MessageStatus::Delivered,
            chain_id: new_id(),
            hop: 0,
            in_reply_to: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotStatus {
    Offline,
    Starting,
    Idle,
    Working,
    NeedsAttention,
    Limited,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BotView {
    #[serde(flatten)]
    pub bot: Bot,
    pub status: BotStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UsageLimitView {
    pub provider: Provider,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionView {
    #[serde(flatten)]
    pub session: Session,
    pub status: BotStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemovedSessions {
    pub sessions: Vec<RemovedSession>,
    pub thread_ids: Vec<String>,
    pub messages: Vec<Message>,
    pub drafts: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemovedSession {
    pub session: Session,
    pub thread_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThreadSummary {
    pub id: String,
    pub participant_a: String,
    pub participant_b: String,
    pub session_a: Option<String>,
    pub session_b: Option<String>,
    pub last_message_at: String,
}

#[cfg(test)]
pub(crate) fn test_bot(instructions: &str, model: Option<&str>) -> Bot {
    Bot {
        project_id: "default".into(),
        id: "01JBOT".into(),
        handle: "researcher".into(),
        display_name: "Researcher".into(),
        color: "#3b82f6".into(),
        provider: Provider::Claude,
        model: model.map(str::to_string),
        instructions: instructions.into(),
        config_version: 1,
        skill_ids: vec![],
        created_at: "t".into(),
        updated_at: "t".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn enums_round_trip_through_strings() {
        for p in [Provider::Claude, Provider::Cursor, Provider::Codex, Provider::Grok, Provider::Opencode] {
            assert_eq!(Provider::parse(p.as_str()), Some(p));
        }
        for k in [MessageKind::Text, MessageKind::RawFallback, MessageKind::System] {
            assert_eq!(MessageKind::parse(k.as_str()), Some(k));
        }
        for s in [
            MessageStatus::Queued,
            MessageStatus::Delivered,
            MessageStatus::Cancelled,
            MessageStatus::Interrupted,
            MessageStatus::Failed,
            MessageStatus::Retried,
        ] {
            assert_eq!(MessageStatus::parse(s.as_str()), Some(s));
        }
        for s in [SkillSource::Written, SkillSource::Imported] {
            assert_eq!(SkillSource::parse(s.as_str()), Some(s));
        }
        for s in [RunStatus::Running, RunStatus::Paused, RunStatus::Finished, RunStatus::Stopped] {
            assert_eq!(RunStatus::parse(s.as_str()), Some(s));
            assert_eq!(serde_json::to_value(s).unwrap(), s.as_str());
        }
        assert_eq!(Provider::parse("gpt"), None);
    }

    #[test]
    fn a_pause_reason_is_tagged_with_its_kind() {
        let reason = PauseReason::VisitCap { step_id: "review".into() };
        assert_eq!(serde_json::to_value(&reason).unwrap(), json!({"kind": "visit_cap", "step_id": "review"}));
        let back: PauseReason = serde_json::from_value(json!({"kind": "bot_missing", "step_id": "s2"})).unwrap();
        assert_eq!(back, PauseReason::BotMissing { step_id: "s2".into() });
        assert_eq!(serde_json::to_value(PauseReason::NoResult { step_id: "s1".into() }).unwrap()["kind"], "no_result");
    }

    #[test]
    fn bot_view_serializes_flat_with_snake_case_status() {
        let view = BotView {
            bot: Bot {
                project_id: "default".into(),
                id: "01J".into(),
                handle: "researcher".into(),
                display_name: "Researcher".into(),
                color: "#3b82f6".into(),
                provider: Provider::Claude,
                model: None,
                instructions: "Find things.".into(),
                config_version: 1,
                skill_ids: vec![],
                created_at: "t".into(),
                updated_at: "t".into(),
            },
            status: BotStatus::NeedsAttention,
        };
        let v = serde_json::to_value(&view).unwrap();
        assert_eq!(v["handle"], "researcher");
        assert_eq!(v["provider"], "claude");
        assert_eq!(v["status"], "needs_attention");
    }

    #[test]
    fn system_message_goes_to_user_and_is_delivered() {
        let session = Session {
            project_id: "default".into(),
            id: "s1".into(),
            bot_id: "bot1".into(),
            number: 1,
            title: None,
            provider_session_id: None,
            session_config_version: None,
            opened_by: None,
            created_at: "t".into(),
            last_activity_at: "t".into(),
            archived_at: None,
            waiting_question_id: None,
            waiting_since: None,
            open_request_id: None,
            seen_at: None,
            waiting_options: vec![],
        };
        let m = NewMessage::system(&session, "session_reset", json!({}));
        assert_eq!((m.sender.as_str(), m.sender_session.as_deref()), ("bot1", Some("s1")));
        assert_eq!((m.recipient.as_str(), m.recipient_session.as_deref()), (USER, None));
        assert_eq!(m.kind, MessageKind::System);
        assert_eq!(m.status, MessageStatus::Delivered);
        assert_eq!(m.system_code.as_deref(), Some("session_reset"));
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Task {
    pub project_id: String,
    pub id: String,
    pub name: String,
    pub bot_id: String,
    pub prompt: String,
    pub every_days: u32,
    pub at: String,
    pub starts_on: String,
    pub enabled: bool,
    pub next_run_at: Option<String>,
    pub last_run_at: Option<String>,
    pub last_run_session: Option<String>,
    pub skipped_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NewTask {
    pub name: String,
    pub bot_id: String,
    pub prompt: String,
    pub every_days: u32,
    pub at: String,
    pub starts_on: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskRow {
    pub name: String,
    pub bot_id: String,
    pub prompt: String,
    pub every_days: u32,
    pub at: String,
    pub starts_on: String,
    pub enabled: bool,
    pub next_run_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CardLine {
    pub text: String,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SentLine {
    pub recipient: String,
    pub text: String,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CutOff {
    pub message_id: String,
    pub sender: String,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reply {
    pub message_id: String,
    pub text: String,
    pub files: Vec<String>,
    pub at: String,
    pub raw: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Glance {
    pub session_id: String,
    pub step: Option<String>,
    pub background_work: bool,
    pub tail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionActivity {
    pub session_id: String,
    pub working_on: Option<CardLine>,
    pub queued: u32,
    pub question: Option<String>,
    pub last_sent: Option<SentLine>,
    pub cut_off: Option<CutOff>,
    pub last_reply: Option<Reply>,
    pub last_for_user_at: Option<String>,
    pub run_of: Option<String>,
    pub workflow: Option<RunGlance>,
}

pub const WORKFLOW_FORMAT: u32 = 1;
pub const DONE: &str = "done";
pub const START: &str = "start";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pos {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Arrow {
    pub to: String,
    #[serde(default)]
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    #[serde(default)]
    pub automatic_child_review: bool,
    pub id: String,
    pub name: String,
    pub bot_id: String,
    pub instructions: String,
    #[serde(default)]
    pub arrows: Vec<Arrow>,
    #[serde(default)]
    pub pos: Option<Pos>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowDef {
    pub version: u32,
    pub start: Option<String>,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub start_pos: Option<Pos>,
    #[serde(default)]
    pub done_pos: Option<Pos>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkRole {
    Request,
    Reminder,
    Result,
    Finished,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowMark {
    pub run_id: String,
    pub visit_id: String,
    pub workflow: String,
    pub step: String,
    pub visit: u32,
    pub role: MarkRole,
    #[serde(default)]
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workflow {
    pub project_id: String,
    pub id: String,
    pub name: String,
    pub when_to_use: String,
    pub def: WorkflowDef,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewWorkflow {
    pub name: String,
    #[serde(default)]
    pub when_to_use: String,
    pub def: WorkflowDef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Paused,
    Finished,
    Stopped,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Paused => "paused",
            RunStatus::Finished => "finished",
            RunStatus::Stopped => "stopped",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "running" => Some(RunStatus::Running),
            "paused" => Some(RunStatus::Paused),
            "finished" => Some(RunStatus::Finished),
            "stopped" => Some(RunStatus::Stopped),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PauseReason {
    VisitCap { step_id: String },
    NoResult { step_id: String },
    BotMissing { step_id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub project_id: String,
    pub id: String,
    pub workflow_id: Option<String>,
    pub name: String,
    pub def: WorkflowDef,
    pub request: String,
    pub request_files: Vec<String>,
    pub opener_session: Option<String>,
    pub root_session: Option<String>,
    pub status: RunStatus,
    pub pause: Option<PauseReason>,
    pub extra_visits: BTreeMap<String, u32>,
    pub created_at: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunGlance {
    pub run_id: String,
    pub name: String,
    pub status: RunStatus,
    pub step: Option<String>,
    pub handle: Option<String>,
    pub visit: u32,
    pub pause: Option<PauseReason>,
    pub outcomes: Vec<String>,
    pub holder: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunView {
    pub run: WorkflowRun,
    pub visits: Vec<Visit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Visit {
    pub id: String,
    pub run_id: String,
    pub ord: i64,
    pub step_id: String,
    pub n: u32,
    pub session_id: Option<String>,
    pub request_message: String,
    pub result_message: Option<String>,
    pub outcome: Option<String>,
    pub reminded: bool,
    pub started_at: String,
    pub ended_at: Option<String>,
}
