use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::{Child, Command};

use crate::{HerdrClient, HerdrError};

pub fn session_socket_path(session: &str) -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".config/herdr/sessions").join(session).join("herdr.sock")
}

pub fn server_config_path(session: &str) -> PathBuf {
    dirs::cache_dir().or_else(dirs::home_dir).unwrap_or_default().join("sonata/herdr").join(format!("{session}.toml"))
}

pub fn find_herdr_bin() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
    herdr_bin_from(std::env::var_os("SONATA_HERDR_BIN"), exe_dir.as_deref())
}

fn herdr_bin_from(var: Option<std::ffi::OsString>, exe_dir: Option<&Path>) -> Option<PathBuf> {
    match var {
        Some(var) => Some(PathBuf::from(var)).filter(|path| path.is_file()),
        None => exe_dir.map(|dir| dir.join("herdr")).filter(|path| path.is_file()),
    }
}

const SERVER_CONFIG: &str =
    "[server]\nheadless_cols = 120\nheadless_rows = 40\n\n[session]\nresume_agents_on_restore = false\n";

fn is_agent_session_marker(key: &str) -> bool {
    matches!(
        key,
        "CLAUDECODE"
            | "AI_AGENT"
            | "CLAUDE_PID"
            | "CLAUDE_EFFORT"
            | "CODEX_THREAD_ID"
            | "OPENCODE"
            | "OPENCODE_SESSION_ID"
            | "OPENCODE_TERMINAL"
    ) || key.starts_with("CLAUDE_CODE_")
}

pub struct HerdrServer {
    client: HerdrClient,
    child: Option<Child>,
    config: PathBuf,
}

impl HerdrServer {
    pub async fn start(bin: &Path, session: &str) -> Result<Self, HerdrError> {
        Self::start_with_env(bin, session, &[]).await
    }

    pub async fn start_with_env(bin: &Path, session: &str, env: &[(String, String)]) -> Result<Self, HerdrError> {
        let client = HerdrClient::new(session_socket_path(session));
        let config = server_config_path(session);
        if client.ping().await.is_ok() {
            return Ok(Self { client, child: None, config });
        }
        if let Some(dir) = config.parent() {
            std::fs::create_dir_all(dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
            }
        }
        std::fs::write(&config, SERVER_CONFIG)?;
        let mut cmd = Command::new(bin);
        crate::host_command_env(cmd.as_std_mut());
        cmd.args(["--session", session, "server"])
            .env("HERDR_CONFIG_PATH", &config)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (key, _) in std::env::vars_os() {
            if is_agent_session_marker(&key.to_string_lossy()) {
                cmd.env_remove(&key);
            }
        }
        for (key, value) in env {
            cmd.env(key, value);
        }
        let mut child = cmd.spawn()?;
        for _ in 0..100 {
            if client.ping().await.is_ok() {
                return Ok(Self { client, child: Some(child), config });
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let _ = child.kill().await;
        let _ = std::fs::remove_file(&config);
        Err(HerdrError::Protocol(format!("herdr session {session} did not start within 10s")))
    }

    pub fn client(&self) -> &HerdrClient {
        &self.client
    }

    pub async fn stop(mut self) -> Result<(), HerdrError> {
        let client = self.client.clone().with_timeout(Duration::from_secs(5));
        let _ = client.server_stop().await;
        if let Some(mut child) = self.child.take() {
            if tokio::time::timeout(Duration::from_secs(5), child.wait()).await.is_err() {
                let _ = child.kill().await;
            }
        }
        let _ = std::fs::remove_file(&self.config);
        for _ in 0..50 {
            if client.ping().await.is_err() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(HerdrError::Protocol("herdr server still running after stop".into()))
    }
}

pub async fn delete_session(bin: &Path, session: &str) -> Result<(), HerdrError> {
    let status = Command::new(bin)
        .args(["session", "delete", session])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await?;
    if status.success() {
        Ok(())
    } else {
        Err(HerdrError::Protocol(format!("herdr session delete {session} failed")))
    }
}

#[cfg(test)]
mod find_tests {
    use super::*;

    fn file(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "").unwrap();
        path
    }

    #[test]
    fn the_variable_wins_over_the_bundled_herdr() {
        let tmp = tempfile::tempdir().unwrap();
        let pinned = file(tmp.path(), "pinned-herdr");
        file(tmp.path(), "herdr");
        assert_eq!(herdr_bin_from(Some(pinned.clone().into()), Some(tmp.path())), Some(pinned));
    }

    #[test]
    fn a_variable_that_names_no_file_finds_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        file(tmp.path(), "herdr");
        assert_eq!(herdr_bin_from(Some(tmp.path().join("missing").into()), Some(tmp.path())), None);
    }

    #[test]
    fn without_the_variable_only_the_herdr_next_to_the_executable_counts() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(herdr_bin_from(None, Some(tmp.path())), None, "no PATH lookup");
        let bundled = file(tmp.path(), "herdr");
        assert_eq!(herdr_bin_from(None, Some(tmp.path())), Some(bundled));
    }
}

#[cfg(test)]
mod marker_tests {
    use super::*;

    #[test]
    fn agent_session_markers_stay_out_of_the_panes() {
        for key in ["CLAUDECODE", "CLAUDE_CODE_SESSION_ID", "AI_AGENT", "CODEX_THREAD_ID"] {
            assert!(is_agent_session_marker(key), "{key}");
        }
        assert!(!is_agent_session_marker("CODEX_HOME"), "the user's codex home must reach codex");
        assert!(!is_agent_session_marker("PATH"));
        for key in ["OPENCODE", "OPENCODE_SESSION_ID", "OPENCODE_TERMINAL"] {
            assert!(is_agent_session_marker(key), "{key}");
        }
        assert!(!is_agent_session_marker("OPENCODE_CONFIG"), "the user's opencode config must reach opencode");
        assert!(!is_agent_session_marker("AGENT"), "too generic a name to take from the user's environment");
    }
}
