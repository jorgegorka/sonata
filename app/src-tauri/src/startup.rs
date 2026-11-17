use sonata_core::{CoreError, Refusal};

use crate::locale::t;

#[derive(Debug, Clone, PartialEq)]
pub struct StartupError {
    pub key: &'static str,
    pub vars: Vec<(&'static str, String)>,
    pub detail: String,
}

impl StartupError {
    pub fn new(key: &'static str, vars: Vec<(&'static str, String)>, detail: impl Into<String>) -> Self {
        Self { key, vars, detail: detail.into() }
    }

    pub fn message(&self) -> String {
        let vars: Vec<(&str, &str)> = self.vars.iter().map(|(name, value)| (*name, value.as_str())).collect();
        t(&format!("startup.{}", self.key), &vars)
    }
}

pub fn lock_error(e: std::io::Error) -> StartupError {
    if e.kind() == std::io::ErrorKind::WouldBlock {
        StartupError::new("alreadyRunning", vec![], e.to_string())
    } else {
        StartupError::new("lock", vec![("error", e.to_string())], e.to_string())
    }
}

pub fn store_error(e: CoreError) -> StartupError {
    match &e {
        CoreError::Refused(Refusal::DatabaseTooNew) => StartupError::new("databaseTooNew", vec![], e.to_string()),
        _ => StartupError::new("database", vec![("error", e.to_string())], e.to_string()),
    }
}

pub fn exit_with(error: StartupError) -> ! {
    let message = error.message();
    if tracing::dispatcher::has_been_set() {
        tracing::error!(key = error.key, detail = %error.detail, "cannot start: {message}");
    } else {
        eprintln!("Sonata cannot start: {message} ({})", error.detail);
    }
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(t("startup.title", &[]))
        .set_description(message)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
    std::process::exit(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_translated_and_filled_in() {
        let e = StartupError::new("bridge", vec![("path", "/tmp/x.sock".into()), ("error", "denied".into())], "");
        assert_eq!(e.message(), "Sonata could not open /tmp/x.sock, which bots use to send messages: denied");
        let keys = [
            "folders",
            "logging",
            "lock",
            "alreadyRunning",
            "database",
            "databaseTooNew",
            "herdrMissing",
            "sbotMissing",
            "bridge",
        ];
        for key in keys {
            assert_ne!(
                StartupError::new(key, vec![], "").message(),
                format!("startup.{key}"),
                "en.json has no startup.{key}"
            );
        }
        assert_eq!(t("startup.title", &[]), "Sonata cannot start");
    }

    #[test]
    fn a_second_instance_is_told_the_app_is_running() {
        let held = std::io::Error::new(std::io::ErrorKind::WouldBlock, "held");
        assert_eq!(lock_error(held).key, "alreadyRunning");
        assert_eq!(lock_error(std::io::Error::other("disk")).key, "lock");
    }

    #[test]
    fn a_database_from_a_newer_version_has_its_own_text() {
        assert_eq!(store_error(Refusal::DatabaseTooNew.into()).key, "databaseTooNew");
        assert_eq!(store_error(CoreError::Io(std::io::Error::other("x"))).key, "database");
    }
}
