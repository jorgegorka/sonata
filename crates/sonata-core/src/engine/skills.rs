use super::*;
use crate::skills::{self, SkillFile, MAX_DESCRIPTION_LEN, MAX_NAME_LEN, SKILL_FILE};
use crate::store::NewCreation;
use crate::CoreError;

const CLAIM_TRIES: usize = 8;

fn checked_skill(new: &NewSkill) -> Result<(String, String)> {
    let name = new.name.trim().to_string();
    if name.is_empty() {
        return Err(Refusal::SkillNameRequired.into());
    }
    let description = new.description.trim().to_string();
    if description.is_empty() {
        return Err(Refusal::SkillDescriptionRequired.into());
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(Refusal::SkillBadFrontmatter(format!("name is longer than {MAX_NAME_LEN} characters")).into());
    }
    if description.chars().count() > MAX_DESCRIPTION_LEN {
        let reason = format!("description is longer than {MAX_DESCRIPTION_LEN} characters");
        return Err(Refusal::SkillBadFrontmatter(reason).into());
    }
    Ok((name, description))
}

fn remove_claimed(slug: &str, dir: &Path) {
    if let Err(e) = std::fs::remove_dir_all(dir) {
        tracing::warn!(skill = %slug, error = %e, "could not remove the folder of a skill that failed");
    }
}

impl Engine {
    pub fn skills_dir(&self) -> PathBuf {
        self.inner.cfg.paths.skills_dir()
    }

    pub fn list_skills(&self) -> Result<Vec<SkillView>> {
        self.inner.store.list_skills()
    }

    fn taken_slugs(&self) -> Result<Vec<String>> {
        let mut taken = self.inner.store.skill_slugs()?;
        let entries = match std::fs::read_dir(self.skills_dir()) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(taken),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            if let Ok(name) = entry?.file_name().into_string() {
                taken.push(name.to_lowercase());
            }
        }
        Ok(taken)
    }

    fn claim_skill_dir(&self, name: &str) -> Result<(String, PathBuf)> {
        let root = self.skills_dir();
        std::fs::create_dir_all(&root)?;
        let mut taken = self.taken_slugs()?;
        let mut tries = 1;
        loop {
            let slug = skills::derive_slug(name, &taken);
            let dir = root.join(&slug);
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok((slug, dir)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && tries < CLAIM_TRIES => {
                    tries += 1;
                    taken.push(slug);
                }
                Err(e) => {
                    tracing::warn!(skill = %slug, error = %e, "could not make the skill's folder");
                    return Err(e.into());
                }
            }
        }
    }

    pub fn create_skill(&self, new: NewSkill) -> Result<Skill> {
        let (name, description) = checked_skill(&new)?;
        let (slug, dir) = self.claim_skill_dir(&name)?;
        let file =
            SkillFile { name: slug.clone(), description: description.clone(), other_lines: Vec::new(), body: new.body };
        let stored = std::fs::write(dir.join(SKILL_FILE), skills::render_skill_md(&file))
            .map_err(CoreError::from)
            .and_then(|()| self.inner.store.insert_skill(&slug, &name, &description, SkillSource::Written));
        match stored {
            Ok(skill) => {
                tracing::info!(skill = %slug, "skill created");
                Ok(skill)
            }
            Err(e) => {
                tracing::warn!(skill = %slug, error = %e, "could not create the skill");
                remove_claimed(&slug, &dir);
                Err(e)
            }
        }
    }

    pub fn import_skill(&self, folder: &Path) -> Result<Skill> {
        let file = skills::check_import(folder)?;
        self.import_skill_with(&file, &folder.display().to_string(), |dir| skills::copy_dir(folder, dir), None)
    }

    pub(super) fn import_skill_with(
        &self,
        file: &SkillFile,
        from: &str,
        fill: impl FnOnce(&Path) -> std::io::Result<()>,
        record: Option<&NewCreation>,
    ) -> Result<Skill> {
        let (slug, dir) = self.claim_skill_dir(&file.name)?;
        let renamed = SkillFile { name: slug.clone(), ..file.clone() };
        let store = &self.inner.store;
        let stored = fill(&dir)
            .and_then(|()| std::fs::write(dir.join(SKILL_FILE), skills::render_skill_md(&renamed)))
            .map_err(CoreError::from)
            .and_then(|()| match record {
                Some(record) => {
                    store.insert_skill_recorded(&slug, &file.name, &file.description, SkillSource::Imported, record)
                }
                None => store.insert_skill(&slug, &file.name, &file.description, SkillSource::Imported),
            });
        match stored {
            Ok(skill) => {
                tracing::info!(skill = %slug, from, "skill imported");
                Ok(skill)
            }
            Err(e) => {
                tracing::warn!(skill = %slug, from, error = %e, "could not import the skill");
                remove_claimed(&slug, &dir);
                Err(e)
            }
        }
    }

    pub(super) fn add_skill_at(
        &self,
        slug: &str,
        name: &str,
        description: &str,
        fill: impl FnOnce(&Path) -> std::io::Result<()>,
    ) -> Result<Skill> {
        let root = self.skills_dir();
        std::fs::create_dir_all(&root)?;
        let dir = root.join(slug);
        match std::fs::create_dir(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(Refusal::SkillFolderTaken(slug.to_string()).into())
            }
            Err(e) => return Err(e.into()),
        }
        let stored = fill(&dir)
            .map_err(CoreError::from)
            .and_then(|()| self.inner.store.insert_skill(slug, name, description, SkillSource::Imported));
        if stored.is_err() {
            remove_claimed(slug, &dir);
        }
        stored
    }

    pub fn skill_body(&self, skill_id: &str) -> Result<String> {
        Ok(self.read_skill_file(skill_id)?.1.body)
    }

    fn read_skill_file(&self, skill_id: &str) -> Result<(Skill, SkillFile, String)> {
        let skill =
            self.inner.store.get_skill(skill_id)?.ok_or_else(|| Refusal::SkillNotFound(skill_id.to_string()))?;
        let folder = self.skills_dir().join(&skill.slug);
        let text = match std::fs::read_to_string(folder.join(SKILL_FILE)) {
            Ok(text) => text,
            Err(e) => {
                tracing::warn!(skill = %skill.slug, error = %e, "could not read the skill's SKILL.md");
                if e.kind() == std::io::ErrorKind::NotFound {
                    return Err(Refusal::SkillNoFile(folder.to_string_lossy().into_owned()).into());
                }
                return Err(e.into());
            }
        };
        let file = skills::parse_skill_md(&text).map_err(Refusal::SkillBadFrontmatter)?;
        Ok((skill, file, text))
    }

    pub fn update_skill(&self, skill_id: &str, edit: NewSkill) -> Result<Skill> {
        let (name, description) = checked_skill(&edit)?;
        let (skill, file, text) = self.read_skill_file(skill_id)?;
        let updated = SkillFile { description: description.clone(), body: edit.body, ..file };
        let rendered = skills::render_skill_md(&updated);
        let file_changed = rendered != text;
        if file_changed {
            if let Err(e) = std::fs::write(self.skills_dir().join(&skill.slug).join(SKILL_FILE), rendered) {
                tracing::warn!(skill = %skill.slug, error = %e, "could not write the skill's SKILL.md");
                return Err(e.into());
            }
        }
        let changed = file_changed || skill.name != name;
        let skill = self
            .inner
            .store
            .update_skill(skill_id, &name, &description)?
            .ok_or_else(|| Refusal::SkillNotFound(skill_id.to_string()))?;
        if changed {
            let bots = self.inner.store.bump_bots_with_skill(skill_id)?;
            tracing::info!(skill = %skill.slug, bots = ?bots, "skill edited; its bots restart with their next message");
        }
        Ok(skill)
    }

    pub fn delete_skill(&self, skill_id: &str) -> Result<PathBuf> {
        let used_by = self.inner.store.bots_using_skill(skill_id)?;
        if !used_by.is_empty() {
            return Err(Refusal::SkillInUse(used_by).into());
        }
        let skill =
            self.inner.store.delete_skill(skill_id)?.ok_or_else(|| Refusal::SkillNotFound(skill_id.to_string()))?;
        tracing::info!(skill = %skill.slug, "skill deleted");
        Ok(self.skills_dir().join(skill.slug))
    }

    pub(super) fn checked_skill_ids(&self, ids: &[String]) -> Result<Vec<String>> {
        let mut ids = ids.to_vec();
        ids.sort();
        ids.dedup();
        for id in &ids {
            if self.inner.store.get_skill(id)?.is_none() {
                return Err(Refusal::SkillNotFound(id.clone()).into());
            }
        }
        Ok(ids)
    }
}
