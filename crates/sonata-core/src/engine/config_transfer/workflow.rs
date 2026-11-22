use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyChoice {
    KeepLocal,
    Replace,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowImportRequest {
    pub workflow: String,
    #[serde(default)]
    pub workflow_choice: Option<DependencyChoice>,
    #[serde(default)]
    pub new_name: Option<String>,
    #[serde(default)]
    pub bots: BTreeMap<String, DependencyChoice>,
    #[serde(default)]
    pub skills: BTreeMap<String, DependencyChoice>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkflowFileEntry {
    pub name: String,
    pub when_to_use: String,
    pub steps: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowImportAction {
    Add,
    Replace,
    Reuse,
    Choose,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkflowImportItem {
    pub kind: ItemKind,
    #[serde(flatten)]
    pub item: PlannedItem,
    pub action: WorkflowImportAction,
    pub can_keep: bool,
    pub required_by: Vec<String>,
    pub changed_fields: Vec<String>,
    pub changed_files: Vec<String>,
    pub used_by_bots: Vec<String>,
    pub used_by_workflows: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkflowImportPreview {
    pub token: String,
    pub items: Vec<WorkflowImportItem>,
    pub can_import: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum WorkflowImportOutcome {
    Changed { preview: WorkflowImportPreview },
    Blocked { preview: WorkflowImportPreview },
    Done { report: ImportReport },
}

struct WorkflowPlan {
    preview: WorkflowImportPreview,
    skills: Vec<SkillStep>,
    bots: Vec<BotStep>,
    workflows: Vec<WorkflowStep>,
}

fn action(status: &ItemStatus, choice: Option<DependencyChoice>, can_keep: bool) -> WorkflowImportAction {
    use WorkflowImportAction::*;
    if choice == Some(DependencyChoice::KeepLocal) {
        return if can_keep { Reuse } else { Blocked };
    }
    match status {
        ItemStatus::New => Add,
        ItemStatus::Unchanged => Reuse,
        ItemStatus::Different if choice == Some(DependencyChoice::Replace) => Replace,
        ItemStatus::Different => Choose,
        ItemStatus::Invalid { .. } if can_keep && choice.is_none() => Choose,
        ItemStatus::Invalid { .. } => Blocked,
    }
}

fn row(kind: ItemKind, item: PlannedItem, choice: Option<DependencyChoice>, can_keep: bool) -> WorkflowImportItem {
    WorkflowImportItem {
        action: action(&item.status, choice, can_keep),
        kind,
        item,
        can_keep,
        required_by: Vec::new(),
        changed_fields: Vec::new(),
        changed_files: Vec::new(),
        used_by_bots: Vec::new(),
        used_by_workflows: Vec::new(),
    }
}

fn fields<T: Serialize>(before: &T, after: &T, keys: &[&str]) -> Vec<String> {
    let before = serde_json::to_value(before).expect("configuration serializes");
    let after = serde_json::to_value(after).expect("configuration serializes");
    keys.iter().filter(|key| before[**key] != after[**key]).map(|key| (*key).to_string()).collect()
}

fn fingerprint<T: Serialize>(hasher: &mut Sha256, value: &T) {
    let bytes = serde_json::to_vec(value).expect("configuration serializes");
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

impl Engine {
    pub fn export_workflow_config(&self, workflow_id: &str) -> Result<ConfigFile> {
        let store = &self.store;
        let workflow = store.get_workflow(workflow_id)?.ok_or_else(|| Refusal::WorkflowNotFound(workflow_id.into()))?;
        let master = self.master_id()?;
        let mut bots = Vec::new();
        let mut handles = HashMap::new();
        let mut skills = BTreeMap::new();
        for step in &workflow.def.steps {
            if handles.contains_key(&step.bot_id) {
                continue;
            }
            let bot =
                store.get_bot(&step.bot_id)?.ok_or_else(|| Refusal::WorkflowStepNoBot { step: step.name.clone() })?;
            let assigned = store.skills_of(&bot.id)?;
            let slugs = assigned.iter().map(|s| s.slug.clone()).collect();
            for skill in assigned {
                skills.insert(skill.slug.clone(), skill);
            }
            bots.push(file_bot(&bot, slugs, master.as_deref() == Some(bot.id.as_str())));
            handles.insert(bot.id, bot.handle);
        }
        check_workflow(
            &NewWorkflow {
                name: workflow.name.clone(),
                when_to_use: workflow.when_to_use.clone(),
                def: workflow.def.clone(),
            },
            |id| handles.contains_key(id),
        )?;
        let mut size = 0;
        let mut files = Vec::new();
        for skill in skills.values() {
            let folder = self.skills_dir().join(&skill.slug);
            if folder.is_dir() {
                let skill_size = config_file::folder_size(&folder)?;
                if skill_size > skills::MAX_SKILL_BYTES {
                    return Err(Refusal::SkillTooLarge { mb: skills::MAX_SKILL_BYTES / (1024 * 1024) }.into());
                }
                size += skill_size;
                if size > MAX_FILE_BYTES {
                    return Err(too_large());
                }
            }
            let file = self.file_skill(skill)?;
            config_file::decode_skill(&file)?;
            files.push(file);
        }
        Ok(self.configuration(bots, files, vec![file_workflow(&workflow, &handles)]))
    }

    pub fn write_workflow_config(&self, workflow_id: &str, path: &Path) -> Result<ExportSummary> {
        self.write_config_file(path, &self.export_workflow_config(workflow_id)?)
    }

    pub fn workflows_in_config(&self, path: &Path) -> Result<Vec<WorkflowFileEntry>> {
        let (file, _) = config_file::read_file(path)?;
        check_file(&file)?;
        Ok(file
            .workflows
            .iter()
            .map(|w| WorkflowFileEntry {
                name: w.name.clone(),
                when_to_use: w.when_to_use.clone(),
                steps: w.definition.steps.len(),
            })
            .collect())
    }

    pub fn preview_workflow_import(
        &self,
        path: &Path,
        request: &WorkflowImportRequest,
    ) -> Result<WorkflowImportPreview> {
        let (file, digest) = config_file::read_file(path)?;
        Ok(self.plan_workflow_import(&file, &digest, request)?.preview)
    }

    pub fn import_workflow_config(
        &self,
        path: &Path,
        request: &WorkflowImportRequest,
        token: &str,
    ) -> Result<WorkflowImportOutcome> {
        let _importing = self.inner.importing.lock().unwrap();
        let (file, digest) = config_file::read_file(path)?;
        let plan = self.plan_workflow_import(&file, &digest, request)?;
        if plan.preview.token != token {
            return Ok(WorkflowImportOutcome::Changed { preview: plan.preview });
        }
        if !plan.preview.can_import {
            return Ok(WorkflowImportOutcome::Blocked { preview: plan.preview });
        }
        let backup = self.write_backup(&plan.skills, &plan.bots, &plan.workflows)?;
        let mut written = Vec::new();
        let error =
            self.write_steps(plan.skills, plan.bots, plan.workflows, &mut written).err().map(|e| error_value(&e));
        if written.iter().any(|item| item.kind == ItemKind::Skill) {
            self.emit(UiEvent::SkillsChanged);
        }
        let backup = backup.map(|p| self.put_backup_away(&p));
        tracing::info!(workflow = %request.workflow, written = written.len(), stopped = error.is_some(), "workflow configuration imported");
        Ok(WorkflowImportOutcome::Done { report: ImportReport { written, backup, error } })
    }

    fn matching_file_bot(&self, bot: &FileBot) -> Result<Option<Bot>> {
        let master = self.master_bot()?;
        Ok(if bot.master {
            master
        } else {
            self.store
                .get_bot_by_handle(&bot.handle.to_lowercase())?
                .filter(|b| master.as_ref().is_none_or(|m| b.id != m.id))
        })
    }

    fn local_file_bot(&self, bot: &Bot) -> Result<FileBot> {
        let slugs = self.store.skills_of(&bot.id)?.into_iter().map(|s| s.slug).collect();
        Ok(file_bot(bot, slugs, self.master_id()?.as_deref() == Some(bot.id.as_str())))
    }

    fn plan_workflow_import(
        &self,
        file: &ConfigFile,
        digest: &[u8; 32],
        request: &WorkflowImportRequest,
    ) -> Result<WorkflowPlan> {
        check_file(file)?;
        let source = file
            .workflows
            .iter()
            .find(|w| w.name == request.workflow)
            .ok_or_else(|| Refusal::WorkflowNotFound(request.workflow.clone()))?;
        let store = &self.store;
        let existing = store.get_workflow_by_name(source.name.trim())?;
        let local_bots = store.list_bots()?;
        let local_handles: HashMap<String, String> =
            local_bots.iter().map(|b| (b.id.clone(), b.handle.clone())).collect();
        let local_workflows = store.list_workflows()?;
        let local_workflow = existing.as_ref().map(|w| file_workflow(w, &local_handles));
        let keep_root = request.new_name.is_none()
            && request.workflow_choice == Some(DependencyChoice::KeepLocal)
            && existing.is_some();
        let mut root = if keep_root { local_workflow.as_ref().unwrap().clone() } else { source.clone() };
        if let Some(name) = &request.new_name {
            root.name = name.trim().to_string();
        }

        let source_handles: HashSet<String> =
            source.definition.steps.iter().filter_map(|s| s.bot.as_ref().map(|h| h.to_lowercase())).collect();
        let candidates: Vec<&FileBot> =
            file.bots.iter().filter(|b| source_handles.contains(&b.handle.to_lowercase())).collect();
        let candidate_slugs: HashSet<&str> =
            candidates.iter().flat_map(|b| b.skills.iter().map(String::as_str)).collect();
        let candidate_skills: HashMap<&str, &FileSkill> = file
            .skills
            .iter()
            .filter(|s| candidate_slugs.contains(s.slug.as_str()))
            .map(|s| (s.slug.as_str(), s))
            .collect();
        let mut matches = HashMap::new();
        for b in &candidates {
            matches.insert(b.handle.to_lowercase(), self.matching_file_bot(b)?);
        }

        let mut hash = Sha256::new();
        fingerprint(&mut hash, &self.store.project_key());
        fingerprint(&mut hash, digest);
        fingerprint(&mut hash, request);
        fingerprint(&mut hash, &existing);
        fingerprint(&mut hash, &self.master_id()?);
        let checkout = self.project_folder()?;
        fingerprint(&mut hash, &checkout);
        let paragraph = checkout_paragraph(checkout.as_deref());

        let mut effective = BTreeMap::<String, (FileBot, bool, Option<Bot>)>::new();
        let mut root_keys = HashMap::new();
        let mut reasons = BTreeMap::<String, Vec<String>>::new();
        for step in &root.definition.steps {
            let Some(handle) = step.bot.as_ref().map(|h| h.to_lowercase()) else {
                continue;
            };
            let candidate = candidates.iter().find(|b| {
                if keep_root {
                    matches[&b.handle.to_lowercase()].as_ref().is_some_and(|local| local.handle == handle)
                } else {
                    b.handle.to_lowercase() == handle
                }
            });
            let (input, incoming, local) = if let Some(b) = candidate {
                ((*b).clone(), true, matches[&b.handle.to_lowercase()].clone())
            } else if let Some(b) = store.get_bot_by_handle(&handle)? {
                (self.local_file_bot(&b)?, false, Some(b))
            } else {
                continue;
            };
            let key = input.handle.clone();
            root_keys.insert(handle, key.clone());
            reasons.entry(key.clone()).or_default().push(step.name.clone());
            effective.insert(key, (input, incoming, local));
        }
        let bot_file = self.configuration(effective.values().map(|(b, _, _)| b.clone()).collect(), vec![], vec![]);
        let (bot_items, mut bot_steps) = self.plan_bots_with_dependencies(&bot_file, &[], false)?;
        let mut items = Vec::new();
        let mut resolved = HashMap::new();
        let mut needed_skills = BTreeMap::<String, Vec<String>>::new();
        let mut selected_bots = Vec::new();
        for ((key, (input, incoming, local)), item) in effective.iter().zip(bot_items) {
            fingerprint(&mut hash, local);
            let local_file = local.as_ref().map(|b| self.local_file_bot(b)).transpose()?;
            fingerprint(&mut hash, &local_file);
            let can_keep = local_file.as_ref().is_some_and(|b| checked_file_bot(b, &paragraph, |_| None).is_ok());
            let choice = if *incoming { request.bots.get(key).copied() } else { Some(DependencyChoice::KeepLocal) };
            let mut item = item;
            if !incoming && can_keep {
                item.status = ItemStatus::Unchanged;
            }
            let mut entry = row(ItemKind::Bot, item, choice, can_keep);
            entry.required_by = reasons.remove(key).unwrap_or_default();
            if let Some(local) = local {
                entry.used_by_workflows = local_workflows
                    .iter()
                    .filter(|w| w.def.steps.iter().any(|s| s.bot_id == local.id))
                    .map(|w| w.name.clone())
                    .collect();
            }
            if let Some(before) = local_file.as_ref().filter(|_| entry.item.status != ItemStatus::Unchanged) {
                let mut normalized = input.clone();
                if let Some(s) = bot_steps.iter().find(|s| s.file_handle == *key) {
                    normalized.provider = s.new.provider.as_str().to_string();
                    normalized.name = s.new.display_name.clone();
                    normalized.color = s.new.color.clone();
                    normalized.model = s.new.model.clone();
                    normalized.instructions = s.new.instructions.clone();
                    normalized.skills = s.skills.clone();
                    if let Some((range, _)) = find_checkout_paragraph(&normalized.instructions) {
                        normalized.instructions.replace_range(range, &checkout_paragraph(None));
                    }
                }
                entry.changed_fields =
                    fields(before, &normalized, &["name", "color", "provider", "model", "instructions", "skills"]);
            }
            let use_local = entry.action == WorkflowImportAction::Reuse && local_file.is_some();
            let chosen = if use_local { local_file.as_ref().unwrap() } else { input };
            if use_local {
                entry.item.target = local.as_ref().unwrap().handle.clone();
            }
            for slug in &chosen.skills {
                let handle = if entry.item.target.is_empty() { &input.handle } else { &entry.item.target };
                needed_skills.entry(slug.clone()).or_default().push(format!("@{handle}"));
            }
            resolved.insert(key.clone(), entry.item.target.clone());
            if matches!(entry.action, WorkflowImportAction::Add | WorkflowImportAction::Replace) {
                if let Some(index) = bot_steps.iter().position(|s| s.file_handle == *key) {
                    selected_bots.push(bot_steps.remove(index));
                }
            }
            items.push(entry);
        }

        let mut selected_skills = Vec::new();
        for (slug, mut required_by) in needed_skills {
            required_by.sort();
            required_by.dedup();
            let local = store.get_skill_by_slug(&slug)?;
            let local_file = local.as_ref().map(|s| self.file_skill(s));
            fingerprint(&mut hash, &local);
            fingerprint(&mut hash, &local_file.as_ref().map(|r| r.as_ref().map_err(error_value)));
            let can_keep =
                local_file.as_ref().is_some_and(|r| r.as_ref().is_ok_and(|f| config_file::decode_skill(f).is_ok()));
            let (planned, step, choice) = if let Some(skill) = candidate_skills.get(slug.as_str()) {
                let (item, step) = self.plan_skill(skill)?;
                (item, step, request.skills.get(&slug).copied())
            } else {
                let item = if can_keep {
                    PlannedItem { name: slug.clone(), target: slug.clone(), status: ItemStatus::Unchanged }
                } else {
                    match &local_file {
                        Some(Ok(file)) => invalid(
                            &slug,
                            &config_file::decode_skill(file)
                                .err()
                                .expect("a local skill that cannot be kept is invalid"),
                        ),
                        Some(Err(error)) => {
                            let error = error_value(error);
                            PlannedItem {
                                name: slug.clone(),
                                target: slug.clone(),
                                status: ItemStatus::Invalid {
                                    code: error["code"].as_str().unwrap().to_string(),
                                    params: error["params"].clone(),
                                },
                            }
                        }
                        None => invalid(&slug, &Refusal::SkillUnknown(slug.clone())),
                    }
                };
                (item, None, Some(DependencyChoice::KeepLocal))
            };
            let mut entry = row(ItemKind::Skill, planned, choice, can_keep);
            entry.required_by = required_by;
            if entry.action == WorkflowImportAction::Reuse {
                entry.item.target = slug.clone();
            }
            if let Some(local) = &local {
                entry.used_by_bots = store.skill_user_labels(&local.id)?;
                let all_bots = store.unscoped().list_bots()?;
                let ids: HashSet<&str> =
                    all_bots.iter().filter(|b| b.skill_ids.contains(&local.id)).map(|b| b.id.as_str()).collect();
                let projects = store.list_projects()?;
                entry.used_by_workflows = store
                    .unscoped()
                    .list_workflows()?
                    .iter()
                    .filter(|w| w.def.steps.iter().any(|s| ids.contains(s.bot_id.as_str())))
                    .map(|w| {
                        if projects.len() > 1 {
                            let project =
                                projects.iter().find(|p| p.id == w.project_id).expect("workflow project exists");
                            format!("{} · {}", w.name, project.name)
                        } else {
                            w.name.clone()
                        }
                    })
                    .collect();
            }
            if let Some(step) = &step {
                if let Some(Ok(before)) = &local_file {
                    entry.changed_fields =
                        [("name", before.name != step.name), ("description", before.description != step.description)]
                            .into_iter()
                            .filter(|(_, changed)| *changed)
                            .map(|(name, _)| name.to_string())
                            .collect();
                    let after: Vec<_> = step.blobs.iter().map(config_file::to_entry).collect();
                    let paths: BTreeSet<&str> = before.files.iter().chain(&after).map(|f| f.path.as_str()).collect();
                    entry.changed_files = paths
                        .into_iter()
                        .filter(|p| before.files.iter().find(|f| f.path == *p) != after.iter().find(|f| f.path == *p))
                        .map(str::to_string)
                        .collect();
                } else {
                    entry.changed_files = step.blobs.iter().map(|b| b.path.clone()).collect();
                }
            }
            if matches!(entry.action, WorkflowImportAction::Add | WorkflowImportAction::Replace) {
                selected_skills.extend(step);
            }
            items.push(entry);
        }

        let known: HashSet<&str> = resolved.values().filter(|h| !h.is_empty()).map(String::as_str).collect();
        let new = resolved_workflow(
            &root,
            |handle| {
                Ok(handle
                    .and_then(|h| root_keys.get(&h.to_lowercase()))
                    .and_then(|k| resolved.get(k))
                    .cloned()
                    .unwrap_or_default())
            },
            &known,
        );
        let mut incoming_comparison = source.clone();
        incoming_comparison.name = incoming_comparison.name.trim().to_string();
        for step in &mut incoming_comparison.definition.steps {
            if let Some(h) = &step.bot {
                if !keep_root {
                    if let Some(target) = root_keys.get(&h.to_lowercase()).and_then(|key| resolved.get(key)) {
                        step.bot = Some(target.clone());
                    }
                } else if let Some(Some(b)) = matches.get(&h.to_lowercase()) {
                    step.bot = Some(b.handle.clone());
                }
            }
        }
        let same = local_workflow.as_ref() == Some(&incoming_comparison);
        let status = if request.new_name.is_some() || existing.is_none() {
            ItemStatus::New
        } else if same {
            ItemStatus::Unchanged
        } else {
            ItemStatus::Different
        };
        let mut workflow_item = PlannedItem { name: source.name.clone(), target: root.name.clone(), status };
        let mut graph_error = new.as_ref().err().cloned();
        if request.new_name.is_some() && store.get_workflow_by_name(&root.name)?.is_some() {
            graph_error = Some(Refusal::WorkflowNameTaken(root.name.clone()));
        }
        if let Some(e) = graph_error {
            workflow_item.status = invalid(&source.name, &e).status;
        }
        let choice = if request.new_name.is_some() { Some(DependencyChoice::Replace) } else { request.workflow_choice };
        let can_keep = existing.is_some() && (keep_root && new.is_ok() || !keep_root);
        let mut workflow_row = row(ItemKind::Workflow, workflow_item, choice, can_keep);
        if matches!(workflow_row.item.status, ItemStatus::Invalid { .. }) && (keep_root || request.new_name.is_some()) {
            workflow_row.action = WorkflowImportAction::Blocked;
        }
        if let Some(before) = &local_workflow {
            workflow_row.changed_fields = fields(before, &incoming_comparison, &["name", "when_to_use", "definition"]);
        }
        let mut workflows = Vec::new();
        if matches!(workflow_row.action, WorkflowImportAction::Add | WorkflowImportAction::Replace) {
            if let Ok(new) = new {
                workflows
                    .push(WorkflowStep { new, existing: if request.new_name.is_some() { None } else { existing } });
            }
        }
        items.insert(0, workflow_row);
        fingerprint(&mut hash, &items);
        let can_import =
            items.iter().all(|i| !matches!(i.action, WorkflowImportAction::Choose | WorkflowImportAction::Blocked));
        Ok(WorkflowPlan {
            preview: WorkflowImportPreview { token: format!("{:x}", hash.finalize()), items, can_import },
            skills: selected_skills,
            bots: selected_bots,
            workflows,
        })
    }
}
