use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeRequest {
    pub v: u32,
    pub pane_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_session_id: Option<String>,
    #[serde(flatten)]
    pub command: BridgeCommand,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum BridgeCommand {
    OnboardingPropose {
        generation: String,
        body: String,
    },
    Reply {
        body: String,
        #[serde(default)]
        files: Vec<String>,
    },
    Send {
        to: String,
        body: String,
        #[serde(default)]
        files: Vec<String>,
    },
    Bots,
    Ask {
        body: String,
        #[serde(default)]
        files: Vec<String>,
        #[serde(default)]
        options: Vec<String>,
    },
    Result {
        body: String,
        #[serde(default)]
        files: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<String>,
    },
    BotShow {
        handle: String,
    },
    Skills,
    SkillCopy {
        slug: String,
    },
    SkillAdd {
        folder: String,
        #[serde(default)]
        recreate: bool,
    },
    BotCreate {
        name: String,
        color: String,
        #[serde(default)]
        provider: String,
        #[serde(default)]
        skills: Vec<String>,
        instructions: String,
        #[serde(default)]
        recreate: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    Tasks,
    TaskCreate {
        name: String,
        bot: String,
        every: u32,
        #[serde(default)]
        at: Option<String>,
        #[serde(default)]
        from: Option<String>,
        prompt: String,
        #[serde(default)]
        recreate: bool,
    },
    TaskEdit {
        task: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        bot: Option<String>,
        #[serde(default)]
        every: Option<u32>,
        #[serde(default)]
        at: Option<String>,
        #[serde(default)]
        from: Option<String>,
        #[serde(default)]
        enabled: Option<bool>,
        #[serde(default)]
        prompt: Option<String>,
    },
    TaskDelete {
        task: String,
    },
    Sessions {
        #[serde(default)]
        archived: bool,
        #[serde(default)]
        all: bool,
    },
    SessionShow {
        session: String,
    },
    SessionArchive {
        sessions: Vec<String>,
    },
    SkillUpdate {
        slug: String,
        folder: String,
    },
    BotEdit {
        handle: String,
        #[serde(default)]
        instructions: Option<String>,
        #[serde(default)]
        add_skills: Vec<String>,
        #[serde(default)]
        remove_skills: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    Workflows,
    WorkflowShow {
        name: String,
    },
    WorkflowRun {
        name: String,
        body: String,
        #[serde(default)]
        files: Vec<String>,
    },
    WorkflowCreate {
        definition: String,
        #[serde(default)]
        recreate: bool,
    },
    WorkflowEdit {
        name: String,
        definition: String,
    },
    WorkflowDelete {
        name: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotABot,
    AppNotRunning,
    UnknownRecipient,
    SelfSend,
    NothingToReplyTo,
    EmptyBody,
    FileNotFound,
    FileTooLarge,
    ChainLimitReached,
    UnsupportedVersion,
    NotImplemented,
    NotAllowed,
    NoOpenRequest,
    CreatedDifferently,
    CreatedThenDeleted,
    BadRequest,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BotSummary {
    pub handle: String,
    pub display_name: String,
    pub provider: String,
    pub status: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub waiting_for_user: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillSummary {
    pub slug: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub used_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BotDetail {
    pub handle: String,
    pub display_name: String,
    pub provider: String,
    pub status: String,
    pub role: String,
    pub instructions: String,
    pub skills: Vec<SkillSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    pub path: String,
    pub resumed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Created {
    pub name: String,
    pub already: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bots: Option<Vec<BotSummary>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot: Option<BotDetail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<SkillSummary>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drafts_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<Draft>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<Created>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<BridgeError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

impl BridgeResponse {
    fn empty() -> Self {
        Self {
            ok: true,
            message_id: None,
            to: None,
            bots: None,
            bot: None,
            skills: None,
            drafts_dir: None,
            draft: None,
            created: None,
            error: None,
            output: None,
        }
    }

    pub fn sent(message_id: impl Into<String>, to: impl Into<String>) -> Self {
        Self { message_id: Some(message_id.into()), to: Some(to.into()), ..Self::empty() }
    }

    pub fn bots(list: Vec<BotSummary>) -> Self {
        Self { bots: Some(list), ..Self::empty() }
    }

    pub fn bot(detail: BotDetail) -> Self {
        Self { bot: Some(detail), ..Self::empty() }
    }

    pub fn skills(list: Vec<SkillSummary>, drafts_dir: impl Into<String>) -> Self {
        Self { skills: Some(list), drafts_dir: Some(drafts_dir.into()), ..Self::empty() }
    }

    pub fn draft(path: impl Into<String>, resumed: bool) -> Self {
        Self { draft: Some(Draft { path: path.into(), resumed }), ..Self::empty() }
    }

    pub fn created(name: impl Into<String>, already: bool) -> Self {
        Self { created: Some(Created { name: name.into(), already }), ..Self::empty() }
    }

    pub fn output(text: impl Into<String>) -> Self {
        Self { output: Some(text.into()), ..Self::empty() }
    }

    pub fn error(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { ok: false, error: Some(BridgeError { code, message: message.into() }), ..Self::empty() }
    }
}
