use super::*;
use crate::runtime::RuntimeHealth;

pub const MAX_HOP_LIMIT: i64 = 100;
pub const LANGUAGES: [&str; 4] = ["en", "es", "fr", "de"];
pub const SYSTEM_LANGUAGE: &str = "system";

pub(crate) enum FolderError {
    Io(std::io::Error),
    Outside(PathBuf),
}

impl Engine {
    pub fn hop_limit(&self) -> Result<i64> {
        let stored = self.store.get_setting("hop_limit")?.and_then(|v| v.trim().parse::<i64>().ok());
        Ok(stored.unwrap_or(DEFAULT_HOP_LIMIT).clamp(1, MAX_HOP_LIMIT))
    }

    pub fn set_hop_limit(&self, limit: i64) -> Result<()> {
        if !(1..=MAX_HOP_LIMIT).contains(&limit) {
            return Err(Refusal::HopLimitRange { min: 1, max: MAX_HOP_LIMIT }.into());
        }
        self.store.set_setting("hop_limit", &limit.to_string())?;
        tracing::info!(limit, "hop limit changed");
        Ok(())
    }

    pub fn language_choice(&self) -> Result<String> {
        let stored = self.store.get_setting("language")?;
        Ok(stored.filter(|v| LANGUAGES.contains(&v.as_str())).unwrap_or_else(|| SYSTEM_LANGUAGE.to_string()))
    }

    pub fn set_language(&self, choice: &str) -> Result<()> {
        if choice != SYSTEM_LANGUAGE && !LANGUAGES.contains(&choice) {
            return Err(Refusal::LanguageUnknown(choice.to_string()).into());
        }
        self.store.set_setting("language", choice)?;
        tracing::info!(language = choice, "language chosen");
        Ok(())
    }

    pub fn set_system_language(&self, code: &str) {
        let code = if LANGUAGES.contains(&code) { code } else { "en" };
        *self.inner.system_language.write().unwrap() = code.to_string();
    }

    pub fn system_language(&self) -> String {
        self.inner.system_language.read().unwrap().clone()
    }

    pub fn language(&self) -> Result<String> {
        let choice = self.language_choice()?;
        Ok(if choice == SYSTEM_LANGUAGE { self.inner.system_language.read().unwrap().clone() } else { choice })
    }

    pub fn setup_done(&self) -> Result<bool> {
        Ok(self.store.get_setting("setup_done")?.as_deref() == Some("true"))
    }

    pub fn finish_setup(&self, project_folder: Option<&Path>) -> Result<()> {
        if let Some(path) = project_folder {
            if !path.is_absolute() || !path.is_dir() {
                return Err(Refusal::BadFolder(path.to_string_lossy().into_owned()).into());
            }
        }
        if !self.setup_done()? {
            self.seed_default_bots(project_folder)?;
            if let Some(path) = project_folder {
                self.store.set_setting(super::defaults::CHECKOUT_SETTING, &path.to_string_lossy())?;
            }
        }
        self.store.set_setting("setup_done", "true")?;
        tracing::info!("first-run setup finished");
        Ok(())
    }

    pub fn close_hint_shown(&self) -> Result<bool> {
        Ok(self.store.get_setting("close_hint_shown")?.as_deref() == Some("true"))
    }

    pub fn mark_close_hint_shown(&self) -> Result<()> {
        self.store.set_setting("close_hint_shown", "true")
    }

    pub fn has_bots(&self) -> Result<bool> {
        Ok(!self.store.bot_handles()?.is_empty())
    }

    pub fn paths(&self) -> AppPaths {
        AppPaths { bots_root: self.bots_root(), ..self.inner.cfg.paths.clone() }
    }

    pub fn set_bots_root(&self, root: &Path) -> Result<()> {
        if !root.is_absolute() {
            return Err(Refusal::BadFolder(root.to_string_lossy().into_owned()).into());
        }
        if self.has_bots()? {
            return Err(Refusal::BotsFolderLocked.into());
        }
        std::fs::create_dir_all(root)?;
        let probe = root.join(".sonata-write-check");
        if std::fs::write(&probe, b"").and_then(|()| std::fs::remove_file(&probe)).is_err() {
            return Err(Refusal::BadFolder(root.to_string_lossy().into_owned()).into());
        }
        self.store.set_setting("bots_root", &root.to_string_lossy())?;
        *self.inner.bots_root.write().unwrap() = root.to_path_buf();
        tracing::info!(bots_root = %root.display(), "bots folder chosen");
        Ok(())
    }

    pub fn bot_folder(&self, bot_id: &str) -> Result<PathBuf> {
        let bot = self.store.get_bot(bot_id)?.ok_or_else(|| Refusal::BotNotFound(bot_id.into()))?;
        Ok(self.bot_directory(&bot))
    }

    pub async fn runtime_health(&self) -> std::result::Result<RuntimeHealth, RuntimeError> {
        self.inner.runtime.health().await
    }

    pub(crate) fn bots_root(&self) -> PathBuf {
        self.inner.bots_root.read().unwrap().clone()
    }

    pub(crate) fn checked_bot_dir(&self, handle: &str) -> std::result::Result<PathBuf, FolderError> {
        let dir = self.bot_dir(handle);
        std::fs::create_dir_all(&dir).map_err(FolderError::Io)?;
        if folder_inside(&dir, &self.bots_root()) {
            Ok(dir)
        } else {
            Err(FolderError::Outside(dir))
        }
    }

    pub(crate) fn bot_dir(&self, handle: &str) -> PathBuf {
        self.project_workspace().join(handle)
    }
}
