use std::path::Path;

use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::EnvFilter;

pub const KEEP_FILES: usize = 7;

pub fn file_appender(logs_dir: &Path) -> Result<RollingFileAppender, String> {
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("sonata")
        .filename_suffix("log")
        .max_log_files(KEEP_FILES)
        .build(logs_dir)
        .map_err(|e| format!("cannot open the log folder {}: {e}", logs_dir.display()))
}

pub fn init(logs_dir: &Path) -> Result<(), String> {
    let file = file_appender(logs_dir)?;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(file.and(std::io::stderr))
        .try_init()
        .map_err(|e| format!("cannot start logging: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_land_in_a_dated_file_in_the_logs_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let subscriber =
            tracing_subscriber::fmt().with_ansi(false).with_writer(file_appender(tmp.path()).unwrap()).finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(bot = %"researcher", message_id = %"01JMSG", "prompt failed");
        });
        let files: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().map(|e| e.unwrap().path()).collect();
        assert_eq!(files.len(), 1, "{files:?}");
        let name = files[0].file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("sonata.") && name.ends_with(".log"), "{name}");
        let text = std::fs::read_to_string(&files[0]).unwrap();
        assert!(
            text.contains("prompt failed") && text.contains("bot=researcher") && text.contains("message_id=01JMSG")
        );
    }
}
