use super::drafts::{draft_links, drafts_relative, folder_fingerprint};
use super::messaging::{internal, not_a_bot, refuse, role_line, status_word, Caller};
use super::settings::FolderError;
use super::*;
use crate::bundled::{self, MASTER};
use crate::context::real_dir_chain;
use crate::skills;
use crate::store::{CreationKind, NewCreation};
use bridge_proto::{BotDetail, BridgeResponse, ErrorCode, SkillSummary};
use sha2::{Digest, Sha256};

pub(crate) const MASTER_SETTING: &str = "sonata_master";

pub(super) fn on_users_turn(flight: Option<&InFlight>) -> bool {
    flight.is_some_and(|f| f.sender == USER && !f.follow_up && !f.scheduled && !f.from_run)
}

fn bot_fingerprint(
    name: &str,
    color: &str,
    provider: &str,
    skills: &[String],
    instructions: &str,
    model: Option<&str>,
) -> String {
    let mut skills = skills.to_vec();
    skills.sort();
    let mut hash = Sha256::new();
    for field in [name, color, provider, &skills.join("\0"), instructions].into_iter().chain(model) {
        hash.update(field.as_bytes());
        hash.update([0]);
    }
    format!("{:x}", hash.finalize())
}

pub(super) fn is_hex_color(color: &str) -> bool {
    color.len() == 7 && color.starts_with('#') && color[1..].chars().all(|c| c.is_ascii_hexdigit())
}

pub(super) fn failed(what: &str, e: impl std::fmt::Display) -> BridgeResponse {
    refuse(ErrorCode::Internal, format!("Could not {what}: {e}"))
}

impl Engine {
    pub fn master_id(&self) -> Result<Option<String>> {
        self.inner.store.get_setting(MASTER_SETTING)
    }

    pub fn master_bot(&self) -> Result<Option<Bot>> {
        match self.master_id()? {
            Some(id) => self.inner.store.get_bot(&id),
            None => Ok(None),
        }
    }

    pub(crate) fn is_master(&self, bot_id: &str) -> Result<bool> {
        Ok(self.master_id()?.as_deref() == Some(bot_id))
    }

    pub(super) fn ensure_master(&self, checkout: Option<&Path>) -> Result<Option<Bot>> {
        if self.master_id()?.is_some() {
            return Ok(None);
        }
        let skill = match self.inner.store.get_skill_by_slug(MASTER.skill_slug)? {
            Some(skill) => skill,
            None => self.create_skill(NewSkill {
                name: MASTER.skill_slug.to_string(),
                description: MASTER.skill_description.to_string(),
                body: bundled::master_skill_body(),
            })?,
        };
        let new = NewBot {
            display_name: MASTER.display_name.to_string(),
            color: MASTER.color.to_string(),
            provider: Provider::Claude,
            model: None,
            instructions: bundled::master_instructions(checkout),
            skill_ids: vec![skill.id],
        };
        let store = &self.inner.store;
        let bot = self.create_bot_with(new, None, Some(MASTER.handle), |new, handle| {
            store.insert_bot_recorded_as(new, handle, MASTER_SETTING)
        })?;
        tracing::info!(bot = %bot.handle, "Master is in place");
        Ok(Some(bot))
    }

    #[allow(clippy::result_large_err)]
    pub(super) async fn master_caller(
        &self,
        pane_id: &str,
        command: &str,
    ) -> std::result::Result<(Caller, Option<InFlight>), BridgeResponse> {
        let Some((caller, flight)) = self.caller(pane_id).await else { return Err(not_a_bot()) };
        match self.is_master(&caller.bot_id) {
            Ok(true) => Ok((caller, flight)),
            Ok(false) => {
                let who = match self.master_bot() {
                    Ok(Some(master)) => format!("@{}", master.handle),
                    _ => "Master".to_string(),
                };
                Err(refuse(ErrorCode::NotAllowed, format!("Only {who} can use `sbot {command}`.")))
            }
            Err(e) => Err(internal(e)),
        }
    }

    #[allow(clippy::result_large_err)]
    pub(super) fn caller_parts(&self, caller: &Caller) -> std::result::Result<(Bot, Session), BridgeResponse> {
        let store = &self.inner.store;
        match (store.get_bot(&caller.bot_id), store.get_session(&caller.session_id)) {
            (Ok(Some(bot)), Ok(Some(session))) => Ok((bot, session)),
            (Err(e), _) | (_, Err(e)) => Err(internal(e)),
            _ => Err(not_a_bot()),
        }
    }

    pub fn drafts_dir(&self, bot: &Bot, session: &Session) -> PathBuf {
        self.bot_dir(&bot.handle).join(drafts_relative(session))
    }

    pub(super) async fn bridge_bot_show(&self, pane_id: &str, handle: &str) -> BridgeResponse {
        if let Err(refusal) = self.master_caller(pane_id, "bot show").await {
            return refusal;
        }
        let name = handle.trim().trim_start_matches('@').to_lowercase();
        let store = &self.inner.store;
        let bot = match store.get_bot_by_handle(&name) {
            Ok(Some(bot)) => bot,
            Ok(None) => {
                return refuse(
                    ErrorCode::UnknownRecipient,
                    format!("No bot named @{name}. Run `sbot bots` to see your teammates."),
                )
            }
            Err(e) => return internal(e),
        };
        let used_by = match self.list_skills() {
            Ok(skills) => skills,
            Err(e) => return internal(e),
        };
        let skills = match store.skills_of(&bot.id) {
            Ok(skills) => skills,
            Err(e) => return internal(e),
        };
        let skills = skills
            .into_iter()
            .map(|s| SkillSummary {
                used_by: used_by.iter().find(|v| v.skill.id == s.id).map(|v| v.used_by.clone()).unwrap_or_default(),
                slug: s.slug,
                name: s.name,
                description: s.description,
            })
            .collect();
        let status = match self.list_bots().await {
            Ok(bots) => bots.into_iter().find(|v| v.bot.id == bot.id).map(|v| v.status),
            Err(e) => return internal(e),
        };
        BridgeResponse::bot(BotDetail {
            role: role_line(&bot.instructions),
            handle: bot.handle,
            display_name: bot.display_name,
            provider: bot.provider.as_str().to_string(),
            status: status_word(status.unwrap_or(BotStatus::Offline)).to_string(),
            instructions: bot.instructions,
            skills,
            model: bot.model,
        })
    }

    pub(super) async fn bridge_skills(&self, pane_id: &str) -> BridgeResponse {
        let (bot, session) = match self.master_caller(pane_id, "skills").await {
            Ok((caller, _)) => match self.caller_parts(&caller) {
                Ok(parts) => parts,
                Err(refusal) => return refusal,
            },
            Err(refusal) => return refusal,
        };
        let drafts = self.drafts_dir(&bot, &session).to_string_lossy().into_owned();
        match self.list_skills() {
            Ok(skills) => BridgeResponse::skills(
                skills
                    .into_iter()
                    .map(|v| SkillSummary {
                        slug: v.skill.slug,
                        name: v.skill.name,
                        description: v.skill.description,
                        used_by: v.used_by,
                    })
                    .collect(),
                drafts,
            ),
            Err(e) => internal(e),
        }
    }

    pub(super) async fn bridge_skill_copy(&self, pane_id: &str, slug: &str) -> BridgeResponse {
        let (bot, session) = match self.master_caller(pane_id, "skill copy").await {
            Ok((caller, _)) => match self.caller_parts(&caller) {
                Ok(parts) => parts,
                Err(refusal) => return refusal,
            },
            Err(refusal) => return refusal,
        };
        let slug = slug.trim();
        let skill = match self.inner.store.get_skill_by_slug(slug) {
            Ok(Some(skill)) => skill,
            Ok(None) => {
                return refuse(
                    ErrorCode::BadRequest,
                    format!("No skill named {slug}. Run `sbot skills` to see the library."),
                )
            }
            Err(e) => return internal(e),
        };
        let draft = self.drafts_dir(&bot, &session).join(&skill.slug);
        let path = draft.to_string_lossy().into_owned();
        if std::fs::symlink_metadata(&draft).is_ok() {
            tracing::info!(bot = %bot.handle, session = session.number, skill = %skill.slug, "the skill's draft is resumed, not copied again");
            return BridgeResponse::draft(path, true);
        }
        match self.make_draft(&bot, &session, &skill.slug) {
            Ok(()) => {
                tracing::info!(bot = %bot.handle, session = session.number, skill = %skill.slug, "copied the skill into a draft");
                BridgeResponse::draft(path, false)
            }
            Err(e) => {
                tracing::warn!(bot = %bot.handle, session = session.number, skill = %skill.slug, error = %e, "could not copy the skill into a draft");
                failed("copy the skill", e)
            }
        }
    }

    fn make_draft(&self, bot: &Bot, session: &Session, slug: &str) -> std::io::Result<()> {
        let dir = match self.checked_bot_dir(&bot.handle) {
            Ok(dir) => dir,
            Err(FolderError::Io(e)) => return Err(e),
            Err(FolderError::Outside(dir)) => {
                return Err(std::io::Error::other(format!("{} leads outside the bots folder", dir.display())))
            }
        };
        let relative = drafts_relative(session);
        real_dir_chain(&dir, &relative)?;
        let drafts = dir.join(relative);
        std::fs::create_dir_all(&drafts)?;
        let draft = drafts.join(slug);
        let copied = skills::copy_dir(&self.skills_dir().join(slug), &draft);
        if copied.is_err() && std::fs::symlink_metadata(&draft).is_ok() {
            if let Err(e) = std::fs::remove_dir_all(&draft) {
                tracing::warn!(bot = %bot.handle, session = session.number, skill = %slug, error = %e, "could not remove a draft whose copy failed");
            }
        }
        copied
    }

    pub(super) fn own_draft(&self, bot: &Bot, session: &Session, folder: &Path) -> Option<(PathBuf, String)> {
        let dir = std::fs::canonicalize(self.bot_dir(&bot.handle)).ok()?;
        let drafts = dir.join(drafts_relative(session));
        let draft = std::fs::canonicalize(folder).ok()?;
        if draft == drafts || !draft.starts_with(&drafts) || !draft.is_dir() {
            return None;
        }
        let key = draft.strip_prefix(&dir).ok()?.to_string_lossy().into_owned();
        Some((draft, key))
    }

    pub(super) async fn bridge_skill_add(&self, pane_id: &str, folder: &str, recreate: bool) -> BridgeResponse {
        let (caller, flight) = match self.master_caller(pane_id, "skill add").await {
            Ok(found) => found,
            Err(refusal) => return refusal,
        };
        if !on_users_turn(flight.as_ref()) {
            return refuse(
                ErrorCode::NotAllowed,
                "Create skills and bots only while answering the user's message, after they approved the draft.",
            );
        }
        let (bot, session) = match self.caller_parts(&caller) {
            Ok(parts) => parts,
            Err(refusal) => return refusal,
        };
        let Some((draft, key)) = self.own_draft(&bot, &session, Path::new(folder)) else {
            let drafts = self.drafts_dir(&bot, &session);
            return refuse(
                ErrorCode::BadRequest,
                format!("Add only folders from this conversation's drafts: {}/<name>.", drafts.display()),
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
                return refuse(ErrorCode::BadRequest, format!("The draft cannot be added: {why}."))
            }
            Err(e) => return failed("read the draft", e),
        };
        let fingerprint = match folder_fingerprint(&draft) {
            Ok(fingerprint) => fingerprint,
            Err(e) => return failed("read the draft", e),
        };
        let store = &self.inner.store;
        let recorded = match store.creation(&session.id, CreationKind::Skill, &key) {
            Ok(recorded) => recorded,
            Err(e) => return internal(e),
        };
        if let Some(recorded) = recorded {
            match store.get_skill(&recorded.result_id) {
                Ok(Some(skill)) if recorded.fingerprint == fingerprint => {
                    return BridgeResponse::created(skill.slug, true)
                }
                Ok(Some(skill)) => {
                    return refuse(
                        ErrorCode::CreatedDifferently,
                        format!(
                            "This conversation already added the skill {} from this draft, which has changed since. \
                             A changed skill needs a new draft folder.",
                            skill.slug
                        ),
                    )
                }
                Ok(None) if !recreate => {
                    return refuse(
                        ErrorCode::CreatedThenDeleted,
                        format!(
                            "The skill {} this conversation added was deleted in the app. Ask the user; only if they \
                             want it again, repeat the command with --recreate.",
                            recorded.result_name
                        ),
                    )
                }
                Ok(None) => {}
                Err(e) => return internal(e),
            }
        }
        let record =
            NewCreation { session_id: &session.id, kind: CreationKind::Skill, key: &key, fingerprint: &fingerprint };
        let from = draft.display().to_string();
        match self.import_skill_with(&file, &from, |dir| skills::copy_dir(&draft, dir), Some(&record)) {
            Ok(skill) => {
                tracing::info!(bot = %bot.handle, session = session.number, skill = %skill.slug, draft = %key, "Master added a skill");
                self.notice(&session.id, "master_added_skill", json!({"slug": skill.slug}));
                self.emit(UiEvent::SkillsChanged);
                BridgeResponse::created(skill.slug, false)
            }
            Err(e) => failed("add the skill", e),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn bridge_bot_create(
        &self,
        pane_id: &str,
        name: &str,
        color: &str,
        provider: &str,
        model: Option<&str>,
        skills: &[String],
        instructions: &str,
        recreate: bool,
    ) -> BridgeResponse {
        let model = model.map(str::trim).filter(|model| !model.is_empty());
        let (caller, flight) = match self.master_caller(pane_id, "bot create").await {
            Ok(found) => found,
            Err(refusal) => return refusal,
        };
        if !on_users_turn(flight.as_ref()) {
            return refuse(
                ErrorCode::NotAllowed,
                "Create skills and bots only while answering the user's message, after they approved the draft.",
            );
        }
        let (bot, session) = match self.caller_parts(&caller) {
            Ok(parts) => parts,
            Err(refusal) => return refusal,
        };
        let unavailable =
            |provider: &str| refuse(ErrorCode::BadRequest, Refusal::ProviderUnavailable(provider.into()).to_string());
        let parsed = match Provider::parse(provider.trim()) {
            Some(parsed) if adapter_for(parsed).is_some() => parsed,
            _ => return unavailable(provider.trim()),
        };
        let key = name.trim().to_lowercase();
        if key.is_empty() {
            return refuse(ErrorCode::BadRequest, "A bot needs a name.");
        }
        if !is_hex_color(color) {
            return refuse(ErrorCode::BadRequest, "Give the color as #rrggbb, for example #4f46e5.");
        }
        let store = &self.inner.store;
        let mut skill_ids = Vec::new();
        let mut slugs = Vec::new();
        for slug in skills {
            let slug = slug.trim();
            match store.get_skill_by_slug(slug) {
                Ok(Some(skill)) => {
                    if !skill_ids.contains(&skill.id) {
                        skill_ids.push(skill.id);
                        slugs.push(skill.slug);
                    }
                }
                Ok(None) => return refuse(ErrorCode::BadRequest, format!("No skill named {slug}. Run `sbot skills`.")),
                Err(e) => return internal(e),
            }
        }
        let checkout = match self.project_folder() {
            Ok(checkout) => checkout,
            Err(e) => return internal(e),
        };
        let fingerprint = bot_fingerprint(name, color, provider, skills, instructions, model);
        let recorded = match store.creation(&session.id, CreationKind::Bot, &key) {
            Ok(recorded) => recorded,
            Err(e) => return internal(e),
        };
        if let Some(recorded) = recorded {
            match store.get_bot(&recorded.result_id) {
                Ok(Some(made)) if recorded.fingerprint == fingerprint => {
                    return BridgeResponse::created(made.handle, true)
                }
                Ok(Some(made)) => {
                    return refuse(
                        ErrorCode::CreatedDifferently,
                        format!(
                            "This conversation already created @{} as {} with other settings. A changed bot needs a new name.",
                            made.handle, name.trim()
                        ),
                    )
                }
                Ok(None) if !recreate => {
                    return refuse(
                        ErrorCode::CreatedThenDeleted,
                        format!(
                            "The bot @{} this conversation created was deleted in the app. Ask the user; only if they \
                             want it again, repeat the command with --recreate.",
                            recorded.result_name
                        ),
                    )
                }
                Ok(None) => {}
                Err(e) => return internal(e),
            }
        }
        let new = NewBot {
            display_name: name.trim().to_string(),
            color: color.to_string(),
            provider: parsed,
            model: model.map(String::from),
            instructions: format!(
                "{}\n\n{}",
                instructions.trim_end(),
                bundled::checkout_paragraph(checkout.as_deref())
            ),
            skill_ids,
        };
        let record =
            NewCreation { session_id: &session.id, kind: CreationKind::Bot, key: &key, fingerprint: &fingerprint };
        match self.create_bot_with(new, None, None, |n, h| store.insert_bot_recorded(n, h, &record)) {
            Ok(made) => {
                tracing::info!(bot = %bot.handle, session = session.number, created = %made.handle, "Master created a bot");
                self.notice(
                    &session.id,
                    "master_created_bot",
                    json!({"handle": made.handle, "skills": if slugs.is_empty() { "—".to_string() } else { slugs.join(", ") }}),
                );
                self.emit(UiEvent::BotAdded { bot: BotView { bot: made.clone(), status: BotStatus::Offline } });
                BridgeResponse::created(made.handle, false)
            }
            Err(CoreError::Refused(why)) => refuse(ErrorCode::BadRequest, why.to_string()),
            Err(e) => failed("create the bot", e),
        }
    }
}
