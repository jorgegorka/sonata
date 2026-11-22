use std::io::Write;

use super::drafts::{draft_links, folder_fingerprint};
use super::master::{failed, on_users_turn};
use super::messaging::{internal, refuse};
use super::*;
use crate::bundled::{self, MASTER};
use crate::skills::{self, SkillFile, SKILL_FILE};
use bridge_proto::{BridgeResponse, ErrorCode};
use chrono::{DateTime, Local, Utc};

pub type Trash = Arc<dyn Fn(&Path) -> std::result::Result<(), String> + Send + Sync>;

const CHANGE_ONLY_ON_USERS_TURN: &str =
    "Change skills and bots only while answering the user's message, after they approved the change.";

pub(super) fn file_stamp(now: DateTime<Utc>) -> String {
    now.with_timezone(&Local).format("%Y%m%d-%H%M%S").to_string()
}

pub(super) fn free_path(dir: &Path, stem: &str, extension: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}{extension}"));
    let mut n = 2;
    while std::fs::symlink_metadata(&path).is_ok() {
        path = dir.join(format!("{stem}-{n}{extension}"));
        n += 1;
    }
    path
}

fn swap_in(library: &Path, staged: &Path, previous: &Path) -> std::io::Result<()> {
    std::fs::rename(library, previous)?;
    if let Err(e) = std::fs::rename(staged, library) {
        if let Err(back) = std::fs::rename(previous, library) {
            let kept = previous.display();
            return Err(std::io::Error::other(format!("{e}; the previous version stays at {kept}: {back}")));
        }
        return Err(e);
    }
    Ok(())
}

fn with_checkout(text: &str, paragraph: &str) -> Option<String> {
    let mut text = text.to_string();
    while let Some((range, _)) = bundled::find_checkout_paragraph(&text) {
        let (before, after) = (text[..range.start].trim_end(), text[range.end..].trim_start_matches('\n'));
        text = if before.is_empty() || after.is_empty() {
            format!("{before}{after}")
        } else {
            format!("{before}\n\n{after}")
        };
    }
    let template = bundled::checkout_paragraph(Some(Path::new("\0")));
    let head = template.split_once('\0').map_or(template.as_str(), |(head, _)| head);
    if text.contains(head) {
        return None;
    }
    let result = match text.trim_end() {
        "" => paragraph.to_string(),
        body => format!("{body}\n\n{paragraph}"),
    };
    let found = bundled::find_checkout_paragraph(&result).map(|(range, _)| range);
    (found == Some(result.len() - paragraph.len()..result.len())).then_some(result)
}

fn remove_leftover(path: &Path, master: &Bot, session: &Session) {
    let removed = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
    if let Err(e) = removed {
        tracing::warn!(bot = %master.handle, session = session.number, path = %path.display(), error = %e, "could not remove what a change left behind");
    }
}

pub(super) fn remove_leftover_of(path: &Path, skill: &str) {
    let removed = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
    if let Err(e) = removed {
        tracing::warn!(skill = %skill, path = %path.display(), error = %e, "could not remove what a change left behind");
    }
}

impl Engine {
    pub fn set_trash(&self, trash: Trash) {
        *self.inner.trash.write().unwrap() = Some(trash);
    }

    fn put_away(&self, path: &Path, master: &Bot, session: &Session) -> String {
        let trash = self.inner.trash.read().unwrap().clone();
        match trash.map(|trash| trash(path)) {
            Some(Ok(())) => return "The previous version is in the Trash.".to_string(),
            Some(Err(e)) => {
                tracing::warn!(bot = %master.handle, session = session.number, path = %path.display(), error = %e, "could not move the previous version to the Trash")
            }
            None => {}
        }
        format!("The previous version is kept at {}.", path.display())
    }

    pub(super) async fn bridge_skill_update(&self, pane_id: &str, slug: &str, folder: &str) -> BridgeResponse {
        let (caller, flight) = match self.master_caller(pane_id, "skill update").await {
            Ok(found) => found,
            Err(refusal) => return refusal,
        };
        if !on_users_turn(flight.as_ref()) {
            return refuse(ErrorCode::NotAllowed, CHANGE_ONLY_ON_USERS_TURN);
        }
        let (master, session) = match self.caller_parts(&caller) {
            Ok(parts) => parts,
            Err(refusal) => return refusal,
        };
        let slug = slug.trim();
        if slug == MASTER.skill_slug {
            return refuse(
                ErrorCode::NotAllowed,
                format!(
                    "The skill {slug} is your own playbook: only the user changes it, in the app. Ask them to make \
                     this change there."
                ),
            );
        }
        let skill = match self.store.get_skill_by_slug(slug) {
            Ok(Some(skill)) => skill,
            Ok(None) => {
                return refuse(
                    ErrorCode::BadRequest,
                    format!("No skill named {slug}. Run `sbot skills` to see the library."),
                )
            }
            Err(e) => return internal(e),
        };
        let Some((draft, _)) = self.own_draft(&master, &session, Path::new(folder)) else {
            let drafts = self.drafts_dir(&master, &session);
            return refuse(
                ErrorCode::BadRequest,
                format!("Update skills only from folders in this conversation's drafts: {}/<name>.", drafts.display()),
            );
        };
        match draft_links(&draft) {
            Ok(links) => {
                if let Some(link) = links.first() {
                    return refuse(
                        ErrorCode::BadRequest,
                        format!(
                            "The draft holds a link, which Sonata does not copy: {}. Replace it with the file itself.",
                            link.display()
                        ),
                    );
                }
            }
            Err(e) => return failed("read the draft", e),
        }
        let file = match skills::check_import(&draft) {
            Ok(file) => file,
            Err(CoreError::Refused(why)) => {
                return refuse(ErrorCode::BadRequest, format!("The draft cannot replace the skill: {why}."))
            }
            Err(e) => return failed("read the draft", e),
        };
        let (previous, bots) = match self.replace_skill(&skill, &draft, &file, &master, &session) {
            Ok(Some(replaced)) => replaced,
            Ok(None) => {
                tracing::info!(bot = %master.handle, session = session.number, skill = %skill.slug, "the skill already matches the draft");
                return BridgeResponse::output(format!("No change: {} already matches the draft.", skill.slug));
            }
            Err(e) => {
                tracing::warn!(bot = %master.handle, session = session.number, skill = %skill.slug, error = %e, "could not update the skill");
                return failed("update the skill", e);
            }
        };
        tracing::info!(bot = %master.handle, session = session.number, skill = %skill.slug, bots = ?bots, "Master changed a skill; its bots restart with their next message");
        let kept = self.put_away(&previous, &master, &session);
        let handles: Vec<String> = bots.iter().map(|handle| format!("@{handle}")).collect();
        let used_by = handles.join(", ");
        if handles.is_empty() {
            self.notice(&session.id, "master_changed_unused_skill", json!({"slug": skill.slug}));
        } else {
            self.notice(&session.id, "master_changed_skill", json!({"slug": skill.slug, "bots": used_by}));
        }
        self.emit(UiEvent::SkillsChanged);
        let reached = if handles.is_empty() {
            "No bot uses it.".to_string()
        } else {
            format!("Bots that use it: {used_by} (each starts with the new version at its next message).")
        };
        BridgeResponse::output(format!("Updated the skill {}. {reached} {kept}", skill.slug))
    }

    fn replace_skill(
        &self,
        skill: &Skill,
        draft: &Path,
        file: &SkillFile,
        master: &Bot,
        session: &Session,
    ) -> Result<Option<(PathBuf, Vec<String>)>> {
        let staged = self.staging_dir(&skill.slug)?;
        let renamed = SkillFile { name: skill.slug.clone(), ..file.clone() };
        let library = self.skills_dir().join(&skill.slug);
        let compared = skills::copy_dir(draft, &staged)
            .and_then(|()| std::fs::write(staged.join(SKILL_FILE), skills::render_skill_md(&renamed)))
            .and_then(|()| Ok(folder_fingerprint(&staged)? == folder_fingerprint(&library)?));
        match compared {
            Ok(false) => {}
            Ok(true) => {
                remove_leftover(&staged, master, session);
                return Ok(None);
            }
            Err(e) => {
                remove_leftover(&staged, master, session);
                return Err(e.into());
            }
        }
        let name = if file.name == skill.slug { skill.name.clone() } else { file.name.clone() };
        self.install_staged(skill, &staged, &name, &file.description).map(Some)
    }

    pub(super) fn staging_dir(&self, slug: &str) -> Result<PathBuf> {
        let dir = self.inner.cfg.paths.previous_dir();
        std::fs::create_dir_all(&dir)?;
        let staged = free_path(&dir, &format!("{}-new-{slug}", file_stamp(self.now_utc())), "");
        std::fs::create_dir(&staged)?;
        Ok(staged)
    }

    pub(super) fn install_staged(
        &self,
        skill: &Skill,
        staged: &Path,
        name: &str,
        description: &str,
    ) -> Result<(PathBuf, Vec<String>)> {
        let dir = self.inner.cfg.paths.previous_dir();
        let library = self.skills_dir().join(&skill.slug);
        let previous = free_path(&dir, &format!("{}-skill-{}", file_stamp(self.now_utc()), skill.slug), "");
        if let Err(e) = swap_in(&library, staged, &previous) {
            if library.exists() {
                remove_leftover_of(staged, &skill.slug);
            }
            return Err(e.into());
        }
        let store = &self.store;
        let stored = store
            .update_skill(&skill.id, name, description)
            .and_then(|updated| updated.ok_or_else(|| Refusal::SkillNotFound(skill.id.clone()).into()))
            .and_then(|_| store.bump_bots_with_skill(&skill.id));
        match stored {
            Ok(bots) => Ok((previous, bots)),
            Err(e) => {
                if let Err(back) = store.update_skill(&skill.id, &skill.name, &skill.description) {
                    tracing::error!(skill = %skill.slug, error = %back, "could not restore the skill's name and description");
                }
                match swap_in(&library, &previous, staged) {
                    Ok(()) => remove_leftover_of(staged, &skill.slug),
                    Err(back) => {
                        tracing::error!(skill = %skill.slug, error = %back, "could not put the previous version of the skill back")
                    }
                }
                Err(e)
            }
        }
    }

    pub(super) async fn bridge_bot_edit(
        &self,
        pane_id: &str,
        handle: &str,
        instructions: Option<&str>,
        add_skills: &[String],
        remove_skills: &[String],
        model: Option<&str>,
    ) -> BridgeResponse {
        let (caller, flight) = match self.master_caller(pane_id, "bot edit").await {
            Ok(found) => found,
            Err(refusal) => return refusal,
        };
        if !on_users_turn(flight.as_ref()) {
            return refuse(ErrorCode::NotAllowed, CHANGE_ONLY_ON_USERS_TURN);
        }
        let (master, session) = match self.caller_parts(&caller) {
            Ok(parts) => parts,
            Err(refusal) => return refusal,
        };
        let target = match self.bot_named(handle) {
            Ok(target) => target,
            Err(refusal) => return refusal,
        };
        match self.is_master(&target.id) {
            Ok(false) => {}
            Ok(true) => {
                return refuse(
                    ErrorCode::NotAllowed,
                    format!(
                        "@{} is you: only the user changes your instructions and skills, in the app. Ask them to make \
                         this change there.",
                        target.handle
                    ),
                )
            }
            Err(e) => return internal(e),
        }
        let store = &self.store;
        let mut skill_ids = target.skill_ids.clone();
        for (slug, add) in add_skills.iter().map(|s| (s, true)).chain(remove_skills.iter().map(|s| (s, false))) {
            let slug = slug.trim();
            let skill = match store.get_skill_by_slug(slug) {
                Ok(Some(skill)) => skill,
                Ok(None) => {
                    return refuse(
                        ErrorCode::BadRequest,
                        format!("No skill named {slug}. Run `sbot skills` to see the library."),
                    )
                }
                Err(e) => return internal(e),
            };
            skill_ids.retain(|id| id != &skill.id);
            if add {
                skill_ids.push(skill.id);
            }
        }
        let instructions =
            match instructions {
                Some(text) if bundled::find_checkout_paragraph(&target.instructions).is_some() => {
                    let checkout = match self.project_folder() {
                        Ok(checkout) => checkout,
                        Err(e) => return internal(e),
                    };
                    match with_checkout(text, &bundled::checkout_paragraph(checkout.as_deref())) {
                        Some(instructions) => instructions,
                        None => return refuse(
                            ErrorCode::BadRequest,
                            "The instructions hold a line about the project folder that is not the app's paragraph. \
                             Leave the project folder paragraph out of the instructions; the app adds the current one.",
                        ),
                    }
                }
                Some(text) => text.trim_end().to_string(),
                None => target.instructions.clone(),
            };
        let model = match model {
            Some(model) => Some(model.trim()).filter(|model| !model.is_empty()).map(String::from),
            None => target.model.clone(),
        };
        let (mut wanted, mut had) = (skill_ids.clone(), target.skill_ids.clone());
        wanted.sort();
        had.sort();
        if instructions == target.instructions && wanted == had && model == target.model {
            tracing::info!(bot = %master.handle, session = session.number, edited = %target.handle, "the bot already has these instructions and skills");
            return BridgeResponse::output(format!(
                "No change: @{} already has these instructions and skills.",
                target.handle
            ));
        }
        let previous = match self.keep_bot(&target) {
            Ok(previous) => previous,
            Err(e) => {
                tracing::warn!(bot = %master.handle, session = session.number, edited = %target.handle, error = %e, "could not keep the bot's previous version");
                return failed("keep the bot's previous version", e);
            }
        };
        let edit = NewBot {
            display_name: target.display_name.clone(),
            color: target.color.clone(),
            provider: target.provider,
            model,
            instructions,
            skill_ids,
        };
        let bot = match self.update_bot(&target.id, edit) {
            Ok(bot) => bot,
            Err(e) => {
                tracing::warn!(bot = %master.handle, session = session.number, edited = %target.handle, error = %e, "could not change the bot");
                remove_leftover(&previous, &master, &session);
                return match e {
                    CoreError::Refused(why) => refuse(ErrorCode::BadRequest, why.to_string()),
                    other => failed("change the bot", other),
                };
            }
        };
        tracing::info!(bot = %master.handle, session = session.number, edited = %bot.handle, "Master changed a bot");
        let kept = self.put_away(&previous, &master, &session);
        self.notice(&session.id, "master_changed_bot", json!({"handle": bot.handle}));
        let reply =
            format!("Updated @{}. Its sessions start with the new version at their next message. {kept}", bot.handle);
        let status = bot_status(&*self.inner.live.lock().await, &bot.id);
        self.emit(UiEvent::BotUpdated { bot: BotView { bot, status } });
        BridgeResponse::output(reply)
    }

    fn keep_bot(&self, bot: &Bot) -> Result<PathBuf> {
        let dir = self.inner.cfg.paths.previous_dir();
        std::fs::create_dir_all(&dir)?;
        let now = self.now_utc();
        let slugs: Vec<String> = self.store.skills_of(&bot.id)?.into_iter().map(|s| s.slug).collect();
        let skills = if slugs.is_empty() { "none".to_string() } else { slugs.join(", ") };
        let model = bot.model.as_deref().map(|model| format!("Model: {model}\n")).unwrap_or_default();
        let text = format!(
            "# @{} before {}\n\nSkills: {skills}\n{model}\n{}\n",
            bot.handle,
            local_minute(now, &Local),
            bot.instructions.trim_end()
        );
        let path = free_path(&dir, &format!("{}-bot-{}", file_stamp(now), bot.handle), ".md");
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .and_then(|mut file| file.write_all(text.as_bytes()));
        if let Err(e) = written {
            if path.exists() {
                let _ = std::fs::remove_file(&path);
            }
            return Err(e.into());
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(path: &Path, text: &str) {
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(path.join("SKILL.md"), text).unwrap();
    }

    #[test]
    fn a_swap_puts_the_new_folder_in_place_and_the_old_one_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let (library, staged, previous) = (tmp.path().join("lib"), tmp.path().join("new"), tmp.path().join("old"));
        folder(&library, "old");
        folder(&staged, "new");
        swap_in(&library, &staged, &previous).unwrap();
        assert_eq!(std::fs::read_to_string(library.join("SKILL.md")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(previous.join("SKILL.md")).unwrap(), "old");
        assert!(!staged.exists());
        swap_in(&library, &previous, &staged).unwrap();
        assert_eq!(std::fs::read_to_string(library.join("SKILL.md")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(staged.join("SKILL.md")).unwrap(), "new");
    }

    #[test]
    fn a_swap_that_cannot_put_the_new_folder_in_place_puts_the_old_one_back() {
        let tmp = tempfile::tempdir().unwrap();
        let (library, previous) = (tmp.path().join("lib"), tmp.path().join("old"));
        folder(&library, "old");
        let missing = tmp.path().join("new");
        assert_eq!(swap_in(&library, &missing, &previous).unwrap_err().kind(), std::io::ErrorKind::NotFound);
        assert_eq!(std::fs::read_to_string(library.join("SKILL.md")).unwrap(), "old");
        assert!(!previous.exists());
    }

    #[test]
    fn previous_versions_never_share_a_name() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(free_path(tmp.path(), "x-bot-a", ".md"), tmp.path().join("x-bot-a.md"));
        std::fs::write(tmp.path().join("x-bot-a.md"), "").unwrap();
        std::fs::write(tmp.path().join("x-bot-a-2.md"), "").unwrap();
        assert_eq!(free_path(tmp.path(), "x-bot-a", ".md"), tmp.path().join("x-bot-a-3.md"));
        std::fs::create_dir(tmp.path().join("x-skill-a")).unwrap();
        assert_eq!(free_path(tmp.path(), "x-skill-a", ""), tmp.path().join("x-skill-a-2"));
    }

    #[test]
    fn the_checkout_paragraph_is_the_current_one_once_at_the_end() {
        let current = bundled::checkout_paragraph(Some(Path::new("/new")));
        let old = bundled::checkout_paragraph(Some(Path::new("/old")));
        let with = |text: &str| with_checkout(text, &current).unwrap();
        assert_eq!(with("Body.\n"), format!("Body.\n\n{current}"));
        assert_eq!(with(&format!("Body.\n\n{old}\n")), format!("Body.\n\n{current}"));
        assert_eq!(with(&format!("A.\n\n{old}\n\nB.\n\n{old}")), format!("A.\n\nB.\n\n{current}"));
        let unset = bundled::checkout_paragraph(None);
        assert_eq!(with(&format!("{unset}\n\nBody.")), format!("Body.\n\n{current}"));
        assert_eq!(with(&old), current);
        assert_eq!(with_checkout(&format!("Body.\n\n{current}"), &unset).unwrap(), format!("Body.\n\n{unset}"));
    }

    #[test]
    fn a_checkout_line_the_app_cannot_replace_is_refused() {
        let current = bundled::checkout_paragraph(Some(Path::new("/new")));
        let unset = bundled::checkout_paragraph(None);
        let old = bundled::checkout_paragraph(Some(Path::new("/old")));
        for text in [
            "Runs queries. The project folder is at `/Users/me/sonata`, use it.".to_string(),
            format!("The project folder is at `/elsewhere`.\n\n{old}"),
            format!("{old}\n\nThe project folder is at `/elsewhere`."),
        ] {
            assert_eq!(with_checkout(&text, &current), None, "{text}");
            assert_eq!(with_checkout(&text, &unset), None, "{text}");
        }
    }
}
