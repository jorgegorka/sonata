use serde_json::{json, Value};

use crate::attachments::AttachmentError;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Refusal {
    #[error("display name is required")]
    NameRequired,
    #[error("provider {0} is not available yet")]
    ProviderUnavailable(String),
    #[error("OpenCode models are written provider/model, not {0}")]
    ModelFormat(String),
    #[error("message is empty")]
    MessageEmpty,
    #[error(transparent)]
    Attachment(#[from] AttachmentError),
    #[error("bot {0} not found")]
    BotNotFound(String),
    #[error("session {0} not found")]
    SessionNotFound(String),
    #[error("the session is starting")]
    SessionStarting,
    #[error("session {0} is archived")]
    SessionArchived(String),
    #[error("message {0} not found")]
    MessageNotFound(String),
    #[error("attachment {index} of message {message} not found")]
    AttachmentNotFound { message: String, index: usize },
    #[error("the message is already being delivered")]
    AlreadyDelivering,
    #[error("only a queued message can be cancelled")]
    NotQueued,
    #[error("only an interrupted message can be retried")]
    NotInterrupted,
    #[error("the bot is not working on a message")]
    NotWorking,
    #[error("could not stop the bot: {0}")]
    StopFailed(String),
    #[error("the bot is not waiting for an answer")]
    NotWaiting,
    #[error("the bot is not running")]
    NotRunning,
    #[error("could not send the answer: {0}")]
    AnswerFailed(String),
    #[error("could not read the bot's terminal: {0}")]
    SnapshotFailed(String),
    #[error("the bot is starting; try again in a moment")]
    BotStarting,
    #[error("database was created by a newer version of Sonata")]
    DatabaseTooNew,
    #[error("the hop limit must be from {min} to {max}")]
    HopLimitRange { min: i64, max: i64 },
    #[error("the bots folder can only be chosen before the first bot exists")]
    BotsFolderLocked,
    #[error("{0} is not a folder Sonata can use")]
    BadFolder(String),
    #[error("the bot's folder {0} leads outside the bots folder")]
    BotFolderOutside(String),
    #[error("the skill needs a name")]
    SkillNameRequired,
    #[error("the skill needs a description")]
    SkillDescriptionRequired,
    #[error("skill {0} not found")]
    SkillNotFound(String),
    #[error("the skill is used by {}", .0.join(", "))]
    SkillInUse(Vec<String>),
    #[error("{0} has no SKILL.md")]
    SkillNoFile(String),
    #[error("SKILL.md frontmatter: {0}")]
    SkillBadFrontmatter(String),
    #[error("the skill folder is over {mb} MB")]
    SkillTooLarge { mb: u64 },
    #[error("the task needs a name")]
    TaskNameRequired,
    #[error("the task name is longer than {max} characters")]
    TaskNameTooLong { max: usize },
    #[error("there is already a task named {0}")]
    TaskNameTaken(String),
    #[error("the task needs a prompt")]
    TaskPromptRequired,
    #[error("a task repeats every {min} to {max} days")]
    TaskEveryRange { min: u32, max: u32 },
    #[error("{0} is not a time of day")]
    TaskTimeInvalid(String),
    #[error("{0} is not a date")]
    TaskStartInvalid(String),
    #[error("task {0} not found")]
    TaskNotFound(String),
    #[error("the workflow needs a name")]
    WorkflowNameRequired,
    #[error("the workflow name is longer than {max} characters")]
    WorkflowNameTooLong { max: usize },
    #[error("there is already a workflow named {0}")]
    WorkflowNameTaken(String),
    #[error("the workflow's \"when to use\" is longer than {max} characters")]
    WorkflowWhenTooLong { max: usize },
    #[error("the workflow has no steps")]
    WorkflowNoSteps,
    #[error("a workflow has at most {max} steps")]
    WorkflowTooManySteps { max: usize },
    #[error("a step needs a name")]
    WorkflowStepNameRequired,
    #[error("the name of step {step} is longer than {max} characters")]
    WorkflowStepNameTooLong { step: String, max: usize },
    #[error("a step cannot be named Done")]
    WorkflowStepNameReserved,
    #[error("there is already a step named {0}")]
    WorkflowStepNameTaken(String),
    #[error("step {step} has no bot")]
    WorkflowStepNoBot { step: String },
    #[error("step {step} has no instructions")]
    WorkflowStepInstructionsRequired { step: String },
    #[error("the workflow does not say where it starts")]
    WorkflowNoStart,
    #[error("an arrow out of step {step} points nowhere")]
    WorkflowArrowTarget { step: String },
    #[error("every arrow out of step {step} needs an outcome")]
    WorkflowOutcomeRequired { step: String },
    #[error("an outcome of step {step} is longer than {max} characters")]
    WorkflowOutcomeTooLong { step: String, max: usize },
    #[error("an outcome of step {step} has more than one line")]
    WorkflowOutcomeLine { step: String },
    #[error("step {step} has the outcome {outcome} twice")]
    WorkflowOutcomeTaken { step: String, outcome: String },
    #[error("step {step} has more than {max} arrows")]
    WorkflowTooManyArrows { step: String, max: usize },
    #[error("step {step} cannot be reached from the start")]
    WorkflowUnreachable { step: String },
    #[error("the workflow definition is malformed")]
    WorkflowBadDefinition,
    #[error("workflow {0} not found")]
    WorkflowNotFound(String),
    #[error("workflow run {0} not found")]
    WorkflowRunNotFound(String),
    #[error("the workflow run is not paused")]
    WorkflowRunNotPaused,
    #[error("{0} is not an outcome of the step")]
    WorkflowOutcomeUnknown(String),
    #[error("Master is gone, so no bot can design a workflow from a description")]
    MasterMissing,
    #[error("the file is over {mb} MB")]
    ConfigFileTooLarge { mb: u64 },
    #[error("line {line}: {reason}")]
    ConfigFileNotYaml { line: usize, reason: String },
    #[error("the file is not a Sonata configuration")]
    ConfigFileWrongFormat,
    #[error("the file has format {version}, newer than this Sonata reads")]
    ConfigFileTooNew { version: u32 },
    #[error("the file has two Master bots")]
    ConfigFileTwoMasters,
    #[error("the file has {0} twice")]
    ConfigFileRepeats(String),
    #[error("the export is over {mb} MB")]
    ConfigExportTooLarge { mb: u64 },
    #[error("the backup of what the import replaces is over {mb} MB")]
    ConfigBackupTooLarge { mb: u64 },
    #[error("{0} is not a valid skill name")]
    SkillSlugInvalid(String),
    #[error("the skill has an invalid file path {0}")]
    SkillPathInvalid(String),
    #[error("the skill has the file {0} twice")]
    SkillPathRepeated(String),
    #[error("{0} is both a file and a folder in the skill")]
    SkillPathConflict(String),
    #[error("the skill file {0} has no readable content")]
    SkillFileContent(String),
    #[error("the skill's SKILL.md does not name {0}")]
    SkillNameNotSlug(String),
    #[error("the folder {0} is already in the skills folder")]
    SkillFolderTaken(String),
    #[error("{0} is not a bot colour")]
    BotColorInvalid(String),
    #[error("unknown provider {0}")]
    ProviderUnknown(String),
    #[error("the skill {0} is neither in the file nor in the library")]
    SkillUnknown(String),
    #[error("the import needs {0}, which can't be imported")]
    ImportDependsOn(String),
    #[error("language {0} is not available")]
    LanguageUnknown(String),
}

impl Refusal {
    pub fn code(&self) -> &'static str {
        match self {
            Refusal::NameRequired => "nameRequired",
            Refusal::ProviderUnavailable(_) => "providerUnavailable",
            Refusal::ModelFormat(_) => "modelFormat",
            Refusal::MessageEmpty => "messageEmpty",
            Refusal::Attachment(AttachmentError::NotFound(_)) => "fileNotFound",
            Refusal::Attachment(AttachmentError::NotAFile(_)) => "fileIsFolder",
            Refusal::Attachment(AttachmentError::TooLarge { .. }) => "fileTooLarge",
            Refusal::Attachment(AttachmentError::Copy { .. }) => "fileCopyFailed",
            Refusal::BotNotFound(_) => "botNotFound",
            Refusal::SessionNotFound(_) => "sessionNotFound",
            Refusal::SessionStarting => "sessionStarting",
            Refusal::SessionArchived(_) => "sessionArchived",
            Refusal::MessageNotFound(_) => "messageNotFound",
            Refusal::AttachmentNotFound { .. } => "attachmentNotFound",
            Refusal::AlreadyDelivering => "alreadyDelivering",
            Refusal::NotQueued => "notQueued",
            Refusal::NotInterrupted => "notInterrupted",
            Refusal::NotWorking => "notWorking",
            Refusal::StopFailed(_) => "stopFailed",
            Refusal::NotWaiting => "notWaiting",
            Refusal::NotRunning => "notRunning",
            Refusal::AnswerFailed(_) => "answerFailed",
            Refusal::SnapshotFailed(_) => "snapshotFailed",
            Refusal::BotStarting => "botStarting",
            Refusal::DatabaseTooNew => "databaseTooNew",
            Refusal::HopLimitRange { .. } => "hopLimitRange",
            Refusal::BotsFolderLocked => "botsFolderLocked",
            Refusal::BadFolder(_) => "badFolder",
            Refusal::BotFolderOutside(_) => "botFolderOutside",
            Refusal::SkillNameRequired => "skillNameRequired",
            Refusal::SkillDescriptionRequired => "skillDescriptionRequired",
            Refusal::SkillNotFound(_) => "skillNotFound",
            Refusal::SkillInUse(_) => "skillInUse",
            Refusal::SkillNoFile(_) => "skillNoFile",
            Refusal::SkillBadFrontmatter(_) => "skillBadFrontmatter",
            Refusal::SkillTooLarge { .. } => "skillTooLarge",
            Refusal::TaskNameRequired => "taskNameRequired",
            Refusal::TaskNameTooLong { .. } => "taskNameTooLong",
            Refusal::TaskNameTaken(_) => "taskNameTaken",
            Refusal::TaskPromptRequired => "taskPromptRequired",
            Refusal::TaskEveryRange { .. } => "taskEveryRange",
            Refusal::TaskTimeInvalid(_) => "taskTimeInvalid",
            Refusal::TaskStartInvalid(_) => "taskStartInvalid",
            Refusal::TaskNotFound(_) => "taskNotFound",
            Refusal::WorkflowNameRequired => "workflowNameRequired",
            Refusal::WorkflowNameTooLong { .. } => "workflowNameTooLong",
            Refusal::WorkflowNameTaken(_) => "workflowNameTaken",
            Refusal::WorkflowWhenTooLong { .. } => "workflowWhenTooLong",
            Refusal::WorkflowNoSteps => "workflowNoSteps",
            Refusal::WorkflowTooManySteps { .. } => "workflowTooManySteps",
            Refusal::WorkflowStepNameRequired => "workflowStepNameRequired",
            Refusal::WorkflowStepNameTooLong { .. } => "workflowStepNameTooLong",
            Refusal::WorkflowStepNameReserved => "workflowStepNameReserved",
            Refusal::WorkflowStepNameTaken(_) => "workflowStepNameTaken",
            Refusal::WorkflowStepNoBot { .. } => "workflowStepNoBot",
            Refusal::WorkflowStepInstructionsRequired { .. } => "workflowStepInstructionsRequired",
            Refusal::WorkflowNoStart => "workflowNoStart",
            Refusal::WorkflowArrowTarget { .. } => "workflowArrowTarget",
            Refusal::WorkflowOutcomeRequired { .. } => "workflowOutcomeRequired",
            Refusal::WorkflowOutcomeTooLong { .. } => "workflowOutcomeTooLong",
            Refusal::WorkflowOutcomeLine { .. } => "workflowOutcomeLine",
            Refusal::WorkflowOutcomeTaken { .. } => "workflowOutcomeTaken",
            Refusal::WorkflowTooManyArrows { .. } => "workflowTooManyArrows",
            Refusal::WorkflowUnreachable { .. } => "workflowUnreachable",
            Refusal::WorkflowBadDefinition => "workflowBadDefinition",
            Refusal::WorkflowNotFound(_) => "workflowNotFound",
            Refusal::WorkflowRunNotFound(_) => "workflowRunNotFound",
            Refusal::WorkflowRunNotPaused => "workflowRunNotPaused",
            Refusal::WorkflowOutcomeUnknown(_) => "workflowOutcomeUnknown",
            Refusal::MasterMissing => "masterMissing",
            Refusal::ConfigFileTooLarge { .. } => "configFileTooLarge",
            Refusal::ConfigFileNotYaml { .. } => "configFileNotYaml",
            Refusal::ConfigFileWrongFormat => "configFileWrongFormat",
            Refusal::ConfigFileTooNew { .. } => "configFileTooNew",
            Refusal::ConfigFileTwoMasters => "configFileTwoMasters",
            Refusal::ConfigFileRepeats(_) => "configFileRepeats",
            Refusal::ConfigExportTooLarge { .. } => "configExportTooLarge",
            Refusal::ConfigBackupTooLarge { .. } => "configBackupTooLarge",
            Refusal::SkillSlugInvalid(_) => "skillSlugInvalid",
            Refusal::SkillPathInvalid(_) => "skillPathInvalid",
            Refusal::SkillPathRepeated(_) => "skillPathRepeated",
            Refusal::SkillPathConflict(_) => "skillPathConflict",
            Refusal::SkillFileContent(_) => "skillFileContent",
            Refusal::SkillNameNotSlug(_) => "skillNameNotSlug",
            Refusal::SkillFolderTaken(_) => "skillFolderTaken",
            Refusal::BotColorInvalid(_) => "botColorInvalid",
            Refusal::ProviderUnknown(_) => "providerUnknown",
            Refusal::SkillUnknown(_) => "skillUnknown",
            Refusal::ImportDependsOn(_) => "importDependsOn",
            Refusal::LanguageUnknown(_) => "languageUnknown",
        }
    }

    pub fn params(&self) -> Value {
        match self {
            Refusal::ProviderUnavailable(provider) => json!({ "provider": provider }),
            Refusal::ModelFormat(model) => json!({ "model": model }),
            Refusal::Attachment(AttachmentError::NotFound(path) | AttachmentError::NotAFile(path)) => {
                json!({ "path": path })
            }
            Refusal::Attachment(AttachmentError::TooLarge { path, mb }) => {
                json!({ "path": path, "mb": mb.to_string() })
            }
            Refusal::Attachment(AttachmentError::Copy { path, error }) => json!({ "path": path, "error": error }),
            Refusal::StopFailed(error) | Refusal::AnswerFailed(error) | Refusal::SnapshotFailed(error) => {
                json!({ "error": error })
            }
            Refusal::HopLimitRange { min, max } => json!({ "min": min.to_string(), "max": max.to_string() }),
            Refusal::BadFolder(path) | Refusal::BotFolderOutside(path) => json!({ "path": path }),
            Refusal::SkillInUse(handles) => {
                let bots: Vec<String> = handles.iter().map(|h| format!("@{h}")).collect();
                json!({ "bots": bots.join(", ") })
            }
            Refusal::SkillNoFile(path) => json!({ "path": path }),
            Refusal::SkillBadFrontmatter(reason) => json!({ "reason": reason }),
            Refusal::SkillTooLarge { mb } => json!({ "mb": mb.to_string() }),
            Refusal::LanguageUnknown(code) => json!({ "language": code }),
            Refusal::TaskNameTooLong { max } => json!({ "max": max.to_string() }),
            Refusal::TaskNameTaken(name) => json!({ "name": name }),
            Refusal::TaskEveryRange { min, max } => json!({ "min": min.to_string(), "max": max.to_string() }),
            Refusal::TaskTimeInvalid(time) => json!({ "time": time }),
            Refusal::TaskStartInvalid(date) => json!({ "date": date }),
            Refusal::WorkflowNameTooLong { max } => json!({ "max": max.to_string() }),
            Refusal::WorkflowNameTaken(name) | Refusal::WorkflowStepNameTaken(name) => json!({ "name": name }),
            Refusal::WorkflowWhenTooLong { max } => json!({ "max": max.to_string() }),
            Refusal::WorkflowTooManySteps { max } => json!({ "max": max.to_string() }),
            Refusal::WorkflowStepNameTooLong { step, max }
            | Refusal::WorkflowOutcomeTooLong { step, max }
            | Refusal::WorkflowTooManyArrows { step, max } => {
                json!({ "step": step, "max": max.to_string() })
            }
            Refusal::WorkflowStepNoBot { step }
            | Refusal::WorkflowStepInstructionsRequired { step }
            | Refusal::WorkflowArrowTarget { step }
            | Refusal::WorkflowOutcomeRequired { step }
            | Refusal::WorkflowOutcomeLine { step }
            | Refusal::WorkflowUnreachable { step } => json!({ "step": step }),
            Refusal::WorkflowOutcomeTaken { step, outcome } => json!({ "step": step, "outcome": outcome }),
            Refusal::WorkflowOutcomeUnknown(outcome) => json!({ "outcome": outcome }),
            Refusal::ConfigFileTooLarge { mb } => json!({ "mb": mb.to_string() }),
            Refusal::ConfigFileNotYaml { line, reason } => json!({ "line": line.to_string(), "reason": reason }),
            Refusal::ConfigFileTooNew { version } => json!({ "version": version.to_string() }),
            Refusal::ConfigFileRepeats(name) => json!({ "name": name }),
            Refusal::ConfigExportTooLarge { mb } => json!({ "mb": mb.to_string() }),
            Refusal::ConfigBackupTooLarge { mb } => json!({ "mb": mb.to_string() }),
            Refusal::SkillSlugInvalid(slug) => json!({ "slug": slug }),
            Refusal::SkillPathInvalid(path) => json!({ "path": path }),
            Refusal::SkillPathRepeated(path) => json!({ "path": path }),
            Refusal::SkillPathConflict(path) => json!({ "path": path }),
            Refusal::SkillFileContent(path) => json!({ "path": path }),
            Refusal::SkillNameNotSlug(slug) => json!({ "slug": slug }),
            Refusal::SkillFolderTaken(slug) => json!({ "slug": slug }),
            Refusal::BotColorInvalid(color) => json!({ "color": color }),
            Refusal::ProviderUnknown(provider) => json!({ "provider": provider }),
            Refusal::SkillUnknown(slug) => json!({ "slug": slug }),
            Refusal::ImportDependsOn(name) => json!({ "name": name }),
            _ => json!({}),
        }
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn examples() -> Vec<Refusal> {
        vec![
            Refusal::NameRequired,
            Refusal::ProviderUnavailable("cursor".into()),
            Refusal::ModelFormat("sonnet".into()),
            Refusal::MessageEmpty,
            Refusal::Attachment(AttachmentError::NotFound("/a.md".into())),
            Refusal::Attachment(AttachmentError::NotAFile("/a".into())),
            Refusal::Attachment(AttachmentError::TooLarge { path: "/a.pdf".into(), mb: 120 }),
            Refusal::Attachment(AttachmentError::Copy { path: "/a.md".into(), error: "denied".into() }),
            Refusal::BotNotFound("01JBOT".into()),
            Refusal::SessionNotFound("01JSES".into()),
            Refusal::SessionStarting,
            Refusal::SessionArchived("01JSES".into()),
            Refusal::MessageNotFound("01JMSG".into()),
            Refusal::AttachmentNotFound { message: "01JMSG".into(), index: 1 },
            Refusal::AlreadyDelivering,
            Refusal::NotQueued,
            Refusal::NotInterrupted,
            Refusal::NotWorking,
            Refusal::StopFailed("x".into()),
            Refusal::NotWaiting,
            Refusal::NotRunning,
            Refusal::AnswerFailed("x".into()),
            Refusal::SnapshotFailed("x".into()),
            Refusal::BotStarting,
            Refusal::DatabaseTooNew,
            Refusal::HopLimitRange { min: 1, max: 100 },
            Refusal::BotsFolderLocked,
            Refusal::BadFolder("relative".into()),
            Refusal::BotFolderOutside("/a".into()),
            Refusal::SkillNameRequired,
            Refusal::SkillDescriptionRequired,
            Refusal::SkillNotFound("01JSKILL".into()),
            Refusal::SkillInUse(vec!["researcher".into(), "writer".into()]),
            Refusal::SkillNoFile("/a/skill".into()),
            Refusal::SkillBadFrontmatter("no description".into()),
            Refusal::SkillTooLarge { mb: 50 },
            Refusal::TaskNameRequired,
            Refusal::TaskNameTooLong { max: 40 },
            Refusal::TaskNameTaken("Wiki update".into()),
            Refusal::TaskPromptRequired,
            Refusal::TaskEveryRange { min: 1, max: 365 },
            Refusal::TaskTimeInvalid("25:00".into()),
            Refusal::TaskStartInvalid("2027-13-01".into()),
            Refusal::TaskNotFound("01JTASK".into()),
            Refusal::LanguageUnknown("pt".into()),
            Refusal::WorkflowNameRequired,
            Refusal::WorkflowNameTooLong { max: 40 },
            Refusal::WorkflowNameTaken("Ticket to merge request".into()),
            Refusal::WorkflowWhenTooLong { max: 300 },
            Refusal::WorkflowNoSteps,
            Refusal::WorkflowTooManySteps { max: 30 },
            Refusal::WorkflowStepNameRequired,
            Refusal::WorkflowStepNameTooLong { step: "Review".into(), max: 40 },
            Refusal::WorkflowStepNameReserved,
            Refusal::WorkflowStepNameTaken("Review".into()),
            Refusal::WorkflowStepNoBot { step: "Review".into() },
            Refusal::WorkflowStepInstructionsRequired { step: "Review".into() },
            Refusal::WorkflowNoStart,
            Refusal::WorkflowArrowTarget { step: "Review".into() },
            Refusal::WorkflowOutcomeRequired { step: "Review".into() },
            Refusal::WorkflowOutcomeTooLong { step: "Review".into(), max: 60 },
            Refusal::WorkflowOutcomeLine { step: "Review".into() },
            Refusal::WorkflowOutcomeTaken { step: "Review".into(), outcome: "approved".into() },
            Refusal::WorkflowTooManyArrows { step: "Review".into(), max: 6 },
            Refusal::WorkflowUnreachable { step: "Test".into() },
            Refusal::WorkflowBadDefinition,
            Refusal::WorkflowNotFound("01JWORKFLOW".into()),
            Refusal::WorkflowRunNotFound("01JRUN".into()),
            Refusal::WorkflowRunNotPaused,
            Refusal::WorkflowOutcomeUnknown("maybe".into()),
            Refusal::MasterMissing,
            Refusal::ConfigFileTooLarge { mb: 256 },
            Refusal::ConfigFileNotYaml { line: 3, reason: "mapping values are not allowed here".into() },
            Refusal::ConfigFileWrongFormat,
            Refusal::ConfigFileTooNew { version: 2 },
            Refusal::ConfigFileTwoMasters,
            Refusal::ConfigFileRepeats("@research".into()),
            Refusal::ConfigExportTooLarge { mb: 256 },
            Refusal::ConfigBackupTooLarge { mb: 256 },
            Refusal::SkillSlugInvalid("My Skill".into()),
            Refusal::SkillPathInvalid("../x".into()),
            Refusal::SkillPathRepeated("notes.md".into()),
            Refusal::SkillPathConflict("docs".into()),
            Refusal::SkillFileContent("notes.md".into()),
            Refusal::SkillNameNotSlug("review".into()),
            Refusal::SkillFolderTaken("review".into()),
            Refusal::BotColorInvalid("red".into()),
            Refusal::ProviderUnknown("gemini".into()),
            Refusal::SkillUnknown("review".into()),
            Refusal::ImportDependsOn("@research".into()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_its_own_code() {
        let mut codes: Vec<&str> = Refusal::examples().iter().map(Refusal::code).collect();
        let all = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), all, "two kinds share a code");
    }

    #[test]
    fn params_are_objects_of_strings() {
        for refusal in Refusal::examples() {
            let params = refusal.params();
            let object = params.as_object().unwrap_or_else(|| panic!("{} params are not an object", refusal.code()));
            assert!(object.values().all(Value::is_string), "{}: {params}", refusal.code());
        }
        assert_eq!(
            Refusal::Attachment(AttachmentError::TooLarge { path: "/a.pdf".into(), mb: 120 }).params(),
            json!({ "path": "/a.pdf", "mb": "120" })
        );
        assert_eq!(
            Refusal::SkillInUse(vec!["researcher".into(), "writer".into()]).params(),
            json!({ "bots": "@researcher, @writer" })
        );
    }

    #[test]
    fn the_english_text_stays_for_logs() {
        assert_eq!(Refusal::NotInterrupted.to_string(), "only an interrupted message can be retried");
        assert_eq!(
            Refusal::Attachment(AttachmentError::NotFound("/x".into())).to_string(),
            "/x does not exist",
            "attachment refusals read like the bridge's"
        );
    }
}
