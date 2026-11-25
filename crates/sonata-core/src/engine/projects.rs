use super::*;
use crate::model::{Project, ProjectInput};

fn session_blocks_archive(state: &Live) -> bool {
    state.in_flight.is_some()
        || state.limited_turn.is_some()
        || matches!(state.status, BotStatus::Starting | BotStatus::Working | BotStatus::Limited)
}

fn project_runtimes(live: &HashMap<String, Live>, bots: &HashSet<String>) -> Vec<(String, RuntimeBot)> {
    let mut running: Vec<_> = live
        .iter()
        .filter(|(_, state)| bots.contains(&state.bot_id))
        .filter_map(|(id, state)| Some((id.clone(), state.rt.clone()?)))
        .collect();
    running.sort_by(|left, right| left.0.cmp(&right.0));
    running
}

impl Engine {
    pub fn for_project(&self, project_id: &str) -> Result<Self> {
        self.store.get_project(project_id)?;
        Ok(Self { store: self.store.scoped(project_id), inner: self.inner.clone() })
    }

    pub fn for_session(&self, session_id: &str) -> Result<Self> {
        let session = self
            .store
            .unscoped()
            .get_session(session_id)?
            .ok_or_else(|| Refusal::SessionNotFound(session_id.into()))?;
        self.for_project(&session.project_id)
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        self.store.list_projects()
    }
    pub fn current_project(&self) -> Result<Project> {
        self.store.get_project(self.store.project_key())
    }
    pub fn resolve_bot_provider(&self, provider: Option<Provider>, existing: Option<&str>) -> Result<Provider> {
        if let Some(provider) = provider {
            return Ok(provider);
        }
        if let Some(id) = existing {
            return self.store.get_bot(id)?.map(|b| b.provider).ok_or_else(|| Refusal::BotNotFound(id.into()).into());
        }
        self.default_provider()
    }
    pub fn default_provider(&self) -> Result<Provider> {
        Ok(self.current_project()?.default_provider)
    }

    fn checked_project(&self, mut input: ProjectInput, editing: Option<&Project>) -> Result<ProjectInput> {
        input.name = input.name.trim().to_string();
        if input.name.is_empty() {
            return Err(CoreError::from(crate::ProjectError::NameRequired));
        }
        if self
            .list_projects()?
            .iter()
            .any(|p| p.name.to_lowercase() == input.name.to_lowercase() && editing.is_none_or(|e| e.id != p.id))
        {
            return Err(CoreError::from(crate::ProjectError::NameTaken));
        }
        input.default_directory = input.default_directory.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
        // Preserve an unavailable legacy directory when changing only name/provider.
        if editing.is_none_or(|old| old.default_directory != input.default_directory) {
            let path = input
                .default_directory
                .as_deref()
                .map(Path::new)
                .ok_or_else(|| CoreError::from(crate::ProjectError::DirectoryRequired))?;
            if !path.is_absolute() || !path.is_dir() {
                return Err(Refusal::BadFolder(path.display().to_string()).into());
            }
        }
        if adapter_for(input.default_provider).is_none() {
            return Err(Refusal::ProviderUnavailable(input.default_provider.as_str().into()).into());
        }
        Ok(input)
    }

    pub fn create_project(&self, input: ProjectInput) -> Result<Project> {
        let _creating = self.inner.importing.lock().unwrap();
        let input = self.checked_project(input, None)?;
        let master = self.new_master(input.default_directory.as_deref().map(Path::new), input.default_provider)?;
        let id = new_id();
        let parent = self.bots_root().join(".projects");
        std::fs::create_dir_all(&parent)?;
        let workspace = parent.join(&id);
        std::fs::create_dir(&workspace)?;
        let master_dir = workspace.join(crate::bundled::MASTER.handle);
        let created = std::fs::create_dir(&master_dir)
            .map_err(CoreError::from)
            .and_then(|()| self.store.insert_project(&id, &input, &master));
        let project = match created {
            Ok(project) => project,
            Err(error) => {
                let _ = std::fs::remove_dir(&master_dir);
                let _ = std::fs::remove_dir(&workspace);
                return Err(error);
            }
        };
        self.emit(UiEvent::ProjectsChanged);
        Ok(project)
    }

    pub fn update_project(&self, id: &str, input: ProjectInput) -> Result<Project> {
        let before = self.store.get_project(id)?;
        let input = self.checked_project(input, Some(&before))?;
        let scoped = self.for_project(id)?;
        scoped.store.require_active()?;
        if input.default_directory != before.default_directory {
            if let Some(path) = &input.default_directory {
                scoped.set_project_folder(Path::new(path))?;
            }
        }
        let project = self.store.update_project(id, &input)?;
        self.emit(UiEvent::ProjectsChanged);
        Ok(project)
    }

    pub fn finish_project_setup(&self, input: ProjectInput) -> Result<Project> {
        // The migration reserves Default; completing setup configures it rather than adding a duplicate.
        let id = crate::store::DEFAULT_PROJECT;
        let input = self.checked_project(input, Some(&self.store.get_project(id)?))?;
        if input.default_directory.is_none() {
            return Err(CoreError::from(crate::ProjectError::DirectoryRequired));
        }
        let project = self.update_project(id, input)?;
        self.for_project(id)?.finish_setup(project.default_directory.as_deref().map(Path::new))?;
        self.emit(UiEvent::ProjectsChanged);
        self.store.get_project(id)
    }

    pub async fn archive_project(&self, id: &str) -> Result<Project> {
        let scoped = self.for_project(id)?;
        // Screen reads wait on the terminal. `live` is process-wide, so copy the sessions and drop it first.
        for _ in 0..8 {
            let bots: HashSet<String> = scoped.store.list_bots()?.into_iter().map(|bot| bot.id).collect();
            let running = {
                let live = self.inner.live.lock().await;
                if live.values().any(|state| bots.contains(&state.bot_id) && session_blocks_archive(state)) {
                    return Err(CoreError::from(crate::ProjectError::ActiveWork));
                }
                project_runtimes(&live, &bots)
            };
            for (session, rt) in &running {
                match scoped.background_work(session, rt).await {
                    Ok(false) => {}
                    Ok(true) => return Err(CoreError::from(crate::ProjectError::BackgroundWork)),
                    Err(error) => return Err(Refusal::SnapshotFailed(error.to_string()).into()),
                }
            }
            let mut stops = Vec::new();
            let archived = {
                let mut live = self.inner.live.lock().await;
                if live.values().any(|state| bots.contains(&state.bot_id) && session_blocks_archive(state)) {
                    return Err(CoreError::from(crate::ProjectError::ActiveWork));
                }
                if project_runtimes(&live, &bots) != running {
                    None
                } else {
                    let project = self.store.archive_project(id)?;
                    let sessions: Vec<String> = live
                        .iter()
                        .filter(|(_, state)| bots.contains(&state.bot_id))
                        .map(|(session, _)| session.clone())
                        .collect();
                    for session in sessions {
                        stops.push(scoped.stop_locked(&mut live, &session)?);
                    }
                    Some(project)
                }
            };
            let Some(project) = archived else { continue };
            for stop in stops {
                scoped.stopped(stop).await;
            }
            self.emit(UiEvent::ProjectsChanged);
            return Ok(project);
        }
        Err(CoreError::from(crate::ProjectError::ActiveWork))
    }

    pub fn restore_project(&self, id: &str) -> Result<Project> {
        let scoped = self.for_project(id)?;
        if scoped.current_project()?.archived_at.is_none() {
            return scoped.current_project();
        }
        let mut next = Vec::new();
        for task in scoped.list_tasks()?.into_iter().filter(|t| t.enabled) {
            next.push((
                task.id.clone(),
                tasks::stamp(Self::schedule_of(&task)?.first_slot_after(self.now_utc(), &chrono::Local)),
            ));
        }
        let project = self.store.restore_project(id, &next)?;
        self.emit(UiEvent::ProjectsChanged);
        Ok(project)
    }

    fn workspace_for(&self, project_id: &str) -> Result<PathBuf> {
        if self.store.legacy_workspace_of(project_id)? {
            Ok(self.bots_root())
        } else {
            Ok(self.bots_root().join(".projects").join(project_id))
        }
    }

    pub(super) fn bot_directory(&self, bot: &Bot) -> Result<PathBuf> {
        Ok(self.workspace_for(&bot.project_id)?.join(&bot.handle))
    }

    pub(super) fn project_workspace(&self) -> Result<PathBuf> {
        self.workspace_for(self.store.project_key())
    }
}

impl Engine {
    /// Desktop commands carry a project explicitly; ID-only commands resolve their stored owner.
    pub fn request_project(&self, project: Option<&str>, targets: &[(&str, &str)]) -> Result<Self> {
        let mut owner = project.map(str::to_string);
        for (table, id) in targets {
            if let Some(found) = self.store.entity_project(table, id)? {
                if owner.as_ref().is_some_and(|owner| owner != &found) {
                    return Err(CoreError::from(crate::ProjectError::Mismatch));
                }
                owner = Some(found);
            }
        }
        match owner {
            Some(id) => {
                let scoped = self.for_project(&id)?;
                scoped.store.require_active()?;
                Ok(scoped)
            }
            None => Ok(self.clone()),
        }
    }

    pub fn event_project(&self, event: &UiEvent) -> Option<String> {
        let target = match event {
            UiEvent::OrchestrationChanged { project_id, .. } => return Some(project_id.clone()),
            UiEvent::BotAdded { bot } | UiEvent::BotUpdated { bot } => return Some(bot.bot.project_id.clone()),
            UiEvent::SessionAdded { session } | UiEvent::SessionUpdated { session } => {
                return Some(session.session.project_id.clone())
            }
            UiEvent::MessageAdded { message } | UiEvent::MessageUpdated { message } => {
                ("messages", message.id.as_str())
            }
            UiEvent::BotStatus { bot_id, .. }
            | UiEvent::SessionStatus { bot_id, .. }
            | UiEvent::SessionRemoved { bot_id, .. } => ("bots", bot_id.as_str()),
            UiEvent::WorkflowRunChanged { run_id, .. } => ("workflow_runs", run_id.as_str()),
            _ => return None,
        };
        self.store.entity_project(target.0, target.1).ok().flatten()
    }
}
