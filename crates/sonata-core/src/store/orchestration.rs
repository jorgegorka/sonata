use super::*;
use crate::orchestration::{self as orch, invalid, Orchestration, Status, TaskState};
use serde_json::json;

pub(super) const SCHEMA: &str = r#"
CREATE TABLE orchestrations (
 id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id),
 session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 request_message_id TEXT NOT NULL UNIQUE, status TEXT NOT NULL, data TEXT NOT NULL
);
CREATE UNIQUE INDEX orchestration_active_session ON orchestrations(session_id) WHERE status = 'active';
CREATE INDEX orchestration_project ON orchestrations(project_id, status);
CREATE INDEX orchestration_session ON orchestrations(session_id);
CREATE TABLE orchestration_operations (
 session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 operation_key TEXT NOT NULL, fingerprint TEXT NOT NULL,
 run_id TEXT NOT NULL REFERENCES orchestrations(id) ON DELETE CASCADE,
 PRIMARY KEY(session_id, operation_key)
);
CREATE TABLE orchestration_executions (
 run_id TEXT NOT NULL REFERENCES orchestrations(id) ON DELETE CASCADE,
 task_key TEXT NOT NULL, session_id TEXT UNIQUE, workflow_run_id TEXT UNIQUE,
 PRIMARY KEY(run_id, task_key)
);
CREATE TABLE orchestration_events (
 run_id TEXT NOT NULL REFERENCES orchestrations(id) ON DELETE CASCADE,
 event_key TEXT NOT NULL, message_id TEXT NOT NULL UNIQUE,
 PRIMARY KEY(run_id, event_key)
);
"#;

fn read(conn: &Connection, id: &str) -> Result<Orchestration> {
    let data: String = conn
        .query_row("SELECT data FROM orchestrations WHERE id = ?1", [id], |r| r.get(0))
        .optional()?
        .ok_or_else(|| invalid("Orchestration not found."))?;
    Ok(serde_json::from_str(&data)?)
}

fn save(conn: &Connection, run: &Orchestration) -> Result<()> {
    conn.execute(
        "UPDATE orchestrations SET data = ?2, status = ?3 WHERE id = ?1",
        params![run.id, serde_json::to_string(run)?, serde_json::to_value(run.status)?.as_str()],
    )?;
    Ok(())
}

fn require_active(run: &Orchestration, revision: i64) -> Result<()> {
    if run.status != Status::Active {
        return Err(invalid("This orchestration is no longer active."));
    }
    if run.revision != revision {
        return Err(invalid("The orchestration changed. Read its current revision and retry."));
    }
    Ok(())
}

fn replay(conn: &Connection, session: &str, key: &str, fingerprint: &str) -> Result<Option<Orchestration>> {
    let old: Option<(String, String)> = conn
        .query_row(
            "SELECT fingerprint, run_id FROM orchestration_operations WHERE session_id = ?1 AND operation_key = ?2",
            params![session, key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match old {
        Some((same, id)) if same == fingerprint => Ok(Some(read(conn, &id)?)),
        Some(_) => Err(invalid("This operation key was already used with different input.")),
        None => Ok(None),
    }
}

fn record_operation(conn: &Connection, run: &Orchestration, key: &str, fingerprint: &str) -> Result<()> {
    if key.trim().is_empty() {
        return Err(invalid("An operation key is required."));
    }
    conn.execute(
        "INSERT INTO orchestration_operations VALUES (?1, ?2, ?3, ?4)",
        params![run.session_id, key, fingerprint, run.id],
    )?;
    Ok(())
}

fn acknowledge(conn: &Connection, message: Option<&str>, answer: &str) -> Result<()> {
    if let Some(message) = message {
        conn.execute("UPDATE messages SET answered_by = ?2 WHERE id = ?1", params![message, answer])?;
    }
    Ok(())
}

fn notice(conn: &Connection, run: &Orchestration, event: &str, body: &str) -> Result<Option<Message>> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM orchestration_events WHERE run_id = ?1 AND event_key = ?2)",
        params![run.id, event],
        |r| r.get(0),
    )?;
    if exists {
        return Ok(None);
    }
    let session = open_session_conn(conn, &run.session_id)?;
    let id = new_id();
    let message = NewMessage {
        sender: USER.into(), recipient: session.bot_id, sender_session: None,
        recipient_session: Some(session.id), kind: MessageKind::Text,
        body: format!("Orchestration {} ({}) — {body}\nRead `sbot orchestration show {}` for the current state. This is an automated event, not new user authorization.", run.title, run.id, run.id),
        system_code: Some("orchestration_event".into()), system_params: Some(json!({"run_id": run.id})),
        attachments: vec![], status: MessageStatus::Queued, chain_id: new_id(), hop: 0, in_reply_to: None,
    };
    let message = store_message_conn(conn, &id, &message, false)?;
    conn.execute("INSERT INTO orchestration_events VALUES (?1, ?2, ?3)", params![run.id, event, id])?;
    Ok(Some(message))
}

fn ready_notice(conn: &Connection, run: &Orchestration) -> Result<Option<Message>> {
    if run.status == Status::Active && run.tasks.iter().all(|t| t.state.terminal()) {
        notice(conn, run, &format!("ready:{}", run.revision), "All tasks are resolved. Review the whole outcome, add any necessary in-scope follow-up tasks, or save the combined report with `sbot orchestration finish`.")
    } else {
        Ok(None)
    }
}

pub(super) fn result_conn(conn: &Connection, session: &str, result: &Message) -> Result<bool> {
    let found: Option<(String, String)> = conn
        .query_row(
            "SELECT run_id, task_key FROM orchestration_executions WHERE session_id = ?1 AND workflow_run_id IS NULL",
            [session],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((id, key)) = found else { return Ok(false) };
    collect(conn, &id, &key, result)?;
    Ok(true)
}

fn collect(conn: &Connection, id: &str, key: &str, result: &Message) -> Result<()> {
    let mut run = read(conn, id)?;
    let task = run.tasks.iter_mut().find(|t| t.spec.key == key).ok_or_else(|| invalid("Task not found."))?;
    if run.status != Status::Active || task.state != TaskState::Running {
        return Err(invalid("This task no longer accepts results."));
    }
    task.state = TaskState::AwaitingReview;
    task.result_message_id = Some(result.id.clone());
    task.result = Some(result.body.clone());
    task.attachments = result.attachments.clone();
    acknowledge(conn, task.request_message_id.as_deref(), &result.id)?;
    conn.execute(
        "UPDATE messages SET system_code = 'orchestration_event', system_params = ?2 WHERE id = ?1",
        params![result.id, json!({"run_id": id, "task": key}).to_string()],
    )?;
    run.revision += 1;
    save(conn, &run)?;
    Ok(())
}

pub(super) fn workflow_finished_conn(conn: &Connection, workflow: &str, result: Option<&Message>) -> Result<()> {
    let found: Option<(String, String)> = conn
        .query_row(
            "SELECT run_id, task_key FROM orchestration_executions WHERE workflow_run_id = ?1",
            [workflow],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((id, key)) = found {
        let result = result.ok_or_else(|| invalid("An orchestrated workflow must preserve its final result."))?;
        collect(conn, &id, &key, result)?;
    }
    Ok(())
}

pub(super) fn workflow_stopped_conn(conn: &Connection, workflow: &str) -> Result<()> {
    let found: Option<(String, String)> = conn
        .query_row(
            "SELECT run_id, task_key FROM orchestration_executions WHERE workflow_run_id = ?1",
            [workflow],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((id, key)) = found {
        let mut run = read(conn, &id)?;
        if run.status == Status::Active {
            if let Some(task) = run.tasks.iter_mut().find(|t| t.spec.key == key && !t.state.terminal()) {
                task.state = TaskState::Cancelled;
                task.review = Some("The workflow was stopped.".into());
                run.revision += 1;
                orch::block_dependents(&mut run);
                save(conn, &run)?;
                notice(conn, &run, &format!("stopped:{key}"), "A task was cancelled. Independent tasks continue.")?;
                ready_notice(conn, &run)?;
            }
        }
    }
    Ok(())
}

impl Store {
    pub fn get_orchestration(&self, id: &str) -> Result<Orchestration> {
        let run = read(&self.conn.lock().unwrap(), id)?;
        if !self.includes(&run.project_id) {
            return Err(crate::ProjectError::Mismatch.into());
        }
        Ok(run)
    }

    pub fn list_orchestrations(&self) -> Result<Vec<Orchestration>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT data FROM orchestrations WHERE ?1 IS NULL OR project_id = ?1 ORDER BY rowid")?;
        let rows =
            stmt.query_map([&self.project_id], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str::<Orchestration>(&r)?))
            .filter(|r: &Result<Orchestration>| r.as_ref().map_or(true, |r| self.includes(&r.project_id)))
            .collect()
    }

    pub fn orchestration_replay(&self, session: &str, key: &str, fingerprint: &str) -> Result<Option<Orchestration>> {
        self.check_owner("sessions", session)?;
        replay(&self.conn.lock().unwrap(), session, key, fingerprint)
    }

    pub fn create_orchestration(&self, run: &Orchestration, key: &str, fingerprint: &str) -> Result<Orchestration> {
        self.check_owner("sessions", &run.session_id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        if let Some(run) = replay(&tx, &run.session_id, key, fingerprint)? {
            return Ok(run);
        }
        let session = open_session_conn(&tx, &run.session_id)?;
        if session.project_id != run.project_id {
            return Err(crate::ProjectError::Mismatch.into());
        }
        let busy: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM orchestrations WHERE session_id = ?1 AND status = 'active')",
            [&run.session_id],
            |r| r.get(0),
        )?;
        if busy {
            return Err(invalid(
                "This Master conversation already has an active orchestration. Update it or start a new conversation.",
            ));
        }
        orch::validate_graph(&run.tasks)?;
        tx.execute(
            "INSERT INTO orchestrations VALUES (?1, ?2, ?3, ?4, 'active', ?5)",
            params![run.id, run.project_id, run.session_id, run.request_message_id, serde_json::to_string(run)?],
        )?;
        record_operation(&tx, run, key, fingerprint)?;
        // A committed orchestration owns this request even if the app exits before the CLI turn ends.
        acknowledge(&tx, Some(&run.request_message_id), &run.id)?;
        tx.commit()?;
        Ok(run.clone())
    }

    pub fn update_orchestration(
        &self,
        id: &str,
        revision: i64,
        key: &str,
        fingerprint: &str,
        add: Vec<orch::Task>,
        replace: Vec<orch::Task>,
        reviews: &[orch::Review],
        answered: Option<&str>,
    ) -> Result<Orchestration> {
        self.get_orchestration(id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut run = read(&tx, id)?;
        if let Some(run) = replay(&tx, &run.session_id, key, fingerprint)? {
            return Ok(run);
        }
        require_active(&run, revision)?;
        let mut keys = std::collections::HashSet::new();
        for task in replace {
            if !keys.insert(task.spec.key.clone()) {
                return Err(invalid("A task was replaced twice."));
            }
            let old = run
                .tasks
                .iter_mut()
                .find(|t| t.spec.key == task.spec.key)
                .ok_or_else(|| invalid("Task to replace not found."))?;
            if old.state != TaskState::Waiting {
                return Err(invalid("Only tasks that have not started can be replaced."));
            }
            *old = task;
        }
        run.tasks.extend(add);
        orch::validate_graph(&run.tasks)?;
        for review in reviews {
            let task = run
                .tasks
                .iter_mut()
                .find(|t| t.spec.key == review.task)
                .ok_or_else(|| invalid("Task to review not found."))?;
            if task.state != TaskState::AwaitingReview || review.explanation.trim().is_empty() {
                return Err(invalid("Review requires a completed execution and a nonempty explanation."));
            }
            task.state = if review.succeeded { TaskState::Succeeded } else { TaskState::Failed };
            task.review = Some(review.explanation.clone());
        }
        orch::block_dependents(&mut run);
        run.revision += 1;
        save(&tx, &run)?;
        record_operation(&tx, &run, key, fingerprint)?;
        acknowledge(&tx, answered, &run.id)?;
        ready_notice(&tx, &run)?;
        tx.commit()?;
        Ok(run)
    }

    pub fn dispatch_orchestration_bot(
        &self,
        id: &str,
        expected: &orch::Task,
        bot: &str,
        message_id: &str,
        message: &NewMessage,
    ) -> Result<Option<Routed>> {
        self.get_orchestration(id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut run = read(&tx, id)?;
        let key = &expected.spec.key;
        let pos = run.tasks.iter().position(|t| t.spec.key == *key).ok_or_else(|| invalid("Task not found."))?;
        if &run.tasks[pos] != expected || !orch::ready(&run, &run.tasks[pos]) {
            return Ok(None);
        }
        let session = create_session_conn(&tx, bot, Some(&run.tasks[pos].spec.title), Some(&run.session_id))?;
        let message = insert_message_conn(
            &tx,
            message_id,
            &NewMessage { recipient_session: Some(session.id.clone()), ..message.clone() },
        )?;
        tx.execute("UPDATE sessions SET open_request_id = ?2 WHERE id = ?1", params![session.id, message_id])?;
        let session = get_session_conn(&tx, &session.id)?.expect("created session");
        let task = &mut run.tasks[pos];
        task.session_id = Some(session.id.clone());
        task.request_message_id = Some(message_id.into());
        task.state = TaskState::Running;
        tx.execute("INSERT INTO orchestration_executions VALUES (?1, ?2, ?3, NULL)", params![id, key, session.id])?;
        run.revision += 1;
        save(&tx, &run)?;
        tx.commit()?;
        Ok(Some(Routed { session, message, created: true }))
    }

    pub fn dispatch_orchestration_workflow(
        &self,
        id: &str,
        expected: &orch::Task,
        workflow: &WorkflowRun,
        first: &VisitOpen,
    ) -> Result<Option<Routed>> {
        self.get_orchestration(id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut run = read(&tx, id)?;
        let key = &expected.spec.key;
        let pos = run.tasks.iter().position(|t| t.spec.key == *key).ok_or_else(|| invalid("Task not found."))?;
        if &run.tasks[pos] != expected || !orch::ready(&run, &run.tasks[pos]) {
            return Ok(None);
        }
        open_session_conn(&tx, &run.session_id)?;
        // The assigned snapshot remains runnable if the reusable workflow is subsequently edited/deleted.
        let source = workflow.workflow_id.as_ref().filter(|id| get_workflow_conn(&tx, id).ok().flatten().is_some());
        tx.execute("INSERT INTO workflow_runs (id, workflow_id, name, definition, request, request_files, opener_session, status, created_at, project_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'running', ?8, ?9)",
            params![workflow.id, source, workflow.name, serde_json::to_string(&workflow.def)?, workflow.request,
                serde_json::to_string(&workflow.request_files)?, run.session_id, now(), run.project_id])?;
        let opened = open_visit_conn(&tx, &workflow.id, 1, first)?;
        let task = &mut run.tasks[pos];
        task.session_id = Some(opened.session.id.clone());
        task.workflow_run_id = Some(workflow.id.clone());
        task.request_message_id = Some(opened.message.id.clone());
        task.state = TaskState::Running;
        tx.execute(
            "INSERT INTO orchestration_executions VALUES (?1, ?2, ?3, ?4)",
            params![id, key, opened.session.id, workflow.id],
        )?;
        run.revision += 1;
        save(&tx, &run)?;
        tx.commit()?;
        Ok(Some(opened))
    }

    pub fn fail_orchestration_dispatch(&self, id: &str, expected: &orch::Task, reason: &str) -> Result<()> {
        self.get_orchestration(id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut run = read(&tx, id)?;
        let key = &expected.spec.key;
        if run.status == Status::Active {
            if let Some(task) = run.tasks.iter_mut().find(|t| *t == expected && t.state == TaskState::Waiting) {
                task.state = TaskState::Failed;
                task.review = Some(reason.into());
                run.revision += 1;
                orch::block_dependents(&mut run);
                save(&tx, &run)?;
                notice(&tx, &run, &format!("failed:{key}"), &format!("Task {key} could not start: {reason}"))?;
                ready_notice(&tx, &run)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn finish_orchestration(
        &self,
        id: &str,
        revision: i64,
        report: &str,
        files: &[Attachment],
        answered: Option<&str>,
    ) -> Result<Message> {
        self.get_orchestration(id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut run = read(&tx, id)?;
        if run.status == Status::Completed && run.final_report.as_deref() == Some(report) && run.final_files == files {
            return get_message_conn(&tx, run.final_message_id.as_deref().unwrap())?
                .ok_or_else(|| invalid("The final report was removed."));
        }
        require_active(&run, revision)?;
        if report.trim().is_empty() || run.tasks.iter().any(|t| !t.state.terminal()) {
            return Err(invalid("A nonempty final report and resolved tasks (including every review) are required."));
        }
        let session = open_session_conn(&tx, &run.session_id)?;
        let message = NewMessage {
            sender: session.bot_id,
            recipient: USER.into(),
            sender_session: Some(session.id),
            recipient_session: None,
            kind: MessageKind::Text,
            body: report.into(),
            attachments: files.to_vec(),
            system_code: None,
            system_params: None,
            status: MessageStatus::Delivered,
            chain_id: new_id(),
            hop: 0,
            in_reply_to: Some(run.request_message_id.clone()),
        };
        let message = insert_message_conn(&tx, &new_id(), &message)?;
        acknowledge(&tx, Some(&run.request_message_id), &message.id)?;
        acknowledge(&tx, answered, &message.id)?;
        run.status = Status::Completed;
        run.final_message_id = Some(message.id.clone());
        run.final_report = Some(report.into());
        run.final_files = files.to_vec();
        run.ended_at = Some(now());
        run.revision += 1;
        save(&tx, &run)?;
        cancel_events(&tx, id)?;
        tx.commit()?;
        Ok(message)
    }

    pub fn cancel_orchestration(&self, id: &str, key: Option<&str>) -> Result<Orchestration> {
        self.get_orchestration(id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut run = read(&tx, id)?;
        if run.status != Status::Active {
            return Ok(run);
        }
        if key.is_some_and(|key| !run.tasks.iter().any(|t| t.spec.key == key)) {
            return Err(invalid("Task not found."));
        }
        for task in &mut run.tasks {
            if key.is_none_or(|key| task.spec.key == key) && !task.state.terminal() {
                task.state = TaskState::Cancelled;
                task.review = Some("Cancelled.".into());
            }
        }
        orch::block_dependents(&mut run);
        if key.is_none() {
            run.status = Status::Cancelled;
            run.ended_at = Some(now());
        }
        run.revision += 1;
        save(&tx, &run)?;
        if key.is_none() {
            cancel_events(&tx, id)?;
        } else {
            ready_notice(&tx, &run)?;
        }
        tx.commit()?;
        Ok(run)
    }

    pub fn set_orchestration_task_archived(&self, id: &str, key: &str, archive: bool) -> Result<()> {
        self.get_orchestration(id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut run = read(&tx, id)?;
        let task = run.tasks.iter_mut().find(|t| t.spec.key == key).ok_or_else(|| invalid("Task not found."))?;
        if !task.state.terminal() {
            return Err(invalid("Cancel or finish the task before marking it done."));
        }
        task.archived_at = archive.then(now);
        save(&tx, &run)?;
        tx.commit()?;
        Ok(())
    }

    pub fn orchestration_of_session(&self, session: &str) -> Result<Option<(Orchestration, Option<String>)>> {
        self.check_owner("sessions", session)?;
        let conn = self.conn.lock().unwrap();
        let parent: Option<String> = conn
            .query_row(
                "SELECT id FROM orchestrations WHERE session_id = ?1 ORDER BY rowid DESC LIMIT 1",
                [session],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = parent {
            return Ok(Some((read(&conn, &id)?, None)));
        }
        let found: Option<(String, String)> = conn.query_row(
            "WITH RECURSIVE ancestors(id, opened_by) AS (SELECT id, opened_by FROM sessions WHERE id = ?1 UNION SELECT s.id, s.opened_by FROM sessions s JOIN ancestors a ON s.id = a.opened_by) SELECT e.run_id, e.task_key FROM orchestration_executions e WHERE e.session_id IN (SELECT id FROM ancestors) OR e.workflow_run_id IN (SELECT run_id FROM workflow_run_sessions WHERE session_id IN (SELECT id FROM ancestors)) LIMIT 1", [session], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        found.map(|(id, key)| Ok((read(&conn, &id)?, Some(key)))).transpose()
    }
}

fn cancel_events(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("UPDATE messages SET status = 'cancelled' WHERE status = 'queued' AND (id IN (SELECT message_id FROM orchestration_events WHERE run_id = ?1) OR (system_code = 'orchestration_event' AND json_extract(system_params, '$.run_id') = ?1))", [id])?;
    Ok(())
}
