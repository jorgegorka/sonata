//! User-facing units of work. Recurring schedules remain `model::Task`.
use crate::orchestration::{Executor, Task as Execution};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Stable graph key for a dependency on an existing sibling, including an earlier workflow step.
pub(crate) fn dependency_key(id: &str) -> String {
    format!("wi_{:x}", Sha256::digest(id.as_bytes()))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkItem {
    pub id: String,
    pub project_id: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub instructions: String,
    pub state: String,
    pub outcome: Option<String>,
    pub primary_session_id: Option<String>,
    pub session_ids: Vec<String>,
    pub workflow_run_id: Option<String>,
    pub orchestration_id: Option<String>,
    pub execution: Option<Execution>,
    pub manual_start: bool,
    pub created_at: String,
    pub finished_at: Option<String>,
    pub archived_at: Option<String>,
    pub result_message_id: Option<String>,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewWorkItem {
    pub parent_id: Option<String>,
    pub title: String,
    pub instructions: String,
    pub executor: Executor,
    #[serde(default)]
    pub manual_start: bool,
    #[serde(default)]
    pub depends_on: Vec<String>,
}
