#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("project not found")]
    NotFound,
    #[error("restore this project before changing it or running work")]
    Archived,
    #[error("a project needs a name")]
    NameRequired,
    #[error("a project with this name already exists, including archived projects")]
    NameTaken,
    #[error("choose a source directory for the project")]
    DirectoryRequired,
    #[error("this item belongs to another project")]
    Mismatch,
    #[error("finish or stop active bot work before archiving this project")]
    ActiveWork,
    #[error("finish or stop queued work and unfinished workflows before archiving this project")]
    QueuedWork,
    #[error("finish background work before archiving; if its status is unavailable, stop the bot first")]
    BackgroundWork,
}
impl ProjectError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "projectNotFound",
            Self::Archived => "projectArchived",
            Self::NameRequired => "projectNameRequired",
            Self::NameTaken => "projectNameTaken",
            Self::DirectoryRequired => "projectDirectoryRequired",
            Self::Mismatch => "projectMismatch",
            Self::ActiveWork => "projectActiveWork",
            Self::QueuedWork => "projectQueuedWork",
            Self::BackgroundWork => "projectBackgroundWork",
        }
    }
}
