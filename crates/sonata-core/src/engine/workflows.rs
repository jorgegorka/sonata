use bridge_proto::{BridgeResponse, ErrorCode};
use sha2::{Digest, Sha256};

use super::master::on_users_turn;
use super::messaging::{file_error_code, internal, not_a_bot, refuse, Caller};
use super::tasks::bad_request;
use super::*;
use crate::ids::now;
use crate::store::{CreationKind, NewCreation, NewRun, RunMoved, RunThen, VisitClose, VisitOpen};
use crate::workflow::prompts::{finished_body, latest_results, reminder_body, visit_body, Seen};
use crate::workflow::text::{format_text, list_entry, parse_text};
use crate::workflow::{self, check_workflow, decide, Decision};

struct Earlier {
    step: String,
    handle: String,
    visit: u32,
    outcome: Option<String>,
    result: Message,
}

fn remove_copies(message: &NewMessage) {
    for file in &message.attachments {
        let _ = std::fs::remove_file(&file.path);
    }
}

fn copies_of(then: &RunThen) -> Option<&NewMessage> {
    match then {
        RunThen::Visit(open) => Some(&open.message),
        RunThen::Finish { to_opener: Some((_, message, _)), .. } => Some(message),
        _ => None,
    }
}

struct Visited {
    step: String,
    handle: String,
    visit: u32,
    outcome: Option<String>,
    body: String,
    files: Vec<String>,
}

impl Engine {
    pub fn list_workflows(&self) -> Result<Vec<Workflow>> {
        self.store.list_workflows()
    }

    pub fn create_workflow(&self, new: NewWorkflow) -> Result<Workflow> {
        self.store.require_active()?;
        let new = self.checked_workflow(new, None)?;
        let workflow = self.store.insert_workflow(&new)?;
        self.emit(UiEvent::WorkflowsChanged);
        Ok(workflow)
    }

    pub fn update_workflow(&self, id: &str, edit: NewWorkflow) -> Result<Workflow> {
        self.store.require_active()?;
        self.store.get_workflow(id)?.ok_or_else(|| Refusal::WorkflowNotFound(id.to_string()))?;
        let edit = self.checked_workflow(edit, Some(id))?;
        let workflow =
            self.store.update_workflow(id, &edit)?.ok_or_else(|| Refusal::WorkflowNotFound(id.to_string()))?;
        self.emit(UiEvent::WorkflowsChanged);
        Ok(workflow)
    }

    pub fn delete_workflow(&self, id: &str) -> Result<Workflow> {
        self.store.require_active()?;
        let workflow = self.store.delete_workflow(id)?.ok_or_else(|| Refusal::WorkflowNotFound(id.to_string()))?;
        self.emit(UiEvent::WorkflowsChanged);
        Ok(workflow)
    }

    pub(super) fn checked_workflow(&self, mut new: NewWorkflow, editing: Option<&str>) -> Result<NewWorkflow> {
        new.name = new.name.trim().to_string();
        self.check_runnable(&new)?;
        if let Some(other) = self.store.get_workflow_by_name(&new.name)? {
            if editing != Some(other.id.as_str()) {
                return Err(Refusal::WorkflowNameTaken(new.name).into());
            }
        }
        Ok(new)
    }

    fn check_runnable(&self, new: &NewWorkflow) -> Result<()> {
        let mut known = HashSet::new();
        for step in &new.def.steps {
            if !step.bot_id.is_empty()
                && self.store.get_bot(&step.bot_id)?.is_some_and(|b| b.project_id == self.store.project_key())
            {
                known.insert(step.bot_id.clone());
            }
        }
        check_workflow(new, |id| known.contains(id))?;
        Ok(())
    }

    pub fn start_workflow(&self, workflow_id: &str, request: &str, files: &[String]) -> Result<Session> {
        self.store.require_active()?;
        let workflow =
            self.store.get_workflow(workflow_id)?.ok_or_else(|| Refusal::WorkflowNotFound(workflow_id.into()))?;
        let moved = self.start_run_by(&workflow, request, files, None)?;
        let opened = moved.opened.ok_or_else(|| CoreError::Internal("a started run opened no visit".into()))?;
        Ok(opened.session)
    }

    pub(super) fn start_run_by(
        &self,
        workflow: &Workflow,
        request: &str,
        files: &[String],
        opener: Option<(&str, &str)>,
    ) -> Result<RunMoved> {
        let request = request.trim();
        if request.is_empty() && files.is_empty() {
            return Err(Refusal::MessageEmpty.into());
        }
        let snapshot = NewWorkflow {
            name: workflow.name.clone(),
            when_to_use: workflow.when_to_use.clone(),
            def: workflow.def.clone(),
        };
        self.check_runnable(&snapshot)?;
        attachments::check(files, Path::new("/")).map_err(Refusal::Attachment)?;
        let first = snapshot.def.start.clone().ok_or(Refusal::WorkflowNoStart)?;
        let run = WorkflowRun {
            project_id: self.store.project_key().into(),
            id: new_id(),
            workflow_id: Some(workflow.id.clone()),
            name: snapshot.name,
            def: snapshot.def,
            request: request.to_string(),
            request_files: files.to_vec(),
            opener_session: opener.map(|(session, _)| session.to_string()),
            root_session: None,
            status: RunStatus::Running,
            pause: None,
            extra_visits: BTreeMap::new(),
            created_at: now(),
            ended_at: None,
        };
        let open = self.visit_open(&run, &first, 1, None, &[])?;
        let new = NewRun {
            id: run.id.clone(),
            workflow_id: workflow.id.clone(),
            name: run.name.clone(),
            def: run.def.clone(),
            request: run.request.clone(),
            request_files: run.request_files.clone(),
            opener_session: run.opener_session.clone(),
        };
        let moved = match self.store.start_workflow_run(&new, &open) {
            Ok(moved) => moved,
            Err(e) => {
                remove_copies(&open.message);
                return Err(e);
            }
        };
        if let Some(routed) = &moved.opened {
            self.emit_routed(routed);
            let by = opener.map_or_else(|| "the user".to_string(), |(_, handle)| format!("@{handle}"));
            tracing::info!(
                bot = %self.handle_of(&routed.session.bot_id),
                session = routed.session.number,
                workflow = %moved.run.name,
                started_by = %by,
                "started a workflow run"
            );
            tokio::spawn(self.pump_session(&routed.session.id));
        }
        self.emit_run(&moved.run);
        Ok(moved)
    }

    pub(super) fn visit_open(
        &self,
        run: &WorkflowRun,
        step_id: &str,
        n: u32,
        extra: Option<Seen>,
        extra_files: &[String],
    ) -> Result<VisitOpen> {
        let step = workflow::step(&run.def, step_id)
            .ok_or_else(|| CoreError::Internal(format!("run {} has no step {step_id}", run.id)))?;
        let bot = self
            .store
            .get_bot(&step.bot_id)?
            .ok_or_else(|| Refusal::WorkflowStepNoBot { step: step.name.trim().to_string() })?;
        let known = self.store.run_session_of(&run.id, &bot.id)?;
        let first_visit = known.is_none();
        let (own_session, seen_before) = match &known {
            Some((session, seen)) => (Some(session.as_str()), *seen),
            None => (None, 0),
        };

        let visits = self.store.run_visits(&run.id)?;
        let mut earlier: Vec<Earlier> = Vec::new();
        let mut seen_ord = seen_before;
        for visit in visits.iter().filter(|v| v.ord > seen_before && v.ended_at.is_some()) {
            if own_session.is_some() && visit.session_id.as_deref() == own_session {
                continue;
            }
            let Some(result) = visit.result_message.as_deref().map(|id| self.store.get_message(id)) else {
                continue;
            };
            let Some(result) = result? else { continue };
            let (step, handle) = match workflow::step(&run.def, &visit.step_id) {
                Some(s) => (s.name.trim().to_string(), self.handle_of(&s.bot_id)),
                None => (visit.step_id.clone(), self.handle_of(&result.sender)),
            };
            seen_ord = seen_ord.max(visit.ord);
            earlier.push(Earlier { step, handle, visit: visit.n, outcome: visit.outcome.clone(), result });
        }
        let extra = extra.filter(|seen| seen.handle != bot.handle);
        let extra_files = if extra.is_some() { extra_files } else { &[] };
        if extra.is_some() {
            seen_ord = seen_ord.max(visits.last().map_or(0, |v| v.ord));
        }

        let message_id = new_id();
        let mut paths: Vec<&str> = Vec::new();
        if first_visit {
            paths.extend(run.request_files.iter().map(String::as_str));
        }
        for e in &earlier {
            paths.extend(e.result.attachments.iter().map(|a| a.path.as_str()));
        }
        paths.extend(extra_files.iter().map(String::as_str));
        let attachments = self.inbox_copies(&bot.handle, &message_id, &paths)?;

        let results: Vec<Seen> = earlier
            .iter()
            .map(|e| Seen {
                step: &e.step,
                handle: &e.handle,
                visit: e.visit,
                outcome: e.outcome.as_deref(),
                body: &e.result.body,
            })
            .chain(extra)
            .collect();
        let request = first_visit.then_some(run.request.as_str());
        let body = visit_body(request, &results, &step.instructions, &workflow::outcomes(step));
        let title = match run.request.trim() {
            "" => run.name.clone(),
            text => first_line(&format!("{} · {text}", run.name), TITLE_MAX).unwrap_or_else(|| run.name.clone()),
        };
        let visit_id = new_id();
        let session = known.map(|(session, _)| session);
        Ok(VisitOpen {
            visit_id: visit_id.clone(),
            step_id: step.id.clone(),
            n,
            bot_id: bot.id.clone(),
            session: session.clone(),
            title,
            opened_by: run.opener_session.clone().or_else(|| run.root_session.clone()),
            message_id,
            message: NewMessage {
                sender: USER.into(),
                recipient: bot.id.clone(),
                sender_session: None,
                recipient_session: session,
                kind: MessageKind::Text,
                body,
                attachments,
                system_code: None,
                system_params: None,
                status: MessageStatus::Queued,
                chain_id: new_id(),
                hop: 0,
                in_reply_to: None,
            },
            mark: WorkflowMark {
                run_id: run.id.clone(),
                visit_id,
                workflow: run.name.clone(),
                step: step.name.trim().to_string(),
                visit: n,
                role: MarkRole::Request,
                outcome: None,
            },
            seen_ord,
        })
    }

    fn inbox_copies(&self, handle: &str, message_id: &str, paths: &[&str]) -> Result<Vec<Attachment>> {
        let files: Vec<PathBuf> = paths
            .iter()
            .filter_map(|path| match attachments::check(&[path.to_string()], Path::new("/")) {
                Ok(mut checked) => checked.pop(),
                Err(e) => {
                    tracing::warn!(bot = %handle, message_id = %message_id, error = %e, "left a file out of a workflow message");
                    None
                }
            })
            .collect();
        if files.is_empty() {
            return Ok(Vec::new());
        }
        let dir = self.inbox_bot_dir(handle)?;
        Ok(attachments::copy_to_inbox(&files, &dir, message_id).map_err(Refusal::Attachment)?)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn step_result(
        &self,
        from: &Caller,
        flight: Option<InFlight>,
        run: WorkflowRun,
        visit: Visit,
        body: String,
        files: Vec<String>,
        outcome: Option<String>,
    ) -> BridgeResponse {
        let body = body.trim_end().to_string();
        if body.trim().is_empty() {
            return refuse(
                ErrorCode::EmptyBody,
                "The result is empty. Pass the text on stdin: sbot result <<'EOF' ... EOF",
            );
        }
        let Some(step) = workflow::step(&run.def, &visit.step_id) else {
            return internal(format!("run {} has no step {}", run.id, visit.step_id));
        };
        if let Err(text) = workflow::follow(step, outcome.as_deref()) {
            return refuse(ErrorCode::BadRequest, text);
        }
        let sender = match self.store.get_bot(&from.bot_id) {
            Ok(Some(bot)) => bot,
            Ok(None) => return not_a_bot(),
            Err(e) => return internal(e),
        };
        let sender_dir = match self.bot_directory(&sender) {
            Ok(dir) => dir,
            Err(e) => return internal(e),
        };
        let checked = match attachments::check(&files, &sender_dir) {
            Ok(checked) => checked,
            Err(e) => return refuse(file_error_code(&e), format!("{e}.")),
        };
        let stored = attachments::in_place(&checked);

        let mut answers = flight;
        let previous = match &answers {
            Some(f) => match self.set_replied(&from.session_id, &f.message_id, true).await {
                Some(was) => Some(was),
                None => {
                    answers = None;
                    None
                }
            },
            None => None,
        };
        match self.move_on(&run, &visit, &sender, &from.session_id, body, stored, outcome, answers.as_ref()) {
            Ok(moved) => {
                let mut response = BridgeResponse::output(format!("{} End your turn.", moved.said));
                response.message_id = moved.result_id;
                response
            }
            Err(e) => {
                if let (Some(f), Some(was)) = (&answers, previous) {
                    self.set_replied(&from.session_id, &f.message_id, was).await;
                }
                match e {
                    CoreError::Refused(Refusal::WorkflowRunNotFound(_)) => refuse(
                        ErrorCode::NoOpenRequest,
                        "The workflow run no longer waits for this step's result, so it was not recorded. End your \
                         turn.",
                    ),
                    CoreError::Refused(Refusal::SessionNotFound(gone)) if gone == from.session_id => not_a_bot(),
                    e => internal(e),
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn move_on(
        &self,
        run: &WorkflowRun,
        visit: &Visit,
        sender: &Bot,
        session_id: &str,
        body: String,
        files: Vec<Attachment>,
        outcome: Option<String>,
        answers: Option<&InFlight>,
    ) -> Result<Moved> {
        let step = workflow::step(&run.def, &visit.step_id)
            .ok_or_else(|| CoreError::Internal(format!("run {} has no step {}", run.id, visit.step_id)))?;
        let arrow = workflow::follow(step, outcome.as_deref())
            .map_err(|_| Refusal::WorkflowOutcomeUnknown(outcome.clone().unwrap_or_default()))?;
        let outcome =
            arrow.filter(|_| step.arrows.len() > 1).and_then(|a| a.outcome.as_deref()).map(|o| o.trim().to_string());
        let step_name = step.name.trim().to_string();
        let who = self.who(session_id);
        let seen =
            Seen { step: &step_name, handle: &sender.handle, visit: visit.n, outcome: outcome.as_deref(), body: &body };
        let paths: Vec<String> = files.iter().map(|a| a.path.clone()).collect();
        let then = self.run_then(run, visit, arrow.map(|a| a.to.as_str()), seen, &paths).inspect_err(|e| {
            tracing::error!(bot = %who.bot, session = who.session, workflow = %run.name, step = %step_name, error = %e, "could not prepare a workflow run's next move");
        })?;

        let (in_reply_to, answered) = match answers {
            Some(f) if f.sender == USER => (Some(f.message_id.clone()), None),
            Some(f) => (None, Some(f.message_id.clone())),
            None => (None, None),
        };
        let (chain_id, hop) = match answers {
            Some(f) => (f.chain_id.clone(), f.hop + 1),
            None => (new_id(), 0),
        };
        let close = VisitClose {
            visit_id: visit.id.clone(),
            result_id: new_id(),
            result: NewMessage {
                sender: sender.id.clone(),
                recipient: USER.into(),
                sender_session: Some(session_id.to_string()),
                recipient_session: None,
                kind: MessageKind::Text,
                body,
                attachments: files,
                system_code: None,
                system_params: None,
                status: MessageStatus::Delivered,
                chain_id,
                hop,
                in_reply_to,
            },
            mark: WorkflowMark {
                run_id: run.id.clone(),
                visit_id: visit.id.clone(),
                workflow: run.name.clone(),
                step: step_name.clone(),
                visit: visit.n,
                role: MarkRole::Result,
                outcome: outcome.clone(),
            },
            outcome: outcome.clone(),
            answered,
        };
        let moved = match self.store.advance_workflow_run(&close, &then) {
            Ok(moved) => moved,
            Err(e) => {
                if let Some(message) = copies_of(&then) {
                    remove_copies(message);
                }
                let expected = match &e {
                    CoreError::Refused(Refusal::WorkflowRunNotFound(_)) => true,
                    CoreError::Refused(Refusal::SessionNotFound(gone)) => gone == session_id,
                    _ => false,
                };
                if !expected {
                    tracing::error!(bot = %who.bot, session = who.session, workflow = %run.name, step = %step_name, error = %e, "could not record a workflow step's result");
                }
                return Err(e);
            }
        };

        if let Some(result) = moved.result.clone() {
            self.emit_message(result);
        }
        let picked = outcome.as_deref().map(|o| format!(" (outcome \"{o}\")")).unwrap_or_default();
        let recorded = format!("Result recorded for step \"{step_name}\"{picked}.");
        let next = match (&moved.opened, &moved.run.status) {
            (Some(routed), _) => {
                self.emit_routed(routed);
                let to = routed.message.workflow_mark.as_ref().map_or_else(String::new, |m| m.step.clone());
                let handle = self.handle_of(&routed.session.bot_id);
                tracing::info!(bot = %who.bot, session = who.session, workflow = %moved.run.name, step = %step_name, outcome = ?outcome, next = %to, next_bot = %handle, "a workflow step handed in its result; the run moves on");
                tokio::spawn(self.pump_session(&routed.session.id));
                format!("The workflow moves on to step \"{to}\" (@{handle}).")
            }
            (None, RunStatus::Finished) => {
                tracing::info!(bot = %who.bot, session = who.session, workflow = %moved.run.name, step = %step_name, outcome = ?outcome, "a workflow step handed in its result; the run is finished");
                "The workflow is finished.".to_string()
            }
            (None, _) => {
                tracing::info!(bot = %who.bot, session = who.session, workflow = %moved.run.name, step = %step_name, outcome = ?outcome, pause = ?moved.run.pause, "a workflow step handed in its result; the run is paused");
                "The workflow is paused for the user.".to_string()
            }
        };
        if let Some(finished) = moved.finished {
            let deliver = finished.recipient_session.clone().filter(|_| finished.status == MessageStatus::Queued);
            self.emit_message(finished);
            if let Some(opener) = deliver {
                tokio::spawn(self.pump_session(&opener));
            }
        }
        self.emit_run(&moved.run);
        Ok(Moved { result_id: moved.result.map(|m| m.id), said: format!("{recorded} {next}") })
    }

    fn run_then(
        &self,
        run: &WorkflowRun,
        visit: &Visit,
        to: Option<&str>,
        seen: Seen,
        files: &[String],
    ) -> Result<RunThen> {
        let visits = self.store.run_visits(&run.id)?;
        let known = self.known_bots(run)?;
        Ok(match decide(run, &visits, to, |id| known.contains(id)) {
            Decision::Visit { step_id, n } => RunThen::Visit(self.visit_open(run, &step_id, n, Some(seen), files)?),
            Decision::Pause(reason) => RunThen::Pause(reason),
            Decision::Finish => match &run.opener_session {
                Some(opener) => RunThen::Finish {
                    to_opener: self.finished_message(run, opener, &visits, visit, seen, files)?,
                    notice: None,
                },
                None => {
                    let root = match &run.root_session {
                        Some(root) => self.store.get_session(root)?,
                        None => None,
                    };
                    let notice = match root {
                        Some(root) if root.archived_at.is_none() => Some(NewMessage::system(
                            &root,
                            "workflow_finished",
                            json!({"name": run.name, "step": seen.step, "outcome": seen.outcome.unwrap_or("")}),
                        )),
                        _ => None,
                    };
                    RunThen::Finish { to_opener: None, notice }
                }
            },
        })
    }

    fn finished_message(
        &self,
        run: &WorkflowRun,
        opener_id: &str,
        visits: &[Visit],
        last: &Visit,
        seen: Seen,
        files: &[String],
    ) -> Result<Option<(String, NewMessage, WorkflowMark)>> {
        let store = &self.store;
        let opener = match store.get_session(opener_id)? {
            Some(session) if session.archived_at.is_none() => store.get_bot(&session.bot_id)?.map(|bot| (session, bot)),
            _ => None,
        };
        let Some((opener, bot)) = opener else {
            tracing::info!(workflow = %run.name, step = %seen.step, "the session that started the workflow run is archived or gone; the run finishes without telling it");
            return Ok(None);
        };
        let mut path = Vec::new();
        for v in visits {
            if v.id == last.id {
                path.push(Visited {
                    step: seen.step.to_string(),
                    handle: seen.handle.to_string(),
                    visit: seen.visit,
                    outcome: seen.outcome.map(str::to_string),
                    body: seen.body.to_string(),
                    files: files.to_vec(),
                });
                continue;
            }
            let result = match v.result_message.as_deref() {
                Some(id) => store.get_message(id)?,
                None => None,
            };
            let (step, handle) = match (workflow::step(&run.def, &v.step_id), &result) {
                (Some(s), _) => (s.name.trim().to_string(), self.handle_of(&s.bot_id)),
                (None, Some(m)) => (v.step_id.clone(), self.handle_of(&m.sender)),
                (None, None) => (v.step_id.clone(), String::new()),
            };
            path.push(Visited {
                step,
                handle,
                visit: v.n,
                outcome: v.outcome.clone(),
                body: result.as_ref().map(|m| m.body.clone()).unwrap_or_default(),
                files: result.map(|m| m.attachments.into_iter().map(|a| a.path).collect()).unwrap_or_default(),
            });
        }
        let seen_path: Vec<Seen> = path
            .iter()
            .map(|v| Seen {
                step: &v.step,
                handle: &v.handle,
                visit: v.visit,
                outcome: v.outcome.as_deref(),
                body: &v.body,
            })
            .collect();
        let latest = latest_results(&seen_path);
        let paths: Vec<&str> = latest
            .iter()
            .filter_map(|l| path.iter().find(|v| v.step == l.step && v.visit == l.visit))
            .flat_map(|v| v.files.iter().map(String::as_str))
            .collect();
        let id = new_id();
        let attachments = self.inbox_copies(&bot.handle, &id, &paths)?;
        let message = NewMessage {
            sender: USER.into(),
            recipient: bot.id.clone(),
            sender_session: None,
            recipient_session: Some(opener.id.clone()),
            kind: MessageKind::Text,
            body: finished_body(&seen_path, &latest),
            attachments,
            system_code: None,
            system_params: None,
            status: MessageStatus::Queued,
            chain_id: new_id(),
            hop: 0,
            in_reply_to: None,
        };
        let mark = WorkflowMark {
            run_id: run.id.clone(),
            visit_id: last.id.clone(),
            workflow: run.name.clone(),
            step: seen.step.to_string(),
            visit: last.n,
            role: MarkRole::Finished,
            outcome: seen.outcome.map(str::to_string),
        };
        Ok(Some((id, message, mark)))
    }

    pub(super) fn run_starter(&self, run_id: &str) -> Result<Option<String>> {
        let Some(opener) = self.store.get_run(run_id)?.and_then(|run| run.opener_session) else {
            return Ok(None);
        };
        let Some(session) = self.store.get_session(&opener)? else {
            return Ok(None);
        };
        Ok(self.store.get_bot(&session.bot_id)?.map(|bot| bot.handle))
    }

    fn known_bots(&self, run: &WorkflowRun) -> Result<HashSet<String>> {
        let mut known = HashSet::new();
        for step in &run.def.steps {
            if self.store.get_bot(&step.bot_id)?.is_some_and(|b| b.project_id == self.store.project_key()) {
                known.insert(step.bot_id.clone());
            }
        }
        Ok(known)
    }

    pub(super) async fn after_step_turn(&self, session_id: &str, flight: &InFlight, background_work: bool) {
        if flight.stopped || background_work {
            return;
        }
        if let Err(e) = self.remind_or_pause(session_id) {
            let who = self.who(session_id);
            tracing::error!(bot = %who.bot, session = who.session, message_id = %flight.message_id, error = %e, "could not check the workflow step whose turn ended");
        }
    }

    fn remind_or_pause(&self, session_id: &str) -> Result<()> {
        let store = &self.store;
        if store.has_pending_task_result(session_id)? || store.own_orchestration(session_id)?.is_some() {
            return Ok(());
        }
        let Some((run, visit)) = store.open_visit_of_session(session_id)? else { return Ok(()) };
        if run.status != RunStatus::Running {
            return Ok(());
        }
        let Some(session) = store.get_session(session_id)? else { return Ok(()) };
        if session.archived_at.is_some() || session.waiting_question_id.is_some() {
            return Ok(());
        }
        if !store.queued_for_session(session_id)?.is_empty() || store.owed_answers(session_id, &visit.started_at)? > 0 {
            return Ok(());
        }
        let who = self.who(session_id);
        if visit.reminded {
            let reason = PauseReason::NoResult { step_id: visit.step_id.clone() };
            if let Some(run) = store.pause_workflow_run(&run.id, &reason)? {
                self.emit_run(&run);
                tracing::info!(bot = %who.bot, session = who.session, workflow = %run.name, step = %step_name(&run, &visit.step_id), "a workflow step ended its turn again without its result; the run is paused");
            }
            return Ok(());
        }
        let (id, message, mark) = reminder(&run, &visit, &session)?;
        let reminder = store.insert_marked(&id, &message, &mark)?;
        store.mark_visit_reminded(&visit.id)?;
        self.emit_message(reminder);
        tracing::info!(bot = %who.bot, session = who.session, workflow = %run.name, step = %mark.step, visit = visit.n, "a workflow step ended its turn without its result; it is reminded");
        Ok(())
    }

    fn run_by_id(&self, run_id: &str) -> Result<WorkflowRun> {
        Ok(self.store.get_run(run_id)?.ok_or_else(|| Refusal::WorkflowRunNotFound(run_id.to_string()))?)
    }

    fn paused_for_result(&self, run_id: &str) -> Result<(WorkflowRun, Visit, Session)> {
        let run = self.run_by_id(run_id)?;
        let step_id = match (run.status, &run.pause) {
            (RunStatus::Paused, Some(PauseReason::NoResult { step_id })) => step_id.clone(),
            _ => return Err(Refusal::WorkflowRunNotPaused.into()),
        };
        let visit = self
            .store
            .run_visits(run_id)?
            .into_iter()
            .rev()
            .find(|v| v.step_id == step_id && v.ended_at.is_none())
            .ok_or_else(|| CoreError::Internal(format!("run {run_id} paused at step {step_id} with no visit open")))?;
        let session = match &visit.session_id {
            Some(id) => self.store.get_session(id)?,
            None => None,
        };
        let session = session.ok_or_else(|| Refusal::SessionNotFound(visit.session_id.clone().unwrap_or_default()))?;
        Ok((run, visit, session))
    }

    fn post_to_step(&self, routed: &Routed) {
        self.emit_routed(routed);
        tokio::spawn(self.pump_session(&routed.session.id));
    }

    pub fn another_round(&self, run_id: &str) -> Result<()> {
        let mut run = self.run_by_id(run_id)?;
        let step_id = match (run.status, &run.pause) {
            (RunStatus::Paused, Some(PauseReason::VisitCap { step_id })) => step_id.clone(),
            _ => return Err(Refusal::WorkflowRunNotPaused.into()),
        };
        *run.extra_visits.entry(step_id.clone()).or_default() += 1;
        let visits = self.store.run_visits(run_id)?;
        let known = self.known_bots(&run)?;
        let then = match decide(&run, &visits, Some(&step_id), |id| known.contains(id)) {
            Decision::Visit { step_id, n } => RunThen::Visit(self.visit_open(&run, &step_id, n, None, &[])?),
            Decision::Pause(reason) => RunThen::Pause(reason),
            Decision::Finish => {
                return Err(CoreError::Internal(format!("run {run_id} would finish at step {step_id}")))
            }
        };
        let moved = self.store.resume_workflow_run(run_id, Some(&step_id), &then).inspect_err(|_| {
            if let Some(message) = copies_of(&then) {
                remove_copies(message);
            }
        })?;
        if let Some(routed) = &moved.opened {
            self.post_to_step(routed);
        }
        self.emit_run(&moved.run);
        tracing::info!(workflow = %moved.run.name, step = %step_name(&run, &step_id), status = moved.run.status.as_str(), pause = ?moved.run.pause, "the user allowed a workflow step another round");
        Ok(())
    }

    pub fn pick_outcome(&self, run_id: &str, outcome: Option<String>) -> Result<()> {
        let (run, visit, session) = self.paused_for_result(run_id)?;
        let sender =
            self.store.get_bot(&session.bot_id)?.ok_or_else(|| Refusal::BotNotFound(session.bot_id.clone()))?;
        let body = self
            .store
            .last_reply_since(&session.id, &visit.started_at)?
            .map(|reply| reply.body.trim_end().to_string())
            .filter(|body| !body.trim().is_empty())
            .unwrap_or_else(|| NO_REPLY.to_string());
        tracing::info!(bot = %sender.handle, session = session.number, workflow = %run.name, step = %step_name(&run, &visit.step_id), outcome = ?outcome, "the user picks a workflow step's outcome");
        self.move_on(&run, &visit, &sender, &session.id, body, Vec::new(), outcome, None).map_err(|e| match e {
            CoreError::Refused(Refusal::WorkflowRunNotFound(_)) => Refusal::WorkflowRunNotPaused.into(),
            e => e,
        })?;
        Ok(())
    }

    pub fn remind_step(&self, run_id: &str) -> Result<()> {
        let (run, visit, session) = self.paused_for_result(run_id)?;
        let (id, message, mark) = reminder(&run, &visit, &session)?;
        let moved = self.store.resume_workflow_run(run_id, None, &RunThen::Remind { id, message, mark })?;
        if let Some(routed) = &moved.opened {
            self.post_to_step(routed);
        }
        self.emit_run(&moved.run);
        tracing::info!(bot = %self.handle_of(&session.bot_id), session = session.number, workflow = %run.name, step = %step_name(&run, &visit.step_id), "the user reminded a workflow step of its result");
        Ok(())
    }

    pub fn get_workflow_run(&self, run_id: &str) -> Result<RunView> {
        let run = self.run_by_id(run_id)?;
        let visits = self.store.run_visits(run_id)?;
        Ok(RunView { run, visits })
    }

    fn emit_run(&self, run: &WorkflowRun) {
        if let Some(opener) = &run.opener_session {
            if let Ok(Some((process, _))) = self.store.orchestration_of_session(opener) {
                self.emit_orchestration(&process);
                if let Err(error) = self.wake_orchestration(&process) {
                    tracing::warn!(%error, "could not wake the orchestration coordinator");
                }
            }
        }
        let read = self.store.run_visits(&run.id).and_then(|visits| {
            let ids = self.store.run_session_ids(&run.id)?;
            Ok((visits, ids))
        });
        match read {
            Ok((visits, session_ids)) => {
                let step = run_step(run, &visits).map(|s| s.name.trim().to_string());
                self.emit(UiEvent::WorkflowRunChanged {
                    run_id: run.id.clone(),
                    name: run.name.clone(),
                    status: run.status,
                    pause: run.pause.clone(),
                    step,
                    session_ids,
                });
            }
            Err(e) => {
                tracing::error!(workflow = %run.name, error = %e, "could not tell the UI that a workflow run moved")
            }
        }
    }

    pub(super) fn run_glance_of(&self, session_id: &str) -> Result<Option<RunGlance>> {
        let Some(run) = self.store.latest_run_of_session(session_id)? else { return Ok(None) };
        let visits = self.store.run_visits(&run.id)?;
        let step = run_step(&run, &visits);
        let open = visits.iter().rev().find(|v| v.ended_at.is_none());
        let last_with_session = || visits.iter().rev().find_map(|v| v.session_id.clone());
        let holder = match &run.pause {
            Some(PauseReason::VisitCap { .. }) => visits.last().and_then(|v| v.session_id.clone()),
            Some(PauseReason::NoResult { .. } | PauseReason::BotMissing { .. }) => {
                open.and_then(|v| v.session_id.clone()).or_else(last_with_session)
            }
            None => None,
        };
        let handle = match step {
            Some(step) => self.store.get_bot(&step.bot_id)?.map(|b| b.handle),
            None => None,
        };
        Ok(Some(RunGlance {
            run_id: run.id.clone(),
            name: run.name.clone(),
            status: run.status,
            step: step.map(|s| s.name.trim().to_string()),
            handle,
            visit: open.or(visits.last()).map_or(0, |v| v.n),
            outcomes: match (&run.pause, step) {
                (Some(PauseReason::NoResult { .. }), Some(step)) => workflow::outcomes(step),
                _ => Vec::new(),
            },
            pause: run.pause.clone(),
            holder: if run.status == RunStatus::Paused { holder } else { None },
        }))
    }

    pub async fn stop_run(&self, run_id: &str) -> Result<()> {
        self.run_by_id(run_id)?;
        let Some(run) = self.store.stop_workflow_run(run_id)? else { return Ok(()) };
        self.emit_run(&run);
        let visits = self.store.run_visits(run_id)?;
        let cancelled = self.cancel_run_queue(&run, &visits).await?;
        tracing::info!(workflow = %run.name, cancelled, "the user stopped a workflow run");
        self.stop_open_step(&visits).await
    }

    async fn cancel_run_queue(&self, run: &WorkflowRun, visits: &[Visit]) -> Result<usize> {
        let store = &self.store;
        let sessions: HashSet<&str> = visits.iter().filter_map(|v| v.session_id.as_deref()).collect();
        let cancelled = {
            let live = self.inner.live.lock().await;
            let claimed: HashSet<&str> =
                live.values().filter_map(|l| l.in_flight.as_ref()).map(|f| f.message_id.as_str()).collect();
            let mut cancelled = Vec::new();
            for session in sessions {
                for queued in store.queued_for_session(session)? {
                    let sent_by_run = queued.workflow_mark.as_ref().is_some_and(|mark| {
                        mark.run_id == run.id && matches!(mark.role, MarkRole::Request | MarkRole::Reminder)
                    });
                    if sent_by_run && !claimed.contains(queued.id.as_str()) {
                        cancelled.extend(store.cancel_queued(&queued.id)?);
                    }
                }
            }
            cancelled
        };
        let count = cancelled.len();
        for message in cancelled {
            self.emit(UiEvent::MessageUpdated { message });
        }
        Ok(count)
    }

    async fn stop_open_step(&self, visits: &[Visit]) -> Result<()> {
        let Some(session) = stopped_visit(visits).and_then(|v| v.session_id.as_deref()) else { return Ok(()) };
        match self.stop_turn(session).await {
            Ok(()) | Err(CoreError::Refused(Refusal::NotWorking)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub(super) fn stop_runs_of_sessions(&self, sessions: &[Session], how: &str) -> Result<Vec<WorkflowRun>> {
        let ids: Vec<String> = sessions.iter().map(|s| s.id.clone()).collect();
        let runs = self.store.stop_runs_of_sessions(&ids)?;
        for run in &runs {
            self.emit_run(run);
            tracing::info!(workflow = %run.name, "a session of the workflow run was {how}; the run is stopped");
        }
        Ok(runs)
    }

    pub(super) async fn wind_down(&self, runs: &[WorkflowRun]) {
        for run in runs {
            let visits = match self.store.run_visits(&run.id) {
                Ok(visits) => visits,
                Err(e) => {
                    tracing::error!(workflow = %run.name, error = %e, "could not read the stopped workflow run's visits");
                    continue;
                }
            };
            let done = async {
                self.cancel_run_queue(run, &visits).await?;
                self.stop_open_step(&visits).await
            };
            if let Err(e) = done.await {
                let step = stopped_visit(&visits).and_then(|v| workflow::step(&run.def, &v.step_id));
                let bot = step.map_or_else(String::new, |s| self.handle_of(&s.bot_id));
                tracing::error!(bot = %bot, workflow = %run.name, error = %e, "could not stop the step of the stopped workflow run");
            }
        }
    }

    pub(super) fn pause_runs_missing_a_session(&self, deleted: Option<&str>) {
        match self.store.pause_runs_missing_a_session() {
            Ok(runs) => {
                for run in runs {
                    self.emit_run(&run);
                    let step = match &run.pause {
                        Some(PauseReason::BotMissing { step_id }) => step_name(&run, step_id),
                        _ => String::new(),
                    };
                    match deleted {
                        Some(handle) => {
                            tracing::info!(bot = %handle, workflow = %run.name, step = %step, "the bot of the workflow run's open step was deleted; the run is paused")
                        }
                        None => {
                            tracing::info!(workflow = %run.name, step = %step, "the bot of the workflow run's open step was deleted before the app quit; the run is paused")
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!(bot = %deleted.unwrap_or_default(), error = %e, "could not pause the workflow runs whose open step was a deleted bot's")
            }
        }
    }
}

fn run_step<'a>(run: &'a WorkflowRun, visits: &[Visit]) -> Option<&'a Step> {
    let id = match (run.status, &run.pause) {
        (
            RunStatus::Paused,
            Some(
                PauseReason::VisitCap { step_id }
                | PauseReason::NoResult { step_id }
                | PauseReason::BotMissing { step_id },
            ),
        ) => step_id.as_str(),
        (RunStatus::Running, _) => visits.iter().rev().find(|v| v.ended_at.is_none())?.step_id.as_str(),
        _ => return None,
    };
    workflow::step(&run.def, id)
}

fn step_name(run: &WorkflowRun, step_id: &str) -> String {
    workflow::step(&run.def, step_id).map_or_else(|| step_id.to_string(), |s| s.name.trim().to_string())
}

fn stopped_visit(visits: &[Visit]) -> Option<&Visit> {
    visits.iter().find(|v| v.ended_at.is_some() && v.result_message.is_none())
}

const CHANGE_ONLY_ON_USERS_TURN: &str = "Create, change and delete workflows only while answering the user's message.";

const IN_A_STEP: &str = "This conversation is a step of a running workflow. Finish the step with `sbot result` \
                         instead of starting another workflow.";

fn definition_fingerprint(definition: &str) -> String {
    format!("{:x}", Sha256::digest(definition.as_bytes()))
}

impl Engine {
    #[allow(clippy::result_large_err)]
    fn named_workflow(&self, name: &str) -> std::result::Result<Workflow, BridgeResponse> {
        match self.store.get_workflow_by_name(name.trim()) {
            Ok(Some(workflow)) => Ok(workflow),
            Ok(None) => {
                Err(refuse(ErrorCode::BadRequest, format!("No workflow named {}; run `sbot workflows`.", name.trim())))
            }
            Err(e) => Err(internal(e)),
        }
    }

    #[allow(clippy::result_large_err)]
    fn read_definition(
        &self,
        text: &str,
        keep: Option<&WorkflowDef>,
    ) -> std::result::Result<NewWorkflow, BridgeResponse> {
        let bots = self.store.list_bots().map_err(internal)?;
        let bot_id_of = |handle: &str| {
            let handle = handle.to_lowercase();
            bots.iter().find(|b| b.handle == handle).map(|b| b.id.clone())
        };
        parse_text(text, bot_id_of, keep).map_err(|why| refuse(ErrorCode::BadRequest, why))
    }

    fn workflow_text(&self, workflow: &Workflow) -> String {
        format_text(&workflow.name, &workflow.when_to_use, &workflow.def, |id| self.handle_of(id))
    }

    pub(super) async fn bridge_workflows(&self, pane_id: &str) -> BridgeResponse {
        if let Err(refusal) = self.master_caller(pane_id, "workflows").await {
            return refusal;
        }
        match self.list_workflows() {
            Ok(workflows) if workflows.is_empty() => BridgeResponse::output("No workflows yet."),
            Ok(workflows) => BridgeResponse::output(
                workflows
                    .iter()
                    .map(|w| list_entry(&w.name, &w.when_to_use, &w.def, |id| self.handle_of(id)))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Err(e) => internal(e),
        }
    }

    pub(super) async fn bridge_workflow_show(&self, pane_id: &str, name: &str) -> BridgeResponse {
        if let Err(refusal) = self.master_caller(pane_id, "workflow show").await {
            return refusal;
        }
        match self.named_workflow(name) {
            Ok(workflow) => BridgeResponse::output(self.workflow_text(&workflow)),
            Err(refusal) => refusal,
        }
    }

    pub(super) async fn bridge_workflow_run(
        &self,
        pane_id: &str,
        name: &str,
        body: String,
        files: Vec<String>,
    ) -> BridgeResponse {
        let (caller, _) = match self.master_caller(pane_id, "workflow run").await {
            Ok(found) => found,
            Err(refusal) => return refusal,
        };
        match self.store.open_visit_of_session(&caller.session_id) {
            Ok(None) => {}
            Ok(Some(_)) => return refuse(ErrorCode::NotAllowed, IN_A_STEP),
            Err(e) => return internal(e),
        }
        let workflow = match self.named_workflow(name) {
            Ok(workflow) => workflow,
            Err(refusal) => return refusal,
        };
        if body.trim().is_empty() {
            return refuse(
                ErrorCode::EmptyBody,
                "The request is empty. Pass it on stdin: sbot workflow run <name> <<'EOF' ... EOF",
            );
        }
        let (master, _) = match self.caller_parts(&caller) {
            Ok(parts) => parts,
            Err(refusal) => return refusal,
        };
        let master_dir = match self.bot_directory(&master) {
            Ok(dir) => dir,
            Err(e) => return internal(e),
        };
        let files: Vec<String> = match attachments::check(&files, &master_dir) {
            Ok(checked) => checked.iter().map(|f| f.to_string_lossy().into_owned()).collect(),
            Err(e) => return refuse(file_error_code(&e), format!("{e}.")),
        };
        match self.start_run_by(&workflow, &body, &files, Some((&caller.session_id, &master.handle))) {
            Ok(moved) => {
                let Some(opened) = &moved.opened else { return internal("the run opened no step") };
                let step = opened.message.workflow_mark.as_ref().map_or_else(String::new, |m| m.step.clone());
                BridgeResponse::output(format!(
                    "Started the workflow \"{}\". Step \"{step}\" went to @{}. The final result arrives here as a \
                     message; end your turn.",
                    moved.run.name,
                    self.handle_of(&opened.session.bot_id)
                ))
            }
            Err(CoreError::Refused(Refusal::Attachment(e))) => refuse(file_error_code(&e), format!("{e}.")),
            Err(e) => bad_request(e),
        }
    }

    pub(super) async fn bridge_workflow_create(
        &self,
        pane_id: &str,
        definition: &str,
        recreate: bool,
    ) -> BridgeResponse {
        let (caller, flight) = match self.master_caller(pane_id, "workflow create").await {
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
        let new = match self.read_definition(definition, None) {
            Ok(new) => new,
            Err(refusal) => return refusal,
        };
        let store = &self.store;
        let key = new.name.trim().to_lowercase();
        let fingerprint = definition_fingerprint(definition);
        let recorded = match store.creation(&session.id, CreationKind::Workflow, &key) {
            Ok(recorded) => recorded,
            Err(e) => return internal(e),
        };
        if let Some(recorded) = recorded {
            match store.get_workflow(&recorded.result_id) {
                Ok(Some(made)) if recorded.fingerprint == fingerprint => {
                    return BridgeResponse::output(format!("Already created.\n{}", self.workflow_text(&made)))
                }
                Ok(Some(made)) => {
                    return refuse(
                        ErrorCode::CreatedDifferently,
                        format!(
                            "This conversation already created the workflow {} with other settings. Change it with \
                             `sbot workflow edit`.",
                            made.name
                        ),
                    )
                }
                Ok(None) if !recreate => {
                    return refuse(
                        ErrorCode::CreatedThenDeleted,
                        format!(
                            "The workflow {} this conversation created was deleted in the app. Ask the user; only if \
                             they want it again, repeat the command with --recreate.",
                            recorded.result_name
                        ),
                    )
                }
                Ok(None) => {}
                Err(e) => return internal(e),
            }
        }
        let new = match self.checked_workflow(new, None) {
            Ok(new) => new,
            Err(e) => return bad_request(e),
        };
        let record =
            NewCreation { session_id: &session.id, kind: CreationKind::Workflow, key: &key, fingerprint: &fingerprint };
        match store.insert_workflow_recorded(&new, &record) {
            Ok(workflow) => {
                tracing::info!(bot = %master.handle, session = session.number, workflow = %workflow.name, "Master created a workflow");
                self.notice(&session.id, "master_created_workflow", json!({"name": workflow.name}));
                self.emit(UiEvent::WorkflowsChanged);
                BridgeResponse::output(self.workflow_text(&workflow))
            }
            Err(e) => refuse(ErrorCode::Internal, format!("Could not create the workflow: {e}")),
        }
    }

    pub(super) async fn bridge_workflow_edit(&self, pane_id: &str, name: &str, definition: &str) -> BridgeResponse {
        let (caller, flight) = match self.master_caller(pane_id, "workflow edit").await {
            Ok(found) => found,
            Err(refusal) => return refusal,
        };
        if !on_users_turn(flight.as_ref()) {
            return refuse(ErrorCode::NotAllowed, CHANGE_ONLY_ON_USERS_TURN);
        }
        let current = match self.named_workflow(name) {
            Ok(current) => current,
            Err(refusal) => return refusal,
        };
        let edit = match self.read_definition(definition, Some(&current.def)) {
            Ok(edit) => edit,
            Err(refusal) => return refusal,
        };
        let changed = match self.update_workflow(&current.id, edit) {
            Ok(changed) => changed,
            Err(e) => return bad_request(e),
        };
        let who = self.who(&caller.session_id);
        tracing::info!(bot = %who.bot, session = who.session, workflow = %changed.name, "Master changed a workflow");
        self.notice(&caller.session_id, "master_changed_workflow", json!({"name": changed.name}));
        BridgeResponse::output(self.workflow_text(&changed))
    }

    pub(super) async fn bridge_workflow_delete(&self, pane_id: &str, name: &str) -> BridgeResponse {
        let (caller, flight) = match self.master_caller(pane_id, "workflow delete").await {
            Ok(found) => found,
            Err(refusal) => return refusal,
        };
        if !on_users_turn(flight.as_ref()) {
            return refuse(ErrorCode::NotAllowed, CHANGE_ONLY_ON_USERS_TURN);
        }
        let current = match self.named_workflow(name) {
            Ok(current) => current,
            Err(refusal) => return refusal,
        };
        match self.delete_workflow(&current.id) {
            Ok(gone) => {
                let who = self.who(&caller.session_id);
                tracing::info!(bot = %who.bot, session = who.session, workflow = %gone.name, "Master deleted a workflow");
                self.notice(&caller.session_id, "master_deleted_workflow", json!({"name": gone.name}));
                BridgeResponse::output(format!("Deleted the workflow {}. Its runs' sessions stay.", gone.name))
            }
            Err(e) => bad_request(e),
        }
    }
}

const NO_REPLY: &str = "(no reply)";

pub(super) struct Moved {
    result_id: Option<String>,
    said: String,
}

fn reminder(run: &WorkflowRun, visit: &Visit, session: &Session) -> Result<(String, NewMessage, WorkflowMark)> {
    let step = workflow::step(&run.def, &visit.step_id)
        .ok_or_else(|| CoreError::Internal(format!("run {} has no step {}", run.id, visit.step_id)))?;
    let name = step.name.trim().to_string();
    let message = NewMessage {
        sender: USER.into(),
        recipient: session.bot_id.clone(),
        sender_session: None,
        recipient_session: Some(session.id.clone()),
        kind: MessageKind::Text,
        body: reminder_body(&name, &workflow::outcomes(step)),
        attachments: Vec::new(),
        system_code: None,
        system_params: None,
        status: MessageStatus::Queued,
        chain_id: new_id(),
        hop: 0,
        in_reply_to: None,
    };
    let mark = WorkflowMark {
        run_id: run.id.clone(),
        visit_id: visit.id.clone(),
        workflow: run.name.clone(),
        step: name,
        visit: visit.n,
        role: MarkRole::Reminder,
        outcome: None,
    };
    Ok((new_id(), message, mark))
}
