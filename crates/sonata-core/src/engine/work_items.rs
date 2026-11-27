use super::*;
use crate::orchestration::{self as orch, invalid, Assignment, TaskSpec};
use crate::work_items::{NewWorkItem, WorkItem};

impl Engine {
    pub fn archive_work_item(&self, id: &str, archived: bool) -> Result<()> {
        for session in self.store.archive_work_item(id, archived)? {
            self.emit(UiEvent::SessionUpdated { session: self.session_view(session) });
        }
        self.emit(UiEvent::WorkItemsChanged);
        Ok(())
    }

    pub async fn cancel_work_item(&self, id: &str) -> Result<()> {
        let items = self.store.list_work_items()?;
        let item = items.iter().find(|w| w.id == id).ok_or_else(|| invalid("Task not found."))?;
        if item.state == "finished" {
            return Ok(());
        }
        let mut ids = HashSet::from([id.to_string()]);
        loop {
            let before = ids.len();
            for w in &items {
                if w.parent_id.as_ref().is_some_and(|p| ids.contains(p)) {
                    ids.insert(w.id.clone());
                }
            }
            if ids.len() == before {
                break;
            }
        }
        for run in self.store.list_orchestrations()?.into_iter().rev() {
            if self.store.work_item_of_session(&run.session_id)?.is_some_and(|id| ids.contains(&id)) {
                self.cancel_orchestration(&run.id, None).await?;
            }
        }
        // A pending subtree may have no sessions or orchestration yet.
        for child in
            items.iter().filter(|w| ids.contains(&w.id) && w.state == "pending" && w.orchestration_id.is_none())
        {
            self.store.cancel_pending_work_item(&child.id)?;
        }
        if let (Some(run), Some(execution)) = (&item.orchestration_id, &item.execution) {
            self.cancel_orchestration(run, Some(&execution.spec.key)).await?;
        } else if item.state == "pending" {
            self.store.cancel_pending_work_item(id)?;
        } else if item.primary_session_id.is_some() {
            self.store.cancel_work_item_record(id)?;
            let sessions = item
                .session_ids
                .iter()
                .map(|id| self.store.get_session(id))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            let runs = self.stop_runs_of_sessions(&sessions, "cancelled")?;
            self.wind_down(&runs).await;
            for session in sessions {
                self.stop_session(&session.id).await?;
            }
        }
        self.emit(UiEvent::WorkItemsChanged);
        Ok(())
    }

    pub(super) fn finish_waiting_task(
        &self,
        session: &str,
        body: &str,
        files: &[String],
        outcome: Option<String>,
    ) -> Result<Message> {
        let s = self.store.get_session(session)?.ok_or_else(|| invalid("Task conversation not found."))?;
        let bot = self.store.get_bot(&s.bot_id)?.ok_or_else(|| invalid("Bot not found."))?;
        let checked = attachments::check(files, &self.bot_directory(&bot)?).map_err(Refusal::Attachment)?;
        let stored = attachments::in_place(&checked);
        if let Some((run, visit)) = self.store.open_visit_of_session(session)? {
            self.move_on(&run, &visit, &bot, session, body.into(), stored, outcome, None)?;
            return self
                .store
                .get_message(
                    &self
                        .store
                        .run_visits(&run.id)?
                        .into_iter()
                        .find(|v| v.id == visit.id)
                        .and_then(|v| v.result_message)
                        .ok_or_else(|| invalid("Missing workflow result."))?,
                )?
                .ok_or_else(|| invalid("Missing workflow result."));
        }
        let own = self.store.own_orchestration(session)?;
        let coordinated = own.map(|r| self.finish_orchestration(&r.id, r.revision, body, files, None)).transpose()?;
        match coordinated {
            Some(message) => Ok(message),
            None => self.store.finish_standalone_task(session, body, &stored),
        }
    }

    pub(super) fn reconcile_work_items(&self) -> Result<()> {
        // Each pass removes at least one durable result; nesting has no fixed depth limit.
        loop {
            let items = self.store.list_work_items()?;
            let parents: HashSet<_> = items
                .iter()
                .filter(|w| w.state == "pending" && w.orchestration_id.is_none())
                .filter_map(|w| w.parent_id.clone())
                .collect();
            for parent in parents {
                let Some(item) = items.iter().find(|w| w.id == parent) else { continue };
                let engine = self.for_project(&item.project_id)?;
                if engine.current_project()?.archived_at.is_none() {
                    engine.attach_pending_children(&parent)?;
                }
            }
            for run in self.store.list_orchestrations()? {
                if run.status == orch::Status::Active {
                    self.for_project(&run.project_id)?.dispatch_orchestration(&run.id)?;
                }
            }
            for item in self.store.list_work_items()?.into_iter().filter(|w| {
                w.state == "pending" && !w.manual_start && w.orchestration_id.is_none() && w.parent_id.is_none()
            }) {
                let engine = self.for_project(&item.project_id)?;
                if engine.current_project()?.archived_at.is_some() {
                    continue;
                }
                match engine.start_work_item(&item.id) {
                    Ok(()) => {}
                    Err(e @ (CoreError::Db(_) | CoreError::Json(_))) => return Err(e),
                    Err(e) => engine.store.fail_pending_work_item(&item.id, &e.to_string())?,
                }
            }
            let ready = self.store.ready_work_item_results()?;
            if ready.is_empty() {
                break;
            }
            for (session, body, files, outcome) in ready {
                let message = self.finish_waiting_task(&session, &body, &files, outcome)?;
                self.emit_message(message);
            }
        }
        self.emit(UiEvent::WorkItemsChanged);
        Ok(())
    }

    pub async fn list_work_items(&self) -> Result<Vec<WorkItem>> {
        let mut items = self.store.list_work_items()?;
        let ids = items.iter().flat_map(|w| w.session_ids.clone()).collect::<Vec<_>>();
        let activity = self.session_activity(&ids).await?;
        let live = self.inner.live.lock().await;
        for item in &mut items {
            if item.archived_at.is_some() {
                item.state = "done".into();
                continue;
            }
            if item.state == "working"
                && item.session_ids.iter().any(|id| {
                    self.store.get_session(id).ok().flatten().is_some_and(|s| s.waiting_question_id.is_some())
                        || live.get(id).is_some_and(|l| l.status == BotStatus::NeedsAttention)
                        || activity.iter().any(|a| {
                            a.session_id == *id
                                && (a.cut_off.is_some()
                                    || a.workflow.as_ref().is_some_and(|w| w.status == RunStatus::Paused))
                        })
                })
            {
                item.state = "needs_you".into();
            }
        }
        drop(live);
        loop {
            let parents: HashSet<String> =
                items.iter().filter(|w| w.state == "needs_you").filter_map(|w| w.parent_id.clone()).collect();
            let mut changed = false;
            for item in &mut items {
                if parents.contains(&item.id) && item.state == "working" {
                    item.state = "needs_you".into();
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        Ok(items)
    }

    pub fn create_work_item(&self, input: NewWorkItem) -> Result<WorkItem> {
        self.store.require_active()?;
        let default_title = session_title(&input.instructions).ok_or_else(|| invalid("Instructions are required."))?;
        let title = match input.title.trim() {
            "" => default_title,
            title => title.into(),
        };
        let key = new_id();
        let items = self.store.list_work_items()?;
        let parent = input
            .parent_id
            .as_ref()
            .map(|id| items.iter().find(|w| &w.id == id).ok_or_else(|| invalid("Parent task not found.")))
            .transpose()?;
        let mut spec = TaskSpec {
            key: key.clone(),
            title,
            instructions: input.instructions,
            acceptance_criteria: vec!["Complete the requested work and report the result.".into()],
            executor: input.executor,
            depends_on: vec![],
            files: vec![],
            manual_start: input.manual_start,
        };
        if let Some(parent) = parent {
            if parent.archived_at.is_some() {
                return Err(invalid("Restore the parent before adding work."));
            }
            for dep in &input.depends_on {
                let sibling = items
                    .iter()
                    .find(|w| &w.id == dep && w.parent_id == input.parent_id)
                    .ok_or_else(|| invalid("Dependencies must be sibling tasks."))?;
                spec.depends_on.push(crate::work_items::dependency_key(&sibling.id));
            }
        } else if !input.depends_on.is_empty() {
            return Err(invalid("Dependencies require a common parent."));
        }
        // Resolve and snapshot the executor without starting a bot or manufacturing a session.
        let assignment = match &spec.executor {
            orch::Executor::Bot { handle } => Assignment::Bot {
                bot_id: self
                    .store
                    .get_bot_by_handle(handle.trim_start_matches('@'))?
                    .ok_or_else(|| invalid("Bot not found."))?
                    .id,
            },
            orch::Executor::Workflow { name } => {
                let w = self.store.get_workflow_by_name(name)?.ok_or_else(|| invalid("Workflow not found."))?;
                self.check_runnable(&NewWorkflow {
                    name: w.name.clone(),
                    when_to_use: w.when_to_use.clone(),
                    def: w.def.clone(),
                })?;
                Assignment::Workflow { workflow_id: w.id, name: w.name, definition: w.def }
            }
        };
        let files = attachments::check(&input.files, Path::new("/")).map_err(Refusal::Attachment)?;
        let id = new_id();
        let input_dir = self.inner.cfg.paths.data_dir.join("task-inputs").join(self.store.project_key()).join(&id);
        spec.files = attachments::snapshot_task_inputs(&files, &input_dir).map_err(Refusal::Attachment)?;
        let inserted = self.store.insert_pending_work_item(
            &id,
            &orch::Task::new(spec, assignment),
            input.parent_id.as_deref(),
            &input.depends_on,
        );
        if inserted.is_err() && !files.is_empty() {
            let _ = std::fs::remove_dir_all(&input_dir);
        }
        let item = inserted?;
        if let Some(parent) = &input.parent_id {
            self.attach_pending_children(parent)?;
        } else if !item.manual_start {
            self.start_work_item(&item.id)?;
        }
        self.emit(UiEvent::WorkItemsChanged);
        self.store.list_work_items()?.into_iter().find(|w| w.id == item.id).ok_or_else(|| invalid("Task not found."))
    }

    pub(super) fn attach_pending_children(&self, parent_id: &str) -> Result<()> {
        let items = self.store.list_work_items()?;
        let Some(parent) = items.iter().find(|w| w.id == parent_id) else { return Ok(()) };
        let active_session = parent
            .workflow_run_id
            .as_ref()
            .map(|id| self.store.run_visits(id))
            .transpose()?
            .unwrap_or_default()
            .into_iter()
            .rev()
            .find(|v| v.ended_at.is_none())
            .and_then(|v| v.session_id);
        let Some(session_id) = active_session.as_deref().or(parent.primary_session_id.as_deref()) else {
            return Ok(());
        };
        let children = items
            .iter()
            .filter(|w| {
                w.parent_id.as_deref() == Some(parent_id) && w.state == "pending" && w.orchestration_id.is_none()
            })
            .filter_map(|w| w.execution.clone())
            .collect::<Vec<_>>();
        if children.is_empty() {
            return Ok(());
        }
        let run = if let Some(run) = self.store.own_orchestration(session_id)? {
            let key = children[0].spec.key.clone();
            self.store.update_orchestration(
                &run.id,
                run.revision,
                &format!("attach:{key}"),
                &key,
                children,
                vec![],
                &[],
                None,
            )?
        } else {
            let message = self.queue_user_message_to_session(
                session_id,
                "Coordinate the planned subtasks and report their results.",
                &[],
            )?;
            let run = orch::Orchestration {
                id: new_id(),
                project_id: parent.project_id.clone(),
                session_id: session_id.into(),
                request_message_id: message.id,
                title: parent.title.clone(),
                goal: if parent.instructions.is_empty() { parent.title.clone() } else { parent.instructions.clone() },
                revision: 1,
                status: orch::Status::Active,
                tasks: children,
                final_message_id: None,
                final_report: None,
                final_files: vec![],
                created_at: crate::ids::now(),
                ended_at: None,
            };
            self.store.create_orchestration(&run, &format!("user-plan:{}", run.id), &run.id)?
        };
        self.dispatch_orchestration(&run.id)?;
        self.emit_orchestration(&run);
        tokio::spawn(self.pump_session(session_id));
        Ok(())
    }

    pub fn start_work_item(&self, id: &str) -> Result<()> {
        self.store.require_active()?;
        let item =
            self.store.list_work_items()?.into_iter().find(|w| w.id == id).ok_or_else(|| invalid("Task not found."))?;
        if item.state != "pending" {
            return Ok(());
        }
        if item.parent_id.is_some() && item.orchestration_id.is_none() {
            self.store.release_pending_work_item(id)?;
            self.attach_pending_children(item.parent_id.as_deref().unwrap())?;
            return Ok(());
        }
        if item.orchestration_id.is_some() {
            let run = self.store.release_work_item(id)?;
            self.dispatch_orchestration(&run)?;
        } else {
            self.store.release_pending_work_item(id)?;
            if self.paused()? {
                self.emit(UiEvent::WorkItemsChanged);
                return Ok(());
            }
            let task = item.execution.ok_or_else(|| invalid("Task has no executor."))?;
            let checked = attachments::check(&task.spec.files, Path::new("/")).map_err(Refusal::Attachment)?;
            let routed = match &task.assignment {
                Assignment::Bot { bot_id } => {
                    let bot =
                        self.store.get_bot(bot_id)?.ok_or_else(|| invalid("The assigned bot no longer exists."))?;
                    let copies = if checked.is_empty() {
                        vec![]
                    } else {
                        attachments::copy_to_inbox(&checked, &self.inbox_bot_dir(&bot.handle)?, &new_id())
                            .map_err(Refusal::Attachment)?
                    };
                    let result = self.store.start_pending_work_item(id, Some(bot_id), None, &copies);
                    if !matches!(result, Ok(Some(_))) {
                        for copy in copies {
                            let _ = std::fs::remove_file(copy.path);
                        }
                    }
                    result?
                }
                Assignment::Workflow { workflow_id, name, definition } => {
                    let run = WorkflowRun {
                        project_id: item.project_id,
                        id: new_id(),
                        workflow_id: Some(workflow_id.clone()),
                        name: name.clone(),
                        def: definition.clone(),
                        request: task.spec.instructions.clone(),
                        request_files: task.spec.files.clone(),
                        opener_session: None,
                        root_session: None,
                        status: RunStatus::Running,
                        pause: None,
                        extra_visits: BTreeMap::new(),
                        created_at: crate::ids::now(),
                        ended_at: None,
                    };
                    let first = self.visit_open(
                        &run,
                        definition.start.as_deref().ok_or_else(|| invalid("Workflow has no start."))?,
                        1,
                        None,
                        &[],
                    )?;
                    let new = crate::store::NewRun {
                        id: run.id.clone(),
                        workflow_id: workflow_id.clone(),
                        name: name.clone(),
                        def: definition.clone(),
                        request: run.request,
                        request_files: run.request_files,
                        opener_session: None,
                    };
                    let result = self.store.start_pending_work_item(id, None, Some((&new, &first)), &[]);
                    if !matches!(result, Ok(Some(_))) {
                        for copy in &first.message.attachments {
                            let _ = std::fs::remove_file(&copy.path);
                        }
                    }
                    result?
                }
            };
            if let Some(routed) = routed {
                self.emit_routed(&routed);
                self.attach_pending_children(id)?;
                tokio::spawn(self.pump_session(&routed.session.id));
            }
        }
        self.emit(UiEvent::WorkItemsChanged);
        Ok(())
    }
}
