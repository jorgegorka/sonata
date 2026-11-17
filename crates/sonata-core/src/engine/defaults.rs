use super::*;
use crate::bundled;

pub(super) const CHECKOUT_SETTING: &str = "project_folder";

impl Engine {
    pub fn seed_default_bots(&self, project: Option<&Path>) -> Result<Vec<Bot>> {
        Ok(self.ensure_master(project)?.into_iter().collect())
    }

    pub fn ensure_default_bots(&self) -> Result<Vec<Bot>> {
        if !self.setup_done()? {
            return Ok(Vec::new());
        }
        self.seed_default_bots(self.project_folder()?.as_deref())
    }

    pub fn project_folder(&self) -> Result<Option<PathBuf>> {
        Ok(self.inner.store.get_setting(CHECKOUT_SETTING)?.map(PathBuf::from))
    }

    pub fn set_project_folder(&self, checkout: &Path) -> Result<Vec<String>> {
        if !checkout.is_absolute() || !checkout.is_dir() {
            return Err(Refusal::BadFolder(checkout.to_string_lossy().into_owned()).into());
        }
        self.inner.store.set_setting(CHECKOUT_SETTING, &checkout.to_string_lossy())?;
        let paragraph = bundled::checkout_paragraph(Some(checkout));
        let mut kept = Vec::new();
        for bot in self.inner.store.list_bots()? {
            let Some((range, _)) = bundled::find_checkout_paragraph(&bot.instructions) else {
                if self.is_master(&bot.id)? {
                    kept.push(bot.handle);
                }
                continue;
            };
            if bot.instructions[range.clone()] == paragraph {
                continue;
            }
            let mut instructions = bot.instructions;
            instructions.replace_range(range, &paragraph);
            let edit = NewBot {
                display_name: bot.display_name,
                color: bot.color,
                provider: bot.provider,
                model: bot.model,
                instructions,
                skill_ids: bot.skill_ids,
            };
            self.update_bot(&bot.id, edit)?;
        }
        tracing::info!(checkout = %checkout.display(), kept = ?kept, "project folder changed");
        Ok(kept)
    }
}
