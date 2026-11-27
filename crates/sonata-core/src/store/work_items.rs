use super::*;
use crate::orchestration::{self as orch, invalid, Orchestration, TaskState};
use crate::work_items::WorkItem;

pub(super) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(r#"
CREATE TABLE work_items (
 id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id),
 parent_id TEXT REFERENCES work_items(id), title TEXT NOT NULL, instructions TEXT NOT NULL DEFAULT '',
 state TEXT NOT NULL CHECK(state IN ('pending','working','finished')),
 outcome TEXT, primary_session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
 followup_session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
 workflow_run_id TEXT REFERENCES workflow_runs(id) ON DELETE SET NULL,
 orchestration_id TEXT REFERENCES orchestrations(id) ON DELETE SET NULL,
 execution TEXT, manual_start INTEGER NOT NULL DEFAULT 0,
 result_message_id TEXT, created_at TEXT NOT NULL, finished_at TEXT, archived_at TEXT
);
CREATE INDEX work_items_parent ON work_items(parent_id);
CREATE INDEX work_items_project ON work_items(project_id, archived_at, state);
CREATE INDEX work_items_orchestration ON work_items(orchestration_id);
CREATE TABLE work_item_sessions (
 session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
 work_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE
);
CREATE INDEX work_item_sessions_owner ON work_item_sessions(work_item_id);
CREATE TABLE work_item_dependencies (
 task_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
 prerequisite_id TEXT NOT NULL REFERENCES work_items(id), PRIMARY KEY(task_id, prerequisite_id)
);
CREATE TABLE work_item_pending_results (
 session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
 body TEXT NOT NULL, files TEXT NOT NULL, outcome TEXT, created_at TEXT NOT NULL
);
CREATE TRIGGER work_item_parent_insert BEFORE INSERT ON work_items WHEN NEW.parent_id IS NOT NULL BEGIN
 SELECT CASE WHEN NEW.id = NEW.parent_id OR NEW.project_id != (SELECT project_id FROM work_items WHERE id = NEW.parent_id)
 THEN RAISE(ABORT, 'invalid task parent') END;
END;
CREATE TRIGGER work_item_parent_update BEFORE UPDATE OF parent_id, project_id ON work_items BEGIN
 SELECT CASE WHEN NEW.parent_id IS NOT OLD.parent_id OR NEW.project_id != OLD.project_id
 THEN RAISE(ABORT, 'task ownership cannot be changed') END;
END;
CREATE TRIGGER work_item_session_project BEFORE INSERT ON work_item_sessions BEGIN
 SELECT CASE WHEN (SELECT project_id FROM sessions WHERE id = NEW.session_id) !=
 (SELECT project_id FROM work_items WHERE id = NEW.work_item_id) THEN RAISE(ABORT, 'cross-project task session') END;
END;
CREATE TRIGGER work_item_archive AFTER UPDATE OF archived_at ON sessions BEGIN
 UPDATE work_items SET archived_at = NEW.archived_at, execution=CASE WHEN execution IS NULL THEN NULL ELSE json_set(execution,'$.archived_at',NEW.archived_at) END WHERE primary_session_id = NEW.id;
END;
CREATE TRIGGER work_item_title AFTER UPDATE OF title ON sessions WHEN NEW.title IS NOT NULL BEGIN
 UPDATE work_items SET title=NEW.title, execution=CASE WHEN execution IS NULL THEN NULL ELSE json_set(execution,'$.spec.title',NEW.title) END WHERE primary_session_id=NEW.id;
END;
CREATE TRIGGER work_item_session_removed BEFORE DELETE ON sessions BEGIN
 UPDATE work_items SET state='finished',outcome=COALESCE(outcome,'cancelled'),finished_at=COALESCE(finished_at,strftime('%Y-%m-%dT%H:%M:%fZ','now')),archived_at=COALESCE(archived_at,strftime('%Y-%m-%dT%H:%M:%fZ','now')) WHERE primary_session_id=OLD.id;
END;
"#)?;
    let sessions = conn
        .prepare(&format!("SELECT {SESSION_COLUMNS} FROM sessions ORDER BY created_at, id"))?
        .query_map([], session_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Resolve openers before descendants, without depending on equal timestamp ordering.
    let mut pending = sessions;
    while !pending.is_empty() {
        let before = pending.len();
        let mut rest = Vec::new();
        for session in pending {
            let waits = session.opened_by.as_ref().is_some_and(|id| {
                get_session_conn(conn, id).ok().flatten().is_some() && owner(conn, id).ok().flatten().is_none()
            });
            if waits {
                rest.push(session);
                continue;
            }
            attach_session(conn, &session)?;
            conn.execute("UPDATE work_items SET state = 'finished', finished_at = ?2, archived_at = ?3 WHERE primary_session_id = ?1",
                params![session.id, session.last_activity_at, session.archived_at])?;
        }
        if rest.len() == before {
            // Corrupt legacy opener cycles must not make startup loop forever.
            for mut session in rest {
                session.opened_by = None;
                attach_session(conn, &session)?;
            }
            break;
        }
        pending = rest;
    }
    let runs = conn
        .prepare("SELECT data FROM orchestrations ORDER BY rowid")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for data in runs {
        let run: Orchestration =
            serde_json::from_str(&data).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        save_graph(conn, &run)?;
    }
    conn.execute_batch(r#"
UPDATE work_items SET instructions=COALESCE((SELECT body FROM messages WHERE recipient_session=primary_session_id AND kind='text' ORDER BY seq LIMIT 1),'') WHERE instructions='';
UPDATE work_items SET workflow_run_id = (SELECT id FROM workflow_runs WHERE root_session = primary_session_id LIMIT 1)
 WHERE workflow_run_id IS NULL;
UPDATE work_items SET state = 'working', finished_at = NULL WHERE EXISTS (
 SELECT 1 FROM work_item_sessions ws JOIN messages m ON m.recipient_session = ws.session_id
 WHERE ws.work_item_id = work_items.id AND m.status IN ('queued','interrupted')) OR EXISTS (
 SELECT 1 FROM workflow_runs r WHERE r.id = work_items.workflow_run_id AND r.status IN ('running','paused'));
"#)?;
    Ok(())
}

pub(super) fn owner(conn: &Connection, session: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT work_item_id FROM work_item_sessions WHERE session_id = ?1", [session], |r| r.get(0))
        .optional()
}

pub(super) fn reopen(conn: &Connection, session: &str, message: &str) -> Result<()> {
    let Some(mut id) = owner(conn, session)? else { return Ok(()) };
    let was_finished: bool =
        conn.query_row("SELECT state='finished' FROM work_items WHERE id=?1", [&id], |r| r.get(0))?;
    if !was_finished {
        return Ok(());
    }
    let mut direct = true;
    loop {
        let (parent, graph, data): (Option<String>, Option<String>, Option<String>) =
            conn.query_row("SELECT parent_id,orchestration_id,execution FROM work_items WHERE id=?1", [&id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?;
        let data = data
            .map(|s| -> Result<String> {
                let mut task: orch::Task = serde_json::from_str(&s)?;
                task.state = TaskState::Running;
                task.result = None;
                task.result_message_id = None;
                task.review = None;
                task.archived_at = None;
                Ok(serde_json::to_string(&task)?)
            })
            .transpose()?;
        conn.execute("UPDATE work_items SET state='working',outcome=NULL,finished_at=NULL,archived_at=NULL,result_message_id=NULL,execution=?2,followup_session_id=CASE WHEN ?3 THEN ?4 ELSE primary_session_id END WHERE id=?1",params![id,data,direct,session])?;
        conn.execute("UPDATE sessions SET archived_at=NULL WHERE id IN (SELECT session_id FROM work_item_sessions WHERE work_item_id=?1)",[&id])?;
        if let Some(graph) = graph {
            conn.execute("UPDATE sessions SET open_request_id=(SELECT json_extract(execution,'$.request_message_id') FROM work_items WHERE id=?1) WHERE id=(SELECT primary_session_id FROM work_items WHERE id=?1)",[&id])?;
            let mut run = orchestration::read(conn, &graph)?;
            run.status = orch::Status::Active;
            run.ended_at = None;
            run.final_report = None;
            run.final_message_id = None;
            run.revision += 1;
            orchestration::save(conn, &run)?;
            orchestration::notice(conn,&run,&format!("reopened:{message}"),"A child task received follow-up work and reopened. Previously released dependents are not restarted. Review the new result when it arrives.")?;
        }
        match parent {
            Some(parent) => {
                id = parent;
                direct = false;
            }
            None => break,
        }
    }
    Ok(())
}

pub(super) fn attach_session(conn: &Connection, session: &Session) -> rusqlite::Result<()> {
    let inherited = session.opened_by.as_deref().map(|s| owner(conn, s)).transpose()?.flatten();
    let id = inherited.as_deref().unwrap_or(&session.id);
    if inherited.is_none() {
        conn.execute("INSERT OR IGNORE INTO work_items (id, project_id, title, state, primary_session_id, created_at) VALUES (?1, ?2, ?3, 'working', ?1, ?4)",
            params![id, session.project_id, session.title.as_deref().unwrap_or("Task"), session.created_at])?;
    }
    conn.execute("INSERT OR REPLACE INTO work_item_sessions VALUES (?1, ?2)", params![session.id, id])?;
    Ok(())
}

pub(super) fn execution_id(run: &str, task: &orch::Task) -> String {
    task.work_item_id.clone().unwrap_or_else(|| format!("{run}:{}", task.spec.key))
}

fn dependencies(conn: &Connection, id: &str) -> Result<Vec<(String, orch::Task)>> {
    let rows = conn.prepare("SELECT w.id,w.execution FROM work_item_dependencies d JOIN work_items w ON w.id=d.prerequisite_id WHERE d.task_id=?1")?
        .query_map([id], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter().map(|(id, data)| Ok((id, serde_json::from_str(&data)?))).collect()
}

/// Dependency snapshots are used only for readiness/validation, never persisted as extra tasks.
pub(super) fn dependency_context(conn: &Connection, run: &Orchestration) -> Result<Orchestration> {
    let mut context = run.clone();
    for task in &run.tasks {
        for (id, saved) in dependencies(conn, &execution_id(&run.id, task))? {
            let key = crate::work_items::dependency_key(&id);
            if !task.spec.depends_on.contains(&key) || context.tasks.iter().any(|t| t.spec.key == key) {
                continue;
            }
            let mut reference = run.tasks.iter().find(|t| execution_id(&run.id, t) == id).cloned().unwrap_or(saved);
            reference.spec.key = key;
            reference.spec.depends_on.clear();
            context.tasks.push(reference);
        }
    }
    Ok(context)
}

pub(super) fn save_graph(conn: &Connection, run: &Orchestration) -> rusqlite::Result<()> {
    let parent = owner(conn, &run.session_id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    conn.execute("UPDATE work_items SET state = CASE WHEN ?2='finished' AND EXISTS(SELECT 1 FROM workflow_runs WHERE id=work_items.workflow_run_id AND status IN ('running','paused')) THEN 'working' ELSE ?2 END, finished_at = ?3 WHERE id = ?1",
        params![parent, if run.status == orch::Status::Active { "working" } else { "finished" }, run.ended_at])?;
    let parent_archive: Option<String> =
        conn.query_row("SELECT archived_at FROM work_items WHERE id=?1", [&parent], |r| r.get(0))?;
    for source in &run.tasks {
        let mut task = source.clone();
        if task.archived_at.is_none() {
            task.archived_at = parent_archive.clone();
        }
        let task = &task;
        let id = execution_id(&run.id, task);
        let state = match task.state {
            TaskState::Waiting => "pending",
            TaskState::Running => "working",
            _ => "finished",
        };
        let outcome = match task.state {
            TaskState::Succeeded => Some("succeeded"),
            TaskState::Failed => Some("failed"),
            TaskState::Cancelled => Some("cancelled"),
            TaskState::Blocked => Some("blocked"),
            _ => None,
        };
        let data = serde_json::to_string(task).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        conn.execute("INSERT INTO work_items (id, project_id, parent_id, title, instructions, state, outcome, primary_session_id, workflow_run_id, orchestration_id, execution, manual_start, result_message_id, created_at, finished_at, archived_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16) ON CONFLICT(id) DO UPDATE SET orchestration_id=excluded.orchestration_id,title=excluded.title, instructions=excluded.instructions, state=excluded.state, outcome=excluded.outcome, primary_session_id=excluded.primary_session_id, workflow_run_id=excluded.workflow_run_id, execution=excluded.execution, manual_start=excluded.manual_start, result_message_id=excluded.result_message_id, finished_at=CASE WHEN excluded.state='finished' THEN COALESCE(work_items.finished_at, excluded.finished_at) ELSE NULL END, archived_at=excluded.archived_at",
            params![id, run.project_id, parent, task.spec.title, task.spec.instructions, state, outcome, task.session_id, task.workflow_run_id, run.id, data, task.spec.manual_start, task.result_message_id, run.created_at, (state == "finished").then(now), task.archived_at])?;
        if let Some(session) = &task.session_id {
            // A child execution owns its supporting conversations, never its siblings.
            let ids = tree_conn(conn, session)?.into_iter().map(|s| s.id).collect::<Vec<_>>();
            for sid in ids {
                let current = owner(conn, &sid)?;
                if current.as_deref() == Some(&parent) || current.as_deref() == Some(&id) {
                    conn.execute("INSERT OR REPLACE INTO work_item_sessions VALUES (?1,?2)", params![sid, id])?;
                }
            }
        }
    }
    for task in &run.tasks {
        let id = execution_id(&run.id, task);
        let previous = dependencies(conn, &id).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        conn.execute("DELETE FROM work_item_dependencies WHERE task_id=?1", [&id])?;
        for key in &task.spec.depends_on {
            let dependency = if let Some(task) = run.tasks.iter().find(|t| &t.spec.key == key) {
                execution_id(&run.id, task)
            } else {
                previous
                    .iter()
                    .find(|(id, _)| crate::work_items::dependency_key(id) == *key)
                    .map(|(id, _)| id.clone())
                    .ok_or(rusqlite::Error::QueryReturnedNoRows)?
            };
            conn.execute("INSERT INTO work_item_dependencies VALUES (?1,?2)", params![id, dependency])?;
        }
    }
    let cycle: bool = conn.query_row("WITH RECURSIVE paths(task_id,prerequisite_id) AS (SELECT task_id,prerequisite_id FROM work_item_dependencies UNION SELECT p.task_id,d.prerequisite_id FROM paths p JOIN work_item_dependencies d ON d.task_id=p.prerequisite_id) SELECT EXISTS(SELECT 1 FROM paths WHERE task_id=prerequisite_id)",[],|r|r.get(0))?;
    if cycle {
        return Err(rusqlite::Error::InvalidParameterName("Task dependencies contain a cycle.".into()));
    }
    Ok(())
}

pub(super) fn graph_tasks(conn: &Connection, run: &str) -> Result<Vec<orch::Task>> {
    let rows = conn
        .prepare("SELECT execution FROM work_items WHERE orchestration_id=?1 ORDER BY rowid")?
        .query_map([run], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter().map(|s| Ok(serde_json::from_str(&s)?)).collect()
}

pub(super) fn complete(conn: &Connection, session: &str, result: &Message) -> Result<()> {
    let Some(id) = owner(conn, session)? else { return Ok(()) };
    let unresolved: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM work_items WHERE parent_id=?1 AND (state!='finished' OR json_extract(execution,'$.state')='awaiting_review'))", [&id], |r| r.get(0))?;
    if unresolved {
        return Err(invalid("This task still has unfinished children or reviews."));
    }
    conn.execute(
        "UPDATE work_items SET state='finished', outcome='succeeded', result_message_id=?2, finished_at=?3 WHERE id=?1",
        params![id, result.id, now()],
    )?;
    conn.execute("DELETE FROM work_item_pending_results WHERE session_id=?1", [session])?;
    Ok(())
}

impl Store {
    pub fn work_item_dependency_context(&self, run: &Orchestration) -> Result<Orchestration> {
        self.check_owner("sessions", &run.session_id)?;
        dependency_context(&self.conn.lock().unwrap(), run)
    }

    pub fn reconcile_work_item_dependencies(&self, id: &str) -> Result<Orchestration> {
        self.get_orchestration(id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut run = orchestration::read(&tx, id)?;
        if run.status == orch::Status::Active {
            let mut context = dependency_context(&tx, &run)?;
            orch::block_dependents(&mut context);
            let mut changed = false;
            for task in &mut run.tasks {
                if let Some(updated) = context.tasks.iter().find(|t| t.spec.key == task.spec.key) {
                    if task != updated {
                        *task = updated.clone();
                        changed = true;
                    }
                }
            }
            if changed {
                run.revision += 1;
                orchestration::save(&tx, &run)?;
                orchestration::notice(
                    &tx,
                    &run,
                    &format!("blocked:{}", run.revision),
                    "A task is blocked because a prerequisite did not succeed. Independent work can continue.",
                )?;
            }
        }
        tx.commit()?;
        Ok(run)
    }

    pub fn archive_work_item(&self, id: &str, archive: bool) -> Result<Vec<Session>> {
        self.check_owner("work_items", id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mark: Option<String> = tx
            .query_row("SELECT archived_at FROM work_items WHERE id=?1", [id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| invalid("Task not found."))?;
        let ids=tx.prepare("WITH RECURSIVE tree(id) AS (SELECT id FROM work_items WHERE id=?1 UNION SELECT w.id FROM work_items w JOIN tree t ON w.parent_id=t.id) SELECT id FROM tree")?.query_map([id],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let at = now();
        let mut sessions = Vec::new();
        for task in ids {
            let (state, old): (String, Option<String>) =
                tx.query_row("SELECT state,archived_at FROM work_items WHERE id=?1", [&task], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
            if archive && state != "finished" {
                return Err(invalid("Finish or cancel all work before marking Done."));
            }
            if (archive && old.is_some()) || (!archive && (mark.is_none() || old != mark)) {
                continue;
            }
            let value = archive.then_some(&at);
            tx.execute("UPDATE work_items SET archived_at=?2 WHERE id=?1", params![task, value])?;
            let session_ids = tx
                .prepare("SELECT session_id FROM work_item_sessions WHERE work_item_id=?1")?
                .query_map([&task], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for sid in session_ids {
                tx.execute("UPDATE sessions SET archived_at=?2 WHERE id=?1", params![sid, value])?;
                if let Some(s) = get_session_conn(&tx, &sid)? {
                    sessions.push(s);
                }
            }
            // The legacy graph adapter reads this canonical row too.
            tx.execute("UPDATE work_items SET execution=json_set(execution,'$.archived_at',?2) WHERE id=?1 AND execution IS NOT NULL",params![task,value])?;
        }
        if !archive {
            let ancestors=tx.prepare("WITH RECURSIVE a(id) AS (SELECT parent_id FROM work_items WHERE id=?1 UNION SELECT w.parent_id FROM work_items w JOIN a ON w.id=a.id) SELECT id FROM a WHERE id IS NOT NULL")?.query_map([id],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            for parent in ancestors {
                tx.execute("UPDATE work_items SET archived_at=NULL,execution=CASE WHEN execution IS NULL THEN NULL ELSE json_set(execution,'$.archived_at',NULL) END WHERE id=?1",[&parent])?;
                tx.execute("UPDATE sessions SET archived_at=NULL WHERE id IN (SELECT session_id FROM work_item_sessions WHERE work_item_id=?1)",[&parent])?;
                let ids = tx
                    .prepare("SELECT session_id FROM work_item_sessions WHERE work_item_id=?1")?
                    .query_map([&parent], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                for sid in ids {
                    if let Some(session) = get_session_conn(&tx, &sid)? {
                        sessions.push(session);
                    }
                }
            }
        }
        tx.commit()?;
        Ok(sessions)
    }

    pub fn release_pending_work_item(&self, id: &str) -> Result<()> {
        self.check_owner("work_items", id)?;
        self.conn.lock().unwrap().execute("UPDATE work_items SET manual_start=0,execution=json_set(execution,'$.spec.manual_start',json('false')) WHERE id=?1 AND state='pending'",[id])?;
        Ok(())
    }

    pub fn cancel_work_item_record(&self, id: &str) -> Result<()> {
        self.check_owner("work_items", id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE work_items SET state='finished',outcome='cancelled',finished_at=?2,execution=CASE WHEN execution IS NULL THEN NULL ELSE json_set(execution,'$.state','cancelled') END WHERE id=?1",
            params![id, now()],
        )?;
        tx.execute("DELETE FROM work_item_pending_results WHERE session_id IN (SELECT session_id FROM work_item_sessions WHERE work_item_id=?1)", [id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn work_item_cancelled_session(&self, session: &str) -> Result<bool> {
        self.check_owner("sessions", session)?;
        Ok(self.conn.lock().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM work_items w JOIN work_item_sessions s ON s.work_item_id=w.id WHERE s.session_id=?1 AND w.outcome='cancelled')", [session], |r| r.get(0))?)
    }

    pub fn cancel_pending_work_item(&self, id: &str) -> Result<()> {
        self.check_owner("work_items", id)?;
        self.conn.lock().unwrap().execute("UPDATE work_items SET state='finished',outcome='cancelled',finished_at=?2,execution=json_set(execution,'$.state','cancelled') WHERE id=?1 AND state='pending' AND orchestration_id IS NULL",params![id,now()])?;
        Ok(())
    }

    pub fn fail_pending_work_item(&self, id: &str, explanation: &str) -> Result<()> {
        self.check_owner("work_items", id)?;
        self.conn.lock().unwrap().execute("UPDATE work_items SET state='finished',outcome='failed',finished_at=?2,execution=json_set(execution,'$.state','failed','$.review',?3) WHERE id=?1 AND state='pending' AND orchestration_id IS NULL",params![id,now(),explanation])?;
        Ok(())
    }

    pub fn session_is_primary_task(&self, session: &str) -> Result<bool> {
        self.check_owner("sessions", session)?;
        Ok(self.conn.lock().unwrap().query_row(
            "SELECT EXISTS(SELECT 1 FROM work_items WHERE primary_session_id=?1)",
            [session],
            |r| r.get(0),
        )?)
    }

    pub fn session_is_task_followup(&self, session: &str) -> Result<bool> {
        self.check_owner("sessions", session)?;
        Ok(self.conn.lock().unwrap().query_row(
            "SELECT EXISTS(SELECT 1 FROM work_items WHERE followup_session_id=?1)",
            [session],
            |r| r.get(0),
        )?)
    }

    pub fn has_pending_task_result(&self, session: &str) -> Result<bool> {
        self.check_owner("sessions", session)?;
        Ok(self.conn.lock().unwrap().query_row(
            "SELECT EXISTS(SELECT 1 FROM work_item_pending_results WHERE session_id=?1)",
            [session],
            |r| r.get(0),
        )?)
    }

    pub fn session_has_workflow(&self, session: &str) -> Result<bool> {
        self.check_owner("sessions", session)?;
        Ok(self.conn.lock().unwrap().query_row(
            "SELECT EXISTS(SELECT 1 FROM workflow_run_sessions WHERE session_id=?1)",
            [session],
            |r| r.get(0),
        )?)
    }

    pub fn stage_work_item_result(
        &self,
        session: &str,
        body: &str,
        files: &[String],
        outcome: Option<&str>,
    ) -> Result<bool> {
        self.check_owner("sessions", session)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let item = owner(&tx, session)?;
        let owns_completion:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM work_items WHERE id=?1 AND (primary_session_id=?2 OR followup_session_id=?2)) OR EXISTS(SELECT 1 FROM workflow_visits WHERE session_id=?2 AND ended_at IS NULL) OR EXISTS(SELECT 1 FROM orchestrations WHERE session_id=?2 AND status='active')",params![item,session],|r|r.get(0))?;
        if !owns_completion {
            return Ok(false);
        }
        let waiting:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM work_items WHERE parent_id=?1 AND (state!='finished' OR json_extract(execution,'$.state')='awaiting_review'))",[item],|r|r.get(0))?;
        if waiting {
            tx.execute("INSERT INTO work_item_pending_results VALUES (?1,?2,?3,?4,?5) ON CONFLICT(session_id) DO UPDATE SET body=excluded.body,files=excluded.files,outcome=excluded.outcome,created_at=excluded.created_at",params![session,body,serde_json::to_string(files)?,outcome,now()])?;
        }
        tx.commit()?;
        Ok(waiting)
    }

    pub fn ready_work_item_results(&self) -> Result<Vec<(String, String, Vec<String>, Option<String>)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt=conn.prepare("SELECT p.session_id,p.body,p.files,p.outcome FROM work_item_pending_results p JOIN work_item_sessions ws ON ws.session_id=p.session_id JOIN work_items w ON w.id=ws.work_item_id WHERE (?1 IS NULL OR w.project_id=?1) AND w.state='working' AND w.archived_at IS NULL AND NOT EXISTS (SELECT 1 FROM work_items c WHERE c.parent_id=w.id AND (c.state!='finished' OR c.outcome IS NOT 'succeeded'))")?;
        let rows = stmt
            .query_map([&self.project_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(id, body, files, outcome)| Ok((id, body, serde_json::from_str(&files)?, outcome)))
            .collect()
    }

    pub fn own_orchestration(&self, session: &str) -> Result<Option<Orchestration>> {
        self.check_owner("sessions", session)?;
        let conn = self.conn.lock().unwrap();
        let id: Option<String> = conn
            .query_row("SELECT id FROM orchestrations WHERE session_id=?1 AND status='active'", [session], |r| r.get(0))
            .optional()?;
        id.map(|id| orchestration::read(&conn, &id)).transpose()
    }

    pub fn insert_pending_work_item(
        &self,
        task: &orch::Task,
        parent: Option<&str>,
        dependencies: &[String],
    ) -> Result<WorkItem> {
        self.require_active()?;
        let id = new_id();
        let mut task = task.clone();
        task.work_item_id = Some(id.clone());
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        if let Some(parent) = parent {
            let project: String =
                tx.query_row("SELECT project_id FROM work_items WHERE id=?1 AND archived_at IS NULL", [parent], |r| {
                    r.get(0)
                })?;
            if project != self.project_key() {
                return Err(crate::ProjectError::Mismatch.into());
            }
        }
        tx.execute("INSERT INTO work_items (id,project_id,parent_id,title,instructions,state,execution,manual_start,created_at) VALUES (?1,?2,?3,?4,?5,'pending',?6,?7,?8)",
            params![id,self.project_key(),parent,task.spec.title,task.spec.instructions,serde_json::to_string(&task)?,task.spec.manual_start,now()])?;
        for dep in dependencies {
            let sibling: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM work_items WHERE id=?1 AND parent_id=?2 AND project_id=?3)",
                params![dep, parent, self.project_key()],
                |r| r.get(0),
            )?;
            if !sibling {
                return Err(invalid("Dependencies must be sibling tasks."));
            }
            tx.execute("INSERT INTO work_item_dependencies VALUES (?1,?2)", params![id, dep])?;
        }
        tx.commit()?;
        drop(conn);
        self.list_work_items()?.into_iter().find(|w| w.id == id).ok_or_else(|| invalid("Task not found."))
    }

    pub fn start_pending_work_item(
        &self,
        id: &str,
        bot: Option<&str>,
        workflow: Option<(&NewRun, &VisitOpen)>,
    ) -> Result<Option<Routed>> {
        self.check_owner("work_items", id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let data: Option<String> = tx
            .query_row(
                "SELECT execution FROM work_items WHERE id=?1 AND state='pending' AND orchestration_id IS NULL",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(data) = data else { return Ok(None) };
        let mut task: orch::Task = serde_json::from_str(&data)?;
        let routed = if let Some((run, first)) = workflow {
            tx.execute("INSERT INTO workflow_runs (id,workflow_id,name,definition,request,request_files,status,created_at,project_id) VALUES (?1,?2,?3,?4,?5,?6,'running',?7,?8)",params![run.id,Some(&run.workflow_id).filter(|id|get_workflow_conn(&tx,id).ok().flatten().is_some()),run.name,serde_json::to_string(&run.def)?,run.request,serde_json::to_string(&run.request_files)?,now(),self.project_key()])?;
            task.workflow_run_id = Some(run.id.clone());
            open_visit_conn(&tx, &run.id, 1, first)?
        } else {
            let bot = bot.ok_or_else(|| invalid("Task has no executor."))?;
            let session = create_session_conn(&tx, bot, Some(&task.spec.title), None)?;
            let message = insert_message_conn(
                &tx,
                &new_id(),
                &NewMessage {
                    sender: USER.into(),
                    recipient: bot.into(),
                    sender_session: None,
                    recipient_session: Some(session.id.clone()),
                    kind: MessageKind::Text,
                    body: task.spec.instructions.clone(),
                    attachments: vec![],
                    system_code: None,
                    system_params: None,
                    status: MessageStatus::Queued,
                    chain_id: new_id(),
                    hop: 0,
                    in_reply_to: None,
                },
            )?;
            Routed { session, message, created: true }
        };
        let temporary = owner(&tx, &routed.session.id)?;
        tx.execute("INSERT OR REPLACE INTO work_item_sessions VALUES (?1,?2)", params![routed.session.id, id])?;
        if let Some(temporary) = temporary.filter(|old| old != id) {
            tx.execute("DELETE FROM work_items WHERE id=?1", [temporary])?;
        }
        task.state = TaskState::Running;
        task.session_id = Some(routed.session.id.clone());
        task.request_message_id = Some(routed.message.id.clone());
        tx.execute("UPDATE work_items SET state='working',manual_start=0,primary_session_id=?2,workflow_run_id=?3,execution=?4 WHERE id=?1",params![id,routed.session.id,task.workflow_run_id,serde_json::to_string(&task)?])?;
        tx.commit()?;
        Ok(Some(routed))
    }

    pub fn work_item_of_session(&self, session: &str) -> Result<Option<String>> {
        self.check_owner("sessions", session)?;
        Ok(owner(&self.conn.lock().unwrap(), session)?)
    }

    pub fn list_work_items(&self) -> Result<Vec<WorkItem>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id,project_id,parent_id,title,instructions,state,outcome,primary_session_id,workflow_run_id,orchestration_id,execution,manual_start,created_at,finished_at,archived_at,result_message_id FROM work_items WHERE ?1 IS NULL OR project_id=?1 ORDER BY created_at,rowid")?;
        let mut items = stmt
            .query_map([&self.project_id], |r| {
                let data: Option<String> = r.get(10)?;
                let execution = data
                    .map(|s| {
                        serde_json::from_str(&s).map_err(|e| {
                            rusqlite::Error::FromSqlConversionFailure(10, rusqlite::types::Type::Text, Box::new(e))
                        })
                    })
                    .transpose()?;
                Ok(WorkItem {
                    id: r.get(0)?,
                    project_id: r.get(1)?,
                    parent_id: r.get(2)?,
                    title: r.get(3)?,
                    instructions: r.get(4)?,
                    state: r.get(5)?,
                    outcome: r.get(6)?,
                    primary_session_id: r.get(7)?,
                    workflow_run_id: r.get(8)?,
                    orchestration_id: r.get(9)?,
                    execution,
                    manual_start: r.get(11)?,
                    created_at: r.get(12)?,
                    finished_at: r.get(13)?,
                    archived_at: r.get(14)?,
                    result_message_id: r.get(15)?,
                    session_ids: vec![],
                    depends_on: vec![],
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for item in &mut items {
            item.session_ids = conn
                .prepare("SELECT session_id FROM work_item_sessions WHERE work_item_id=?1 ORDER BY rowid")?
                .query_map([&item.id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            item.depends_on = conn
                .prepare("SELECT prerequisite_id FROM work_item_dependencies WHERE task_id=?1")?
                .query_map([&item.id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
        }
        Ok(items)
    }

    pub fn finish_standalone_task(&self, session: &str, body: &str, files: &[Attachment]) -> Result<Message> {
        self.check_owner("sessions", session)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let s = open_session_conn(&tx, session)?;
        let id = owner(&tx, session)?.ok_or_else(|| invalid("Task not found."))?;
        let primary: bool = tx.query_row(
            "SELECT primary_session_id=?2 OR followup_session_id IS ?2 FROM work_items WHERE id=?1",
            params![id, session],
            |r| r.get(0),
        )?;
        if !primary {
            return Err(invalid("Only the task's primary conversation can finish it without an open request."));
        }
        let previous: Option<String> = tx
            .query_row("SELECT result_message_id FROM work_items WHERE id=?1 AND state='finished'", [&id], |r| r.get(0))
            .optional()?
            .flatten();
        if let Some(previous) = previous {
            if let Some(m) = get_message_conn(&tx, &previous)? {
                if m.body == body && m.attachments == files {
                    return Ok(m);
                }
            }
            return Err(invalid("This task has already finished."));
        }
        let parent: Option<String> = tx.query_row("SELECT o.session_id FROM work_items w JOIN orchestrations o ON o.id=w.orchestration_id WHERE w.id=?1 AND w.followup_session_id=?2", params![id,session], |r| r.get(0)).optional()?;
        let parent = parent.map(|id| open_session_conn(&tx, &id)).transpose()?;
        let m = NewMessage {
            sender: s.bot_id,
            recipient: parent.as_ref().map_or_else(|| USER.into(), |s| s.bot_id.clone()),
            sender_session: Some(s.id),
            recipient_session: parent.as_ref().map(|s| s.id.clone()),
            kind: MessageKind::Text,
            body: body.into(),
            attachments: files.to_vec(),
            system_code: None,
            system_params: None,
            status: if parent.is_some() { MessageStatus::Queued } else { MessageStatus::Delivered },
            chain_id: new_id(),
            hop: 0,
            in_reply_to: None,
        };
        let m = insert_message_conn(&tx, &new_id(), &m)?;
        if !orchestration::result_conn(&tx, session, &m)? {
            complete(&tx, session, &m)?;
        }
        tx.execute("DELETE FROM work_item_pending_results WHERE session_id=?1", [session])?;
        tx.execute("UPDATE sessions SET open_request_id=NULL WHERE id=?1", [session])?;
        tx.execute("UPDATE messages SET answered_by=?2 WHERE recipient_session=?1 AND answered_by IS NULL AND status='delivered' AND kind='text'",params![session,m.id])?;
        tx.commit()?;
        Ok(m)
    }

    pub fn release_work_item(&self, id: &str) -> Result<String> {
        self.check_owner("work_items", id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let run_id: Option<String> = tx
            .query_row("SELECT orchestration_id FROM work_items WHERE id=?1 AND state='pending'", [id], |r| r.get(0))
            .optional()?
            .flatten();
        let run_id = run_id.ok_or_else(|| invalid("Only pending tasks can be started."))?;
        let mut run = orchestration::read(&tx, &run_id)?;
        let task =
            run.tasks.iter_mut().find(|t| execution_id(&run_id, t) == id).ok_or_else(|| invalid("Task not found."))?;
        task.spec.manual_start = false;
        run.revision += 1;
        orchestration::save(&tx, &run)?;
        tx.commit()?;
        Ok(run_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn legacy(path: &Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        for migration in &MIGRATIONS[..15] {
            migration.apply(&conn).unwrap();
        }
        conn.pragma_update(None, "user_version", 15).unwrap();
        conn.execute_batch("INSERT INTO bots(id,handle,display_name,color,provider,instructions,created_at,updated_at,project_id) VALUES ('b','worker','Worker','#123456','claude','','2026','2026','default'); INSERT INTO sessions(id,bot_id,number,title,created_at,last_activity_at,project_id,provider_session_id,archived_at) VALUES ('root','b',1,'History','2026','2026','default','keep-context','archive-mark'); INSERT INTO sessions(id,bot_id,number,title,opened_by,created_at,last_activity_at,project_id,archived_at) VALUES ('helper','b',2,'Support','root','2026','2026','default','archive-mark');").unwrap();
        conn
    }
    #[test]
    fn migration_preserves_sessions_archive_and_provider_context_and_makes_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sonata.db");
        drop(legacy(&path));
        let store = Store::open(&path).unwrap();
        let items = store.list_work_items().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "root");
        assert_eq!(items[0].session_ids, vec!["root", "helper"]);
        assert_eq!(items[0].archived_at.as_deref(), Some("archive-mark"));
        assert_eq!(items[0].state, "finished");
        assert_eq!(items[0].outcome, None);
        assert_eq!(store.get_session("root").unwrap().unwrap().provider_session_id.as_deref(), Some("keep-context"));
        let backup = Connection::open(tmp.path().join("sonata.db.work-items.bak")).unwrap();
        assert_eq!(backup.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 15);
        assert_eq!(
            store.conn.lock().unwrap().query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0)).unwrap(),
            0
        );
        drop(store);
        assert_eq!(Store::open(&path).unwrap().list_work_items().unwrap().len(), 1);
    }
    #[test]
    fn malformed_legacy_graph_rolls_back_the_entire_migration() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sonata.db");
        let conn = legacy(&path);
        conn.execute("INSERT INTO orchestrations VALUES ('bad','default','root','request','completed','{')", [])
            .unwrap();
        drop(conn);
        assert!(Store::open(&path).is_err());
        let conn = Connection::open(&path).unwrap();
        assert_eq!(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 15);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE name='work_items'", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    }
}
