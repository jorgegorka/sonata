mod workflow;
pub use workflow::*;
mod onboarding;
pub use onboarding::*;

use std::io::Write;

use super::changing::{file_stamp, free_path, remove_leftover_of};
use super::master::{is_hex_color, MASTER_SETTING};
use super::*;
use crate::bundled::{checkout_paragraph, find_checkout_paragraph};
use crate::config_file::{
    self, Blob, ConfigFile, FileBot, FileDef, FileSkill, FileStep, FileWorkflow, FORMAT, MAX_FILE_BYTES, VERSION,
};
use crate::handle::is_valid_handle;
use crate::skills::{self, SKILL_FILE};
use crate::workflow::check_workflow;
use crate::Refusal;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExportSummary {
    pub bots: usize,
    pub skills: usize,
    pub workflows: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImportPreview {
    pub token: String,
    pub skills: Vec<PlannedItem>,
    pub bots: Vec<PlannedItem>,
    pub workflows: Vec<PlannedItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlannedItem {
    pub name: String,
    pub target: String,
    pub status: ItemStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ItemStatus {
    New,
    Unchanged,
    Different,
    Invalid { code: String, params: serde_json::Value },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportMode {
    AddNew,
    AddAndReplace,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ImportOutcome {
    Changed { preview: ImportPreview },
    Done { report: ImportReport },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImportReport {
    pub written: Vec<WrittenItem>,
    pub backup: Option<BackupPlace>,
    pub error: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WrittenItem {
    pub kind: ItemKind,
    pub name: String,
    pub replaced: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Skill,
    Bot,
    Workflow,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BackupPlace {
    pub file_name: String,
    pub kept_at: Option<String>,
}

struct Plan {
    preview: ImportPreview,
    skills: Vec<SkillStep>,
    bots: Vec<BotStep>,
    workflows: Vec<WorkflowStep>,
}

struct SkillStep {
    slug: String,
    name: String,
    description: String,
    blobs: Vec<Blob>,
    existing: Option<Skill>,
}

struct BotStep {
    file_handle: String,
    target: String,
    new: NewBot,
    skills: Vec<String>,
    existing: Option<Bot>,
    master: bool,
}

struct WorkflowStep {
    new: NewWorkflow,
    existing: Option<Workflow>,
}

struct CheckedBot {
    new: NewBot,
    skills: Vec<String>,
    existing: Option<Bot>,
    target: String,
}

fn invalid(name: &str, refusal: &Refusal) -> PlannedItem {
    PlannedItem {
        name: name.to_string(),
        target: String::new(),
        status: ItemStatus::Invalid { code: refusal.code().to_string(), params: refusal.params() },
    }
}

fn check_file(file: &ConfigFile) -> Result<()> {
    if file.bots.iter().filter(|bot| bot.master).count() > 1 {
        return Err(Refusal::ConfigFileTwoMasters.into());
    }
    let mut handles = HashSet::new();
    for bot in &file.bots {
        if !handles.insert(bot.handle.to_lowercase()) {
            return Err(Refusal::ConfigFileRepeats(format!("@{}", bot.handle)).into());
        }
    }
    let mut slugs = HashSet::new();
    for skill in &file.skills {
        if !slugs.insert(skill.slug.as_str()) {
            return Err(Refusal::ConfigFileRepeats(skill.slug.clone()).into());
        }
    }
    let mut names = HashSet::new();
    for workflow in &file.workflows {
        if !names.insert(workflow.name.trim().to_ascii_lowercase()) {
            return Err(Refusal::ConfigFileRepeats(workflow.name.clone()).into());
        }
    }
    Ok(())
}

fn checked_file_bot(
    bot: &FileBot,
    paragraph: &str,
    skill_refusal: impl Fn(&str) -> Option<Refusal>,
) -> Result<(NewBot, Vec<String>)> {
    if !is_hex_color(&bot.color) {
        return Err(Refusal::BotColorInvalid(bot.color.clone()).into());
    }
    let provider =
        Provider::parse(bot.provider.trim()).ok_or_else(|| Refusal::ProviderUnknown(bot.provider.clone()))?;
    let mut instructions = bot.instructions.clone();
    if bot.checkout {
        if let Some((range, _)) = find_checkout_paragraph(&instructions) {
            instructions.replace_range(range, paragraph);
        }
    }
    let new = editing::checked_bot(NewBot {
        display_name: bot.name.clone(),
        color: bot.color.clone(),
        provider,
        model: trimmed_model(bot.model.as_deref()),
        instructions,
        skill_ids: Vec::new(),
    })?;
    let mut slugs = bot.skills.clone();
    slugs.sort();
    slugs.dedup();
    if let Some(refusal) = slugs.iter().find_map(|slug| skill_refusal(slug)) {
        return Err(refusal.into());
    }
    Ok((new, slugs))
}

fn trimmed_model(model: Option<&str>) -> Option<String> {
    model.map(str::trim).filter(|m| !m.is_empty()).map(str::to_string)
}

fn def_by_handle(def: &WorkflowDef, handles: &HashMap<String, String>) -> WorkflowDef {
    let mut def = def.clone();
    for step in &mut def.steps {
        step.bot_id = handles.get(&step.bot_id).cloned().unwrap_or_default();
    }
    def
}

fn resolved_workflow(
    workflow: &FileWorkflow,
    bot_here: impl Fn(Option<&str>) -> std::result::Result<String, Refusal>,
    known: &HashSet<&str>,
) -> std::result::Result<NewWorkflow, Refusal> {
    let def = &workflow.definition;
    let steps = def
        .steps
        .iter()
        .map(|step| {
            Ok(Step {
                automatic_child_review: step.automatic_child_review,
                id: step.id.clone(),
                name: step.name.clone(),
                bot_id: bot_here(step.bot.as_deref())?,
                instructions: step.instructions.clone(),
                arrows: step.arrows.clone(),
                pos: step.pos.clone(),
            })
        })
        .collect::<std::result::Result<Vec<_>, Refusal>>()?;
    let new = NewWorkflow {
        name: workflow.name.trim().to_string(),
        when_to_use: workflow.when_to_use.clone(),
        def: WorkflowDef {
            version: def.version,
            start: def.start.clone(),
            steps,
            start_pos: def.start_pos.clone(),
            done_pos: def.done_pos.clone(),
        },
    };
    check_workflow(&new, |handle| known.contains(handle))?;
    Ok(new)
}

fn too_large() -> CoreError {
    Refusal::ConfigExportTooLarge { mb: MAX_FILE_BYTES / (1024 * 1024) }.into()
}

fn error_value(e: &CoreError) -> serde_json::Value {
    match e {
        CoreError::Refused(refusal) => json!({ "code": refusal.code(), "params": refusal.params() }),
        e => json!({ "code": "internal", "params": { "error": e.to_string() } }),
    }
}

fn file_bot(bot: &Bot, mut slugs: Vec<String>, master: bool) -> FileBot {
    let mut instructions = bot.instructions.clone();
    let mut checkout = false;
    if let Some((range, _)) = find_checkout_paragraph(&instructions) {
        instructions.replace_range(range, &checkout_paragraph(None));
        checkout = true;
    }
    slugs.sort();
    FileBot {
        handle: bot.handle.clone(),
        name: bot.display_name.clone(),
        color: bot.color.clone(),
        provider: bot.provider.as_str().to_string(),
        model: bot.model.clone(),
        checkout,
        master,
        skills: slugs,
        instructions,
    }
}

fn file_workflow(workflow: &Workflow, handles: &HashMap<String, String>) -> FileWorkflow {
    let def = &workflow.def;
    FileWorkflow {
        name: workflow.name.clone(),
        when_to_use: workflow.when_to_use.clone(),
        definition: FileDef {
            version: def.version,
            start: def.start.clone(),
            start_pos: def.start_pos.clone(),
            done_pos: def.done_pos.clone(),
            steps: def
                .steps
                .iter()
                .map(|step| FileStep {
                    automatic_child_review: step.automatic_child_review,
                    id: step.id.clone(),
                    name: step.name.clone(),
                    bot: handles.get(&step.bot_id).cloned(),
                    pos: step.pos.clone(),
                    arrows: step.arrows.clone(),
                    instructions: step.instructions.clone(),
                })
                .collect(),
        },
    }
}

impl Engine {
    fn file_skill(&self, skill: &Skill) -> Result<FileSkill> {
        let folder = self.skills_dir().join(&skill.slug);
        if !folder.join(SKILL_FILE).is_file() {
            return Err(Refusal::SkillNoFile(skill.slug.clone()).into());
        }
        let blobs = config_file::read_skill_folder(&folder)?;
        Ok(FileSkill {
            slug: skill.slug.clone(),
            name: skill.name.clone(),
            description: skill.description.clone(),
            files: blobs.iter().map(config_file::to_entry).collect(),
        })
    }

    pub fn export_config(&self) -> Result<ConfigFile> {
        let store = &self.store;
        let master = self.master_id()?;
        let mut bots = Vec::new();
        let mut handles = HashMap::new();
        for bot in store.list_bots()? {
            let slugs = store.skills_of(&bot.id)?.into_iter().map(|s| s.slug).collect();
            let is_master = master.as_deref() == Some(bot.id.as_str());
            handles.insert(bot.id.clone(), bot.handle.clone());
            let entry = file_bot(&bot, slugs, is_master);
            if is_master {
                bots.insert(0, entry);
            } else {
                bots.push(entry);
            }
        }
        let used: HashSet<String> = store.list_bots()?.iter().flat_map(|b| b.skill_ids.clone()).collect();
        let library: Vec<_> =
            store.list_skills()?.into_iter().filter(|s| !store.is_scoped() || used.contains(&s.skill.id)).collect();
        let mut size = 0;
        for view in &library {
            let folder = self.skills_dir().join(&view.skill.slug);
            if folder.is_dir() {
                size += config_file::folder_size(&folder)?;
                if size > MAX_FILE_BYTES {
                    return Err(too_large());
                }
            }
        }
        let skills = library.iter().map(|view| self.file_skill(&view.skill)).collect::<Result<Vec<_>>>()?;
        let workflows = store.list_workflows()?.iter().map(|w| file_workflow(w, &handles)).collect();
        Ok(self.configuration(bots, skills, workflows))
    }

    fn configuration(&self, bots: Vec<FileBot>, skills: Vec<FileSkill>, workflows: Vec<FileWorkflow>) -> ConfigFile {
        ConfigFile {
            format: FORMAT.to_string(),
            version: VERSION,
            exported_at: self.now_utc().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            bots,
            skills,
            workflows,
        }
    }

    pub fn write_config(&self, path: &Path) -> Result<ExportSummary> {
        self.write_config_file(path, &self.export_config()?)
    }

    fn write_config_file(&self, path: &Path, file: &ConfigFile) -> Result<ExportSummary> {
        let yaml = config_file::to_yaml(file)?;
        if yaml.len() as u64 > MAX_FILE_BYTES {
            return Err(too_large());
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let temp = path.with_file_name(format!(".{name}.tmp-{}", ulid::Ulid::new()));
        let written = std::fs::write(&temp, yaml).and_then(|()| std::fs::rename(&temp, path));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&temp);
            return Err(e.into());
        }
        let summary =
            ExportSummary { bots: file.bots.len(), skills: file.skills.len(), workflows: file.workflows.len() };
        tracing::info!(
            path = %path.display(),
            bots = summary.bots,
            skills = summary.skills,
            workflows = summary.workflows,
            "configuration exported"
        );
        Ok(summary)
    }

    pub fn preview_config_import(&self, path: &Path) -> Result<ImportPreview> {
        let (file, digest) = config_file::read_file(path)?;
        let preview = self.plan(&file, &digest)?.preview;
        tracing::info!(
            path = %path.display(),
            skills = preview.skills.len(),
            bots = preview.bots.len(),
            workflows = preview.workflows.len(),
            "configuration import previewed"
        );
        Ok(preview)
    }

    pub fn import_config(&self, path: &Path, mode: ImportMode, token: &str) -> Result<ImportOutcome> {
        let _importing = self.inner.importing.lock().unwrap();
        let (file, digest) = config_file::read_file(path)?;
        let plan = self.plan(&file, &digest)?;
        if plan.preview.token != token {
            tracing::info!(path = %path.display(), "the file or the install changed since the preview: nothing imported");
            return Ok(ImportOutcome::Changed { preview: plan.preview });
        }
        let replace = mode == ImportMode::AddAndReplace;
        let skills: Vec<SkillStep> =
            plan.skills.into_iter().filter(|step| replace || step.existing.is_none()).collect();
        let bots: Vec<BotStep> = plan.bots.into_iter().filter(|step| replace || step.existing.is_none()).collect();
        let workflows: Vec<WorkflowStep> =
            plan.workflows.into_iter().filter(|step| replace || step.existing.is_none()).collect();
        let backup = match self.write_backup(&skills, &bots, &workflows) {
            Ok(backup) => backup,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "could not back up what the import replaces: nothing imported");
                return Err(e);
            }
        };
        let mut written = Vec::new();
        let error = self.write_steps(skills, bots, workflows, &mut written).err().map(|e| error_value(&e));
        if written.iter().any(|item| item.kind == ItemKind::Skill) {
            self.emit(UiEvent::SkillsChanged);
        }
        let backup = backup.map(|backup| self.put_backup_away(&backup));
        tracing::info!(path = %path.display(), written = written.len(), stopped = error.is_some(), "configuration imported");
        Ok(ImportOutcome::Done { report: ImportReport { written, backup, error } })
    }

    fn write_backup(
        &self,
        skills: &[SkillStep],
        bots: &[BotStep],
        workflows: &[WorkflowStep],
    ) -> Result<Option<PathBuf>> {
        let old_skills: Vec<&Skill> = skills.iter().filter_map(|step| step.existing.as_ref()).collect();
        let old_bots: Vec<&Bot> = bots.iter().filter_map(|step| step.existing.as_ref()).collect();
        let old_workflows: Vec<&Workflow> = workflows.iter().filter_map(|step| step.existing.as_ref()).collect();
        if old_skills.is_empty() && old_bots.is_empty() && old_workflows.is_empty() {
            return Ok(None);
        }
        let backup_too_large = || CoreError::from(Refusal::ConfigBackupTooLarge { mb: MAX_FILE_BYTES / (1024 * 1024) });
        let mut size = 0;
        for skill in &old_skills {
            let folder = self.skills_dir().join(&skill.slug);
            if folder.is_dir() {
                size += config_file::folder_size(&folder)?;
                if size > MAX_FILE_BYTES {
                    return Err(backup_too_large());
                }
            }
        }
        let store = &self.store;
        let master = self.master_id()?;
        let file_bots = old_bots
            .iter()
            .map(|bot| {
                let slugs = store.skills_of(&bot.id)?.into_iter().map(|s| s.slug).collect();
                Ok(file_bot(bot, slugs, master.as_deref() == Some(bot.id.as_str())))
            })
            .collect::<Result<Vec<_>>>()?;
        let file_skills = old_skills.iter().map(|skill| self.file_skill(skill)).collect::<Result<Vec<_>>>()?;
        let handles: HashMap<String, String> = store.list_bots()?.into_iter().map(|bot| (bot.id, bot.handle)).collect();
        let file_workflows = old_workflows.iter().map(|workflow| file_workflow(workflow, &handles)).collect();
        let yaml = config_file::to_yaml(&self.configuration(file_bots, file_skills, file_workflows))?;
        if yaml.len() as u64 > MAX_FILE_BYTES {
            return Err(backup_too_large());
        }
        let dir = self.inner.cfg.paths.previous_dir();
        std::fs::create_dir_all(&dir)?;
        let stem = format!("sonata-config-before-import-{}", file_stamp(self.now_utc()));
        let path = free_path(&dir, &stem, ".yaml");
        let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
        if let Err(e) = out.write_all(yaml.as_bytes()).and_then(|()| out.sync_all()) {
            drop(out);
            let _ = std::fs::remove_file(&path);
            return Err(e.into());
        }
        tracing::info!(
            path = %path.display(),
            bots = old_bots.len(),
            skills = old_skills.len(),
            workflows = old_workflows.len(),
            "backed up what the import replaces"
        );
        Ok(Some(path))
    }

    fn put_backup_away(&self, path: &Path) -> BackupPlace {
        let file_name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let trash = self.inner.trash.read().unwrap().clone();
        match trash.map(|trash| trash(path)) {
            Some(Ok(())) => return BackupPlace { file_name, kept_at: None },
            Some(Err(e)) => {
                tracing::warn!(path = %path.display(), error = %e, "could not move the import's backup to the Trash")
            }
            None => {}
        }
        BackupPlace { file_name, kept_at: Some(path.display().to_string()) }
    }

    fn write_steps(
        &self,
        skills: Vec<SkillStep>,
        bots: Vec<BotStep>,
        workflows: Vec<WorkflowStep>,
        written: &mut Vec<WrittenItem>,
    ) -> Result<()> {
        for step in skills {
            let replaced = step.existing.is_some();
            if let Err(e) = self.write_skill(&step) {
                tracing::warn!(skill = %step.slug, error = %e, "could not import the skill; the import stops");
                return Err(e);
            }
            tracing::info!(skill = %step.slug, replaced, "skill imported");
            written.push(WrittenItem { kind: ItemKind::Skill, name: step.slug, replaced });
        }
        let mut ids = HashMap::new();
        for step in bots {
            let replaced = step.existing.is_some();
            let bot = match self.write_bot(&step) {
                Ok(bot) => bot,
                Err(e) => {
                    tracing::warn!(bot = %step.target, error = %e, "could not import the bot; the import stops");
                    return Err(e);
                }
            };
            tracing::info!(bot = %bot.handle, file_handle = %step.file_handle, replaced, "bot imported");
            ids.insert(step.target, bot.id);
            written.push(WrittenItem { kind: ItemKind::Bot, name: bot.handle, replaced });
        }
        for step in workflows {
            let replaced = step.existing.is_some();
            let name = step.new.name.clone();
            let workflow = match self.write_workflow(step, &ids) {
                Ok(workflow) => workflow,
                Err(e) => {
                    tracing::warn!(workflow = %name, error = %e, "could not import the workflow; the import stops");
                    return Err(e);
                }
            };
            tracing::info!(workflow = %workflow.name, replaced, "workflow imported");
            written.push(WrittenItem { kind: ItemKind::Workflow, name: workflow.name, replaced });
        }
        Ok(())
    }

    fn write_skill(&self, step: &SkillStep) -> Result<()> {
        let fill = |dir: &Path| config_file::write_blobs(dir, &step.blobs);
        let Some(skill) = &step.existing else {
            self.add_skill_at(&step.slug, &step.name, &step.description, fill)?;
            return Ok(());
        };
        let staged = self.staging_dir(&step.slug)?;
        if let Err(e) = fill(&staged) {
            remove_leftover_of(&staged, &step.slug);
            return Err(e.into());
        }
        let (previous, bots) = self.install_staged(skill, &staged, &step.name, &step.description)?;
        tracing::info!(skill = %step.slug, bots = ?bots, "skill replaced; its bots restart with their next message");
        remove_leftover_of(&previous, &step.slug);
        Ok(())
    }

    fn write_bot(&self, step: &BotStep) -> Result<Bot> {
        let store = &self.store;
        let mut skill_ids = Vec::new();
        for slug in &step.skills {
            let skill = store.get_skill_by_slug(slug)?.ok_or_else(|| Refusal::SkillUnknown(slug.clone()))?;
            skill_ids.push(skill.id);
        }
        let new = NewBot { skill_ids, ..step.new.clone() };
        let target = Some(step.target.as_str());
        match &step.existing {
            Some(bot) => self.update_bot(&bot.id, new),
            None if step.master => self.create_bot_with(new, target, None, |new, handle| {
                store.insert_bot_recorded_as(new, handle, MASTER_SETTING)
            }),
            None => self.create_bot_with(new, target, None, |new, handle| store.insert_bot(new, handle)),
        }
    }

    fn write_workflow(&self, step: WorkflowStep, ids: &HashMap<String, String>) -> Result<Workflow> {
        let mut new = step.new;
        for def_step in &mut new.def.steps {
            def_step.bot_id = match ids.get(&def_step.bot_id) {
                Some(id) => id.clone(),
                None => self.store.get_bot_by_handle(&def_step.bot_id)?.map(|bot| bot.id).unwrap_or_default(),
            };
        }
        match &step.existing {
            Some(workflow) => self.update_workflow(&workflow.id, new),
            None => self.create_workflow(new),
        }
    }

    fn plan(&self, file: &ConfigFile, digest: &[u8; 32]) -> Result<Plan> {
        check_file(file)?;
        let mut skills = Vec::new();
        let mut skill_steps = Vec::new();
        for skill in &file.skills {
            let (item, step) = self.plan_skill(skill)?;
            skills.push(item);
            skill_steps.extend(step);
        }
        let (bots, bot_steps) = self.plan_bots(file, &skills)?;
        let (workflows, workflow_steps) = self.plan_workflows(file, &bots)?;
        let mut hasher = Sha256::new();
        hasher.update(self.store.project_key().as_bytes());
        hasher.update(digest);
        hasher.update(serde_json::to_vec(&(&skills, &bots, &workflows)).expect("a preview serializes"));
        let token = format!("{:x}", hasher.finalize());
        Ok(Plan {
            preview: ImportPreview { token, skills, bots, workflows },
            skills: skill_steps,
            bots: bot_steps,
            workflows: workflow_steps,
        })
    }

    fn plan_bots(&self, file: &ConfigFile, skills: &[PlannedItem]) -> Result<(Vec<PlannedItem>, Vec<BotStep>)> {
        self.plan_bots_with_dependencies(file, skills, true)
    }

    fn plan_bots_with_dependencies(
        &self,
        file: &ConfigFile,
        skills: &[PlannedItem],
        check_dependencies: bool,
    ) -> Result<(Vec<PlannedItem>, Vec<BotStep>)> {
        let store = &self.store;
        let importable: HashMap<&str, bool> = file
            .skills
            .iter()
            .zip(skills)
            .map(|(skill, item)| (skill.slug.as_str(), !matches!(item.status, ItemStatus::Invalid { .. })))
            .collect();
        let library: HashSet<String> = store.list_skills()?.into_iter().map(|view| view.skill.slug).collect();
        let skill_refusal = |slug: &str| {
            if !check_dependencies {
                None
            } else {
                match importable.get(slug) {
                    Some(true) => None,
                    Some(false) => Some(Refusal::ImportDependsOn(slug.to_string())),
                    None if library.contains(slug) => None,
                    None => Some(Refusal::SkillUnknown(slug.to_string())),
                }
            }
        };
        let paragraph = checkout_paragraph(self.project_folder()?.as_deref());
        let master = self.master_bot()?;
        let mut checked = Vec::new();
        for bot in &file.bots {
            let (new, slugs) = match checked_file_bot(bot, &paragraph, skill_refusal) {
                Ok(checked) => checked,
                Err(CoreError::Refused(refusal)) => {
                    checked.push(Err(refusal));
                    continue;
                }
                Err(e) => return Err(e),
            };
            let existing = if bot.master {
                master.clone()
            } else {
                store
                    .get_bot_by_handle(&bot.handle.to_lowercase())?
                    .filter(|here| master.as_ref().is_none_or(|master| master.id != here.id))
            };
            let target = existing.as_ref().map(|here| here.handle.clone()).unwrap_or_default();
            checked.push(Ok(CheckedBot { new, skills: slugs, existing, target }));
        }

        let mut taken = self.taken_handles()?;
        let mut new_bots: Vec<(&FileBot, &mut CheckedBot)> = file
            .bots
            .iter()
            .zip(&mut checked)
            .filter_map(|(bot, checked)| Some((bot, checked.as_mut().ok().filter(|c| c.existing.is_none())?)))
            .collect();
        for (bot, checked) in &mut new_bots {
            let handle = bot.handle.to_lowercase();
            if is_valid_handle(&handle) && !taken.contains(&handle) {
                taken.push(handle.clone());
                checked.target = handle;
            }
        }
        for (bot, checked) in &mut new_bots {
            if checked.target.is_empty() {
                checked.target = derive_handle(&bot.handle, &taken);
                taken.push(checked.target.clone());
            }
        }

        let mut items = Vec::new();
        let mut steps = Vec::new();
        for (bot, checked) in file.bots.iter().zip(checked) {
            let CheckedBot { new, skills, existing, target } = match checked {
                Ok(checked) => checked,
                Err(refusal) => {
                    items.push(invalid(&bot.handle, &refusal));
                    continue;
                }
            };
            let status = match &existing {
                Some(here) => {
                    let here_skills: Vec<String> = store.skills_of(&here.id)?.into_iter().map(|s| s.slug).collect();
                    let same = here.display_name == new.display_name
                        && here.color == new.color
                        && here.provider == new.provider
                        && trimmed_model(here.model.as_deref()) == new.model
                        && here_skills == skills
                        && here.instructions == new.instructions;
                    if same {
                        ItemStatus::Unchanged
                    } else {
                        ItemStatus::Different
                    }
                }
                None => ItemStatus::New,
            };
            if status != ItemStatus::Unchanged {
                steps.push(BotStep {
                    file_handle: bot.handle.clone(),
                    target: target.clone(),
                    new,
                    skills,
                    existing,
                    master: bot.master,
                });
            }
            items.push(PlannedItem { name: bot.handle.clone(), target, status });
        }
        Ok((items, steps))
    }

    fn plan_workflows(&self, file: &ConfigFile, bots: &[PlannedItem]) -> Result<(Vec<PlannedItem>, Vec<WorkflowStep>)> {
        let store = &self.store;
        let decided: HashMap<String, &PlannedItem> =
            file.bots.iter().zip(bots).map(|(bot, item)| (bot.handle.to_lowercase(), item)).collect();
        let here: HashMap<String, String> = store.list_bots()?.into_iter().map(|bot| (bot.id, bot.handle)).collect();
        let here_handles: HashSet<&str> = here.values().map(String::as_str).collect();
        let mut known = here_handles.clone();
        known.extend(
            bots.iter().filter(|item| !matches!(item.status, ItemStatus::Invalid { .. })).map(|i| i.target.as_str()),
        );
        let bot_here = |handle: Option<&str>| {
            let Some(handle) = handle.map(str::to_lowercase) else { return Ok(String::new()) };
            match decided.get(&handle) {
                Some(item) if matches!(item.status, ItemStatus::Invalid { .. }) => {
                    Err(Refusal::ImportDependsOn(format!("@{}", item.name)))
                }
                Some(item) => Ok(item.target.clone()),
                None if here_handles.contains(handle.as_str()) => Ok(handle),
                None => Ok(String::new()),
            }
        };
        let mut items = Vec::new();
        let mut steps = Vec::new();
        for workflow in &file.workflows {
            let new = match resolved_workflow(workflow, bot_here, &known) {
                Ok(new) => new,
                Err(refusal) => {
                    items.push(invalid(&workflow.name, &refusal));
                    continue;
                }
            };
            let existing = store.get_workflow_by_name(&new.name)?;
            let (target, status) = match &existing {
                Some(stored) => {
                    let same = stored.name == new.name
                        && stored.when_to_use == new.when_to_use
                        && def_by_handle(&stored.def, &here) == new.def;
                    (stored.name.clone(), if same { ItemStatus::Unchanged } else { ItemStatus::Different })
                }
                None => (new.name.clone(), ItemStatus::New),
            };
            if status != ItemStatus::Unchanged {
                steps.push(WorkflowStep { new, existing });
            }
            items.push(PlannedItem { name: workflow.name.clone(), target, status });
        }
        Ok((items, steps))
    }

    fn plan_skill(&self, skill: &FileSkill) -> Result<(PlannedItem, Option<SkillStep>)> {
        let slug = &skill.slug;
        let decoded = match config_file::decode_skill(skill) {
            Ok(decoded) => decoded,
            Err(refusal) => return Ok((invalid(slug, &refusal), None)),
        };
        let name = match skill.name.trim() {
            "" => slug.clone(),
            name => name.to_string(),
        };
        let description = decoded.file.description;
        let blobs = decoded.blobs;
        let folder = self.skills_dir().join(slug);
        let existing = self.store.get_skill_by_slug(slug)?;
        let status = match &existing {
            Some(here) => {
                let same_files = config_file::read_skill_folder(&folder).is_ok_and(|files| files == blobs);
                if same_files && here.name == name && here.description == description {
                    ItemStatus::Unchanged
                } else {
                    ItemStatus::Different
                }
            }
            None => match std::fs::symlink_metadata(&folder) {
                Ok(_) => return Ok((invalid(slug, &Refusal::SkillFolderTaken(slug.clone())), None)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => ItemStatus::New,
                Err(e) => return Err(e.into()),
            },
        };
        let item = PlannedItem { name: slug.clone(), target: slug.clone(), status };
        let step = (item.status != ItemStatus::Unchanged).then(|| SkillStep {
            slug: slug.clone(),
            name,
            description,
            blobs,
            existing,
        });
        Ok((item, step))
    }
}
