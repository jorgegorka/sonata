pub mod attachments;
pub mod bridge;
pub mod bundled;
pub mod config_file;
pub mod context;
pub mod detect;
pub mod engine;
pub mod envelope;
pub mod handle;
pub mod ids;
pub mod limits;
pub mod model;
pub mod paths;
mod project_error;
pub mod provider;
pub use project_error::ProjectError;
pub mod refusal;
pub mod runtime;
pub mod schedule;
pub mod skills;
pub mod store;
pub mod workflow;

pub use refusal::Refusal;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Refused(#[from] Refusal),
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;
