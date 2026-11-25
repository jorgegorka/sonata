use super::master::on_users_turn;
use super::messaging::refuse;
use super::*;
use crate::ids::now;
use crate::orchestration::{
    self as orch, invalid, Assignment, Executor, Orchestration, OrchestrationView, Plan, Status, TaskState, TaskView,
    Update,
};
use bridge_proto::{BridgeCommand, BridgeResponse, ErrorCode};
use sha2::{Digest, Sha256};

pub const GUIDANCE: &str = include_str!("../../bundled/master/orchestration.md");

fn fingerprint<T: Serialize>(input: &T) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(input)?)))
}

impl Engine {
    fn prepare_orchestration_tasks(&self, session: &Session, specs: Vec<orch::TaskSpec>) -> Result<Vec<orch::Task>> {
        let master = self.store.get_bot(&session.bot_id)?.ok_or_else(|| invalid("Master is missing."))?;
        let directory = self.bot_directory(&master)?;
        specs
            .into_iter()
            .map(|mut spec| {
                spec.files = attachments::check(&spec.files, &directory)
                    .map_err(Refusal::Attachment)?
                    .into_iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                let assignment = match &spec.executor {
                    Executor::Bot { handle } => {
                        let bot = self
                            .store
                            .get_bot_by_handle(handle.trim_start_matches('@'))?
                            .ok_or_else(|| invalid(format!("Bot {handle} does not exist in this project.")))?;
                        Assignment::Bot { bot_id: bot.id }
                    }
                    Executor::Workflow { name } => {
                        let workflow = self
                            .store
                            .get_workflow_by_name(name)?
                            .ok_or_else(|| invalid(format!("Workflow {name} does not exist in this project.")))?;
                        let input = NewWorkflow {
                            name: workflow.name.clone(),
                            when_to_use: workflow.when_to_use,
                            def: workflow.def.clone(),
                        };
                        crate::workflow::check_workflow(&input, |id| self.store.get_bot(id).ok().flatten().is_some())?;
                        Assignment::Workflow { workflow_id: workflow.id, name: workflow.name, definition: workflow.def }
                    }
                };
                Ok(orch::Task::new(spec, assignment))
            })
            .collect()
    }

    pub fn start_orchestration(&self, session_id: &str, request: &str, key: &str, plan: Plan) -> Result<Orchestration> {
        self.store.require_active()?;
        let session = self.store.get_session(session_id)?.ok_or_else(|| invalid("Conversation not found."))?;
        if !self.is_master(&session.bot_id)? {
            return Err(invalid("Only Master can orchestrate tasks."));
        }
        let fp = fingerprint(&("start", request, &plan))?;
        let operation = format!("start:{key}");
        if let Some(run) = self.store.orchestration_replay(session_id, &operation, &fp)? {
            self.dispatch_orchestration(&run.id)?;
            return self.store.get_orchestration(&run.id);
        }
        let message = self.store.get_message(request)?.ok_or_else(|| invalid("Original request not found."))?;
        if message.recipient_session.as_deref() != Some(session_id)
            || message.sender != USER
            || message.task_id.is_some()
            || message.workflow_mark.is_some()
            || is_event(&message)
        {
            return Err(invalid("Start orchestration on the user's own request."));
        }
        if self.store.orchestration_of_session(session_id)?.is_some_and(|(_, task)| task.is_some()) {
            return Err(invalid("Nested orchestrations are not supported."));
        }
        if key.trim().is_empty() || plan.title.trim().is_empty() || plan.goal.trim().is_empty() {
            return Err(invalid("An operation key, title and goal are required."));
        }
        let tasks = self.prepare_orchestration_tasks(&session, plan.tasks)?;
        orch::validate_graph(&tasks)?;
        let run = Orchestration {
            id: new_id(),
            project_id: session.project_id,
            session_id: session.id,
            request_message_id: request.into(),
            title: plan.title,
            goal: plan.goal,
            revision: 1,
            status: Status::Active,
            tasks,
            final_message_id: None,
            final_report: None,
            final_files: vec![],
            created_at: now(),
            ended_at: None,
        };
        let run = self.store.create_orchestration(&run, &operation, &fp)?;
        self.emit_orchestration(&run);
        self.dispatch_orchestration(&run.id)?;
        self.store.get_orchestration(&run.id)
    }

    pub fn update_orchestration(
        &self,
        id: &str,
        revision: i64,
        key: &str,
        input: Update,
        answered: Option<&str>,
    ) -> Result<Orchestration> {
        self.store.require_active()?;
        let run = self.store.get_orchestration(id)?;
        if key.trim().is_empty() {
            return Err(invalid("An operation key is required."));
        }
        if input.add.is_empty() && input.replace.is_empty() && input.reviews.is_empty() {
            return Err(invalid("An update must add tasks, replace pending tasks or review results."));
        }
        let fp = fingerprint(&("update", id, revision, &input))?;
        let operation = format!("update:{id}:{key}");
        if self.store.orchestration_replay(&run.session_id, &operation, &fp)?.is_some() {
            self.dispatch_orchestration(id)?;
            return self.store.get_orchestration(id);
        }
        let session =
            self.store.get_session(&run.session_id)?.ok_or_else(|| invalid("Master conversation not found."))?;
        let add = self.prepare_orchestration_tasks(&session, input.add)?;
        let replace = self.prepare_orchestration_tasks(&session, input.replace)?;
        let run =
            self.store.update_orchestration(id, revision, &operation, &fp, add, replace, &input.reviews, answered)?;
        self.emit_orchestration(&run);
        self.dispatch_orchestration(id)?;
        self.wake_orchestration(&run)?;
        self.store.get_orchestration(id)
    }

    fn task_brief(&self, run: &Orchestration, task: &orch::Task) -> (String, Vec<String>) {
        let mut body = format!(
            "Overall goal: {}\n\nTask: {}\n{}\n\nAcceptance criteria:\n{}",
            run.goal,
            task.spec.title,
            task.spec.instructions,
            task.spec
                .acceptance_criteria
                .iter()
                .enumerate()
                .map(|(i, c)| format!("{}. {c}", i + 1))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let mut files = task.spec.files.clone();
        for key in &task.spec.depends_on {
            if let Some(prerequisite) = run.tasks.iter().find(|t| t.spec.key == *key) {
                body.push_str(&format!(
                    "\n\nAccepted prerequisite {key}:\n{}\nMaster's review: {}",
                    prerequisite.result.as_deref().unwrap_or(""),
                    prerequisite.review.as_deref().unwrap_or("")
                ));
                files.extend(prerequisite.attachments.iter().map(|a| a.path.clone()));
            }
        }
        body.push_str("\n\nUse sbot result only when this execution is complete. Progress replies and questions do not finish this task. Master will review your result. Do not start a nested orchestration.");
        (body, files)
    }

    pub(super) fn dispatch_orchestration(&self, id: &str) -> Result<()> {
        let run = self.store.get_orchestration(id)?;
        for task in run.tasks.iter().filter(|t| orch::ready(&run, t)) {
            match self.dispatch_orchestration_task(&run, task) {
                Ok(Some(routed)) => {
                    self.emit_routed(&routed);
                    tokio::spawn(self.pump_session(&routed.session.id));
                }
                Ok(None) => {}
                Err(e @ (CoreError::Db(_) | CoreError::Json(_))) => return Err(e),
                Err(e) => self.store.fail_orchestration_dispatch(id, task, &e.to_string())?,
            }
        }
        let run = self.store.get_orchestration(id)?;
        self.emit_orchestration(&run);
        self.wake_orchestration(&run)
    }

    fn dispatch_orchestration_task(&self, run: &Orchestration, task: &orch::Task) -> Result<Option<Routed>> {
        let (body, files) = self.task_brief(run, task);
        // Required inputs are checked before the workflow helper's best-effort attachment copying.
        let checked = attachments::check(&files, Path::new("/")).map_err(Refusal::Attachment)?;
        match &task.assignment {
            Assignment::Bot { bot_id } => {
                let bot = self.store.get_bot(bot_id)?.ok_or_else(|| invalid("The assigned bot was deleted."))?;
                let master = self
                    .store
                    .get_session(&run.session_id)?
                    .ok_or_else(|| invalid("Master conversation not found."))?;
                let id = new_id();
                let copies = if checked.is_empty() {
                    vec![]
                } else {
                    attachments::copy_to_inbox(&checked, &self.inbox_bot_dir(&bot.handle)?, &id)
                        .map_err(Refusal::Attachment)?
                };
                let message = NewMessage {
                    sender: master.bot_id,
                    recipient: bot.id.clone(),
                    sender_session: Some(run.session_id.clone()),
                    recipient_session: None,
                    kind: MessageKind::Text,
                    body,
                    attachments: copies,
                    system_code: None,
                    system_params: None,
                    status: MessageStatus::Queued,
                    chain_id: new_id(),
                    hop: 1,
                    in_reply_to: None,
                };
                let result = self.store.dispatch_orchestration_bot(&run.id, task, &bot.id, &id, &message);
                if !matches!(result, Ok(Some(_))) {
                    remove_copies(&message);
                }
                result
            }
            Assignment::Workflow { workflow_id, name, definition } => {
                for step in &definition.steps {
                    if self.store.get_bot(&step.bot_id)?.is_none() {
                        return Err(invalid(format!("The bot for workflow step {} was deleted.", step.name)));
                    }
                }
                let workflow = WorkflowRun {
                    project_id: run.project_id.clone(),
                    id: new_id(),
                    workflow_id: Some(workflow_id.clone()),
                    name: name.clone(),
                    def: definition.clone(),
                    request: body,
                    request_files: files,
                    opener_session: Some(run.session_id.clone()),
                    root_session: None,
                    status: RunStatus::Running,
                    pause: None,
                    extra_visits: BTreeMap::new(),
                    created_at: now(),
                    ended_at: None,
                };
                let first = self.visit_open(
                    &workflow,
                    definition.start.as_deref().ok_or_else(|| invalid("Workflow has no first step."))?,
                    1,
                    None,
                    &[],
                )?;
                let result = self.store.dispatch_orchestration_workflow(&run.id, task, &workflow, &first);
                if !matches!(result, Ok(Some(_))) {
                    remove_copies(&first.message);
                }
                result
            }
        }
    }

    pub fn orchestration_view(&self, id: &str) -> Result<OrchestrationView> {
        let run = self.store.get_orchestration(id)?;
        let executions = run
            .tasks
            .iter()
            .map(|task| {
                let mut ids = match &task.workflow_run_id {
                    Some(id) => self.store.run_session_ids(id)?,
                    None => task.session_id.iter().cloned().collect(),
                };
                // run_session_ids includes the opener for UI notifications, not execution ownership.
                ids.retain(|id| *id != run.session_id);
                for id in ids.clone() {
                    ids.extend(self.store.session_tree(&id)?.into_iter().map(|s| s.id));
                }
                ids.sort();
                ids.dedup();
                Ok(TaskView { task: task.clone(), session_ids: ids })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(OrchestrationView { run, executions })
    }

    pub fn list_orchestrations(&self) -> Result<Vec<OrchestrationView>> {
        self.store.list_orchestrations()?.iter().map(|run| self.orchestration_view(&run.id)).collect()
    }

    pub fn finish_orchestration(
        &self,
        id: &str,
        revision: i64,
        report: &str,
        files: &[String],
        answered: Option<&str>,
    ) -> Result<Message> {
        self.store.require_active()?;
        let run = self.store.get_orchestration(id)?;
        let master = self
            .store
            .get_session(&run.session_id)?
            .and_then(|s| self.store.get_bot(&s.bot_id).ok().flatten())
            .ok_or_else(|| invalid("Master conversation not found."))?;
        let checked = attachments::check(files, &self.bot_directory(&master)?).map_err(Refusal::Attachment)?;
        let message =
            self.store.finish_orchestration(id, revision, report, &attachments::in_place(&checked), answered)?;
        self.emit_message(message.clone());
        self.emit_orchestration(&self.store.get_orchestration(id)?);
        Ok(message)
    }

    pub async fn cancel_orchestration(&self, id: &str, key: Option<&str>) -> Result<()> {
        let run = self.store.cancel_orchestration(id, key)?;
        self.stop_cancelled_orchestration(&run, key).await?;
        self.emit_orchestration(&run);
        self.wake_orchestration(&run)?;
        Ok(())
    }

    pub(super) async fn stop_cancelled_orchestration(&self, run: &Orchestration, key: Option<&str>) -> Result<()> {
        // Read ownership after cancellation commits: concurrent dispatch may have created a
        // session since the caller last read the graph, but cannot start another one now.
        let view = self.orchestration_view(&run.id)?;
        for execution in view
            .executions
            .iter()
            .filter(|t| key.is_none_or(|key| t.task.spec.key == key) && t.task.state == TaskState::Cancelled)
        {
            let sessions = execution
                .session_ids
                .iter()
                .map(|id| self.store.get_session(id))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            let runs = self.stop_runs_of_sessions(&sessions, "cancelled")?;
            self.wind_down(&runs).await;
            let stops = {
                let mut live = self.inner.live.lock().await;
                execution.session_ids.iter().map(|id| self.stop_locked(&mut live, id)).collect::<Result<Vec<_>>>()?
            };
            for stop in stops {
                self.stopped(stop).await;
            }
        }
        Ok(())
    }

    pub async fn archive_orchestration_task(&self, id: &str, key: &str, archived: bool) -> Result<()> {
        let view = self.orchestration_view(id)?;
        let task = view.executions.iter().find(|t| t.task.spec.key == key).ok_or_else(|| invalid("Task not found."))?;
        if !task.task.state.terminal() {
            return Err(invalid("Cancel or finish the task before marking it done."));
        }
        if !archived {
            // A task hidden by its parent's archive must become discoverable again too.
            for session in self.store.restore_session(&view.run.session_id)?.unwrap_or_default() {
                self.emit(UiEvent::SessionUpdated { session: self.session_view(session) });
            }
        }
        for session in &task.session_ids {
            if self.store.get_session(session)?.is_none() {
                continue;
            }
            if archived {
                self.archive_tree(session, None).await?;
            } else {
                for session in self.store.restore_session(session)?.unwrap_or_default() {
                    self.emit(UiEvent::SessionUpdated { session: self.session_view(session) });
                }
            }
        }
        self.store.set_orchestration_task_archived(id, key, archived)?;
        self.emit_orchestration(&self.store.get_orchestration(id)?);
        Ok(())
    }

    pub(super) fn emit_orchestration(&self, run: &Orchestration) {
        self.emit(UiEvent::OrchestrationChanged { project_id: run.project_id.clone(), run_id: run.id.clone() });
    }

    pub(super) fn wake_orchestration(&self, run: &Orchestration) -> Result<()> {
        if run.status == Status::Active {
            for message in self.store.queued_for_session(&run.session_id)?.into_iter().filter(is_event) {
                self.emit_message(message);
            }
            tokio::spawn(self.pump_session(&run.session_id));
        }
        Ok(())
    }

    pub(super) fn orchestration_cancelled_session(&self, session: &str) -> Result<bool> {
        Ok(self.store.orchestration_of_session(session)?.is_some_and(|(run, task)| {
            task.is_some_and(|key| {
                run.status == Status::Cancelled
                    || run.tasks.iter().any(|t| t.spec.key == key && t.state == TaskState::Cancelled)
            })
        }))
    }

    pub(super) fn guard_orchestration_archive(&self, session: &str) -> Result<()> {
        if let Some((run, key)) = self.store.orchestration_of_session(session)? {
            let active = match key {
                Some(key) => run.tasks.iter().any(|t| t.spec.key == key && !t.state.terminal()),
                None => run.status == Status::Active,
            };
            if active {
                return Err(invalid("Cancel or finish the orchestration task before marking it done."));
            }
        }
        Ok(())
    }

    pub(super) async fn cancel_orchestration_for_removal(&self, session: &str) -> Result<()> {
        if let Some((run, key)) = self.store.orchestration_of_session(session)? {
            if run.status == Status::Active {
                self.cancel_orchestration(&run.id, key.as_deref()).await?;
            }
        }
        Ok(())
    }

    pub(super) async fn bridge_orchestration(&self, pane: &str, command: BridgeCommand) -> BridgeResponse {
        let (caller, flight) = match self.master_caller(pane, "orchestration").await {
            Ok(caller) => caller,
            Err(refusal) => return refusal,
        };
        let id = match &command {
            BridgeCommand::OrchestrationShow { id }
            | BridgeCommand::OrchestrationUpdate { id, .. }
            | BridgeCommand::OrchestrationFinish { id, .. }
            | BridgeCommand::OrchestrationCancel { id, .. } => Some(id),
            _ => None,
        };
        if let Some(id) = id {
            match self.store.get_orchestration(id) {
                Ok(run) if run.session_id == caller.session_id => {}
                Ok(_) => {
                    return refuse(
                        ErrorCode::NotAllowed,
                        "Only the owning Master conversation can change or inspect this orchestration.",
                    )
                }
                Err(e) => return refuse(ErrorCode::BadRequest, e.to_string()),
            }
        }
        let answer = flight.as_ref().map(|f| f.message_id.as_str());
        let result = match command {
            BridgeCommand::OrchestrationStart { key, body } => {
                if !on_users_turn(flight.as_ref()) {
                    return refuse(ErrorCode::NotAllowed, "Start orchestration on the user's own message.");
                }
                serde_json::from_str::<Plan>(&body)
                    .map_err(CoreError::from)
                    .and_then(|plan| self.start_orchestration(&caller.session_id, answer.unwrap(), &key, plan))
                    .and_then(|run| Ok(serde_json::to_string_pretty(&run)?))
            }
            BridgeCommand::OrchestrationShow { id } => {
                self.orchestration_view(&id).and_then(|run| Ok(serde_json::to_string_pretty(&run)?))
            }
            BridgeCommand::OrchestrationUpdate { id, revision, key, body } => serde_json::from_str::<Update>(&body)
                .map_err(CoreError::from)
                .and_then(|input| self.update_orchestration(&id, revision, &key, input, answer))
                .and_then(|run| Ok(serde_json::to_string_pretty(&run)?)),
            BridgeCommand::OrchestrationFinish { id, revision, body, files } => self
                .finish_orchestration(&id, revision, &body, &files, answer)
                .map(|_| "Combined report delivered. End your turn.".into()),
            BridgeCommand::OrchestrationCancel { id, task } => {
                if !on_users_turn(flight.as_ref()) {
                    return refuse(
                        ErrorCode::NotAllowed,
                        "Cancellation requires the user's message; report failed results with update.",
                    );
                }
                self.cancel_orchestration(&id, task.as_deref()).await.map(|_| "Cancelled.".into())
            }
            _ => unreachable!(),
        };
        match result {
            Ok(text) => {
                // show is read-only; the other commands durably handle the current turn.
                if let Some(flight) = &flight {
                    if self.store.answered_by(&flight.message_id).ok().flatten().is_some() {
                        self.set_replied(&caller.session_id, &flight.message_id, true).await;
                    }
                }
                BridgeResponse::output(text)
            }
            Err(e) => refuse(ErrorCode::BadRequest, e.to_string()),
        }
    }
}

pub(super) fn is_event(message: &Message) -> bool {
    message.system_code.as_deref() == Some("orchestration_event")
}

fn remove_copies(message: &NewMessage) {
    for file in &message.attachments {
        let _ = std::fs::remove_file(&file.path);
    }
}
