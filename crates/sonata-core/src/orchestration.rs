use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::model::{Attachment, WorkflowDef};
use crate::{CoreError, Result};

pub fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::Orchestration(message.into())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Executor {
    Bot { handle: String },
    Workflow { name: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSpec {
    pub key: String,
    pub title: String,
    pub instructions: String,
    pub acceptance_criteria: Vec<String>,
    pub executor: Executor,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub title: String,
    pub goal: String,
    pub tasks: Vec<TaskSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub task: String,
    pub succeeded: bool,
    pub explanation: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    #[serde(default)]
    pub add: Vec<TaskSpec>,
    #[serde(default)]
    pub replace: Vec<TaskSpec>,
    #[serde(default)]
    pub reviews: Vec<Review>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Waiting,
    Running,
    AwaitingReview,
    Succeeded,
    Failed,
    Blocked,
    Cancelled,
}

impl TaskState {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Blocked | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Active,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Assignment {
    Bot { bot_id: String },
    Workflow { workflow_id: String, name: String, definition: WorkflowDef },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub spec: TaskSpec,
    pub assignment: Assignment,
    pub state: TaskState,
    pub session_id: Option<String>,
    pub workflow_run_id: Option<String>,
    pub request_message_id: Option<String>,
    pub result_message_id: Option<String>,
    pub result: Option<String>,
    pub attachments: Vec<Attachment>,
    pub review: Option<String>,
    pub archived_at: Option<String>,
}

impl Task {
    pub fn new(spec: TaskSpec, assignment: Assignment) -> Self {
        Self {
            spec,
            assignment,
            state: TaskState::Waiting,
            session_id: None,
            workflow_run_id: None,
            request_message_id: None,
            result_message_id: None,
            result: None,
            attachments: vec![],
            review: None,
            archived_at: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Orchestration {
    pub id: String,
    pub project_id: String,
    pub session_id: String,
    pub request_message_id: String,
    pub title: String,
    pub goal: String,
    pub revision: i64,
    pub status: Status,
    pub tasks: Vec<Task>,
    pub final_message_id: Option<String>,
    pub final_report: Option<String>,
    pub final_files: Vec<Attachment>,
    pub created_at: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskView {
    #[serde(flatten)]
    pub task: Task,
    pub session_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrchestrationView {
    #[serde(flatten)]
    pub run: Orchestration,
    pub executions: Vec<TaskView>,
}

pub fn validate_graph(tasks: &[Task]) -> Result<()> {
    if tasks.is_empty() {
        return Err(invalid("An orchestration needs at least one task."));
    }
    let mut by_key = HashMap::new();
    for task in tasks {
        let s = &task.spec;
        if s.key.is_empty()
            || s.key.len() > 80
            || !s.key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err(invalid("Task keys use 1–80 letters, digits, hyphens or underscores."));
        }
        if by_key.insert(s.key.as_str(), task).is_some() {
            return Err(invalid(format!("Duplicate task key: {}.", s.key)));
        }
        if s.title.trim().is_empty()
            || s.instructions.trim().is_empty()
            || s.acceptance_criteria.is_empty()
            || s.acceptance_criteria.iter().any(|c| c.trim().is_empty())
        {
            return Err(invalid(format!("Task {} needs a title, instructions and acceptance criteria.", s.key)));
        }
    }
    for task in tasks {
        let mut seen = HashSet::new();
        for dep in &task.spec.depends_on {
            if !by_key.contains_key(dep.as_str()) || !seen.insert(dep) {
                return Err(invalid(format!("Task {} has an unknown or duplicate dependency: {dep}.", task.spec.key)));
            }
        }
    }
    let mut visited = HashSet::new();
    loop {
        let before = visited.len();
        for task in tasks {
            if task.spec.depends_on.iter().all(|d| visited.contains(d)) {
                visited.insert(task.spec.key.clone());
            }
        }
        if visited.len() == tasks.len() {
            return Ok(());
        }
        if before == visited.len() {
            return Err(invalid("Task dependencies contain a cycle."));
        }
    }
}

pub fn block_dependents(run: &mut Orchestration) {
    loop {
        let failed: HashSet<_> = run
            .tasks
            .iter()
            .filter(|t| t.state.terminal() && t.state != TaskState::Succeeded)
            .map(|t| t.spec.key.clone())
            .collect();
        let mut changed = false;
        for task in &mut run.tasks {
            if task.state == TaskState::Waiting {
                let causes: Vec<_> = task.spec.depends_on.iter().filter(|d| failed.contains(*d)).cloned().collect();
                if !causes.is_empty() {
                    task.state = TaskState::Blocked;
                    task.review = Some(format!("Prerequisites did not succeed: {}.", causes.join(", ")));
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

pub fn ready(run: &Orchestration, task: &Task) -> bool {
    run.status == Status::Active
        && task.state == TaskState::Waiting
        && task
            .spec
            .depends_on
            .iter()
            .all(|key| run.tasks.iter().any(|t| t.spec.key == *key && t.state == TaskState::Succeeded))
}
