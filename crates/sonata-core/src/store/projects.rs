use super::*;

pub const DEFAULT_PROJECT: &str = "default";
const COLUMNS: &str = "id, name, default_directory, default_provider, master_id, archived_at, created_at, updated_at";

pub(super) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(r#"
CREATE TABLE projects (
 id TEXT PRIMARY KEY, name TEXT NOT NULL COLLATE NOCASE UNIQUE,
 default_directory TEXT, default_provider TEXT NOT NULL,
 master_id TEXT,
 archived_at TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
 legacy_workspace INTEGER NOT NULL DEFAULT 0
);
INSERT INTO projects
 SELECT 'default', 'Default', (SELECT value FROM settings WHERE key = 'project_folder'),
 COALESCE((SELECT provider FROM bots WHERE id = (SELECT value FROM settings WHERE key = 'sonata_master')), 'claude'),
 (SELECT value FROM settings WHERE key = 'sonata_master'),
 NULL, strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1;
CREATE TABLE bots_project (
 id TEXT PRIMARY KEY, handle TEXT NOT NULL, display_name TEXT NOT NULL, color TEXT NOT NULL,
 provider TEXT NOT NULL, model TEXT, instructions TEXT NOT NULL, config_version INTEGER NOT NULL DEFAULT 1,
 created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
 project_id TEXT NOT NULL REFERENCES projects(id), UNIQUE(project_id, handle)
);
INSERT INTO bots_project SELECT id, handle, display_name, color, provider, model, instructions, config_version,
 created_at, updated_at, 'default' FROM bots;
DROP TABLE bots;
ALTER TABLE bots_project RENAME TO bots;
ALTER TABLE sessions ADD COLUMN project_id TEXT NOT NULL DEFAULT 'default' REFERENCES projects(id);
ALTER TABLE tasks ADD COLUMN project_id TEXT NOT NULL DEFAULT 'default' REFERENCES projects(id);
ALTER TABLE workflows ADD COLUMN project_id TEXT NOT NULL DEFAULT 'default' REFERENCES projects(id);
ALTER TABLE workflow_runs ADD COLUMN project_id TEXT NOT NULL DEFAULT 'default' REFERENCES projects(id);
DROP INDEX tasks_name;
CREATE UNIQUE INDEX tasks_name ON tasks(project_id, name COLLATE NOCASE);
DROP INDEX workflows_name;
CREATE UNIQUE INDEX workflows_name ON workflows(project_id, name COLLATE NOCASE);
CREATE INDEX sessions_project ON sessions(project_id, archived_at);
CREATE INDEX runs_project ON workflow_runs(project_id, status);
DELETE FROM settings WHERE key IN ('project_folder', 'sonata_master');

CREATE TRIGGER sessions_project_insert BEFORE INSERT ON sessions BEGIN
 SELECT CASE WHEN NEW.project_id != (SELECT project_id FROM bots WHERE id = NEW.bot_id)
 OR (NEW.opened_by IS NOT NULL AND NEW.project_id != (SELECT project_id FROM sessions WHERE id = NEW.opened_by))
 THEN RAISE(ABORT, 'cross-project session') END;
 SELECT CASE WHEN (SELECT archived_at FROM projects WHERE id = NEW.project_id) IS NOT NULL
 THEN RAISE(ABORT, 'project is archived') END;
END;
CREATE TRIGGER tasks_project_insert BEFORE INSERT ON tasks BEGIN
 SELECT CASE WHEN NEW.project_id != (SELECT project_id FROM bots WHERE id = NEW.bot_id)
 THEN RAISE(ABORT, 'cross-project schedule') END;
END;
CREATE TRIGGER tasks_project_update BEFORE UPDATE OF bot_id ON tasks BEGIN
 SELECT CASE WHEN NEW.project_id != (SELECT project_id FROM bots WHERE id = NEW.bot_id)
 THEN RAISE(ABORT, 'cross-project schedule') END;
END;
CREATE TRIGGER messages_project_insert BEFORE INSERT ON messages BEGIN
 SELECT CASE WHEN NEW.sender_session IS NOT NULL AND NEW.recipient_session IS NOT NULL
 AND (SELECT project_id FROM sessions WHERE id = NEW.sender_session) != (SELECT project_id FROM sessions WHERE id = NEW.recipient_session)
 THEN RAISE(ABORT, 'cross-project message') END;
 SELECT CASE WHEN NEW.status = 'queued' AND EXISTS (
 SELECT 1 FROM sessions s JOIN projects p ON p.id = s.project_id
 WHERE s.id IN (NEW.sender_session, NEW.recipient_session) AND p.archived_at IS NOT NULL)
 THEN RAISE(ABORT, 'project is archived') END;
END;
CREATE TRIGGER messages_project_retry BEFORE UPDATE OF status ON messages WHEN NEW.status = 'queued' BEGIN
 SELECT CASE WHEN EXISTS (SELECT 1 FROM sessions s JOIN projects p ON p.id = s.project_id
 WHERE s.id IN (NEW.sender_session, NEW.recipient_session) AND p.archived_at IS NOT NULL)
 THEN RAISE(ABORT, 'project is archived') END;
END;
"#)
}

fn project_from_row(r: &Row) -> rusqlite::Result<Project> {
    let provider: String = r.get(3)?;
    Ok(Project {
        id: r.get(0)?,
        name: r.get(1)?,
        default_directory: r.get(2)?,
        default_provider: Provider::parse(&provider).ok_or_else(|| bad_value("default_provider", &provider))?,
        master_id: r.get(4)?,
        archived_at: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

impl Store {
    pub fn scoped(&self, project_id: &str) -> Self {
        Self { conn: self.conn.clone(), project_id: Some(project_id.into()) }
    }
    pub fn unscoped(&self) -> Self {
        Self { conn: self.conn.clone(), project_id: None }
    }
    pub fn project_key(&self) -> &str {
        self.project_id.as_deref().unwrap_or(DEFAULT_PROJECT)
    }
    pub fn is_scoped(&self) -> bool {
        self.project_id.is_some()
    }
    // Keep a deleted Master's ID as a tombstone, matching the existing no-recreation policy.
    pub fn master_initialized(&self) -> Result<bool> {
        Ok(self.get_project(self.project_key())?.master_id.is_some())
    }
    pub fn includes(&self, project_id: &str) -> bool {
        self.project_id.as_deref().is_none_or(|p| p == project_id)
    }
    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM projects ORDER BY created_at, id"))?;
        let rows = stmt.query_map([], project_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
    pub fn get_project(&self, id: &str) -> Result<Project> {
        self.conn
            .lock()
            .unwrap()
            .query_row(&format!("SELECT {COLUMNS} FROM projects WHERE id = ?1"), [id], project_from_row)
            .optional()?
            .ok_or_else(|| CoreError::from(crate::ProjectError::NotFound))
    }
    pub fn require_active(&self) -> Result<()> {
        if self.get_project(self.project_key())?.archived_at.is_some() {
            return Err(CoreError::from(crate::ProjectError::Archived));
        }
        Ok(())
    }
    pub fn insert_project(&self, id: &str, input: &ProjectInput, master: &NewBot) -> Result<Project> {
        let t = now();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute("INSERT INTO projects (id, name, default_directory, default_provider, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)", params![id, input.name, input.default_directory, input.default_provider.as_str(), t])?;
        let bot = insert_bot_conn(&tx, master, crate::bundled::MASTER.handle, id)?;
        tx.execute("UPDATE projects SET master_id = ?2 WHERE id = ?1", params![id, bot.id])?;
        tx.commit()?;
        drop(conn);
        self.get_project(id)
    }
    pub fn update_project(&self, id: &str, input: &ProjectInput) -> Result<Project> {
        self.conn.lock().unwrap().execute("UPDATE projects SET name = ?2, default_directory = ?3, default_provider = ?4, updated_at = ?5 WHERE id = ?1", params![id, input.name, input.default_directory, input.default_provider.as_str(), now()])?;
        self.get_project(id)
    }
    pub fn legacy_workspace(&self) -> Result<bool> {
        Ok(self.conn.lock().unwrap().query_row(
            "SELECT legacy_workspace FROM projects WHERE id = ?1",
            [self.project_key()],
            |r| r.get(0),
        )?)
    }
    pub fn check_owner(&self, table: &str, id: &str) -> Result<()> {
        if let Some(project) = &self.project_id {
            let owner = self.entity_project(table, id)?;
            if owner.as_ref().is_some_and(|owner| owner != project) {
                return Err(CoreError::from(crate::ProjectError::Mismatch));
            }
        }
        Ok(())
    }
    pub(super) fn project_setting(&self, key: &str) -> Result<Option<String>> {
        let project = self.get_project(self.project_key())?;
        Ok(if key == "sonata_master" { project.master_id } else { project.default_directory })
    }
    pub(super) fn set_project_setting(&self, key: &str, value: Option<&str>) -> Result<()> {
        let column = if key == "sonata_master" { "master_id" } else { "default_directory" };
        self.conn.lock().unwrap().execute(
            &format!("UPDATE projects SET {column} = ?1, updated_at = ?3 WHERE id = ?2"),
            params![value, self.project_key(), now()],
        )?;
        Ok(())
    }
    // The DB mutex serializes this final check and archive write with queue admission.
    pub fn archive_project(&self, id: &str) -> Result<Project> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let busy: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM messages m JOIN sessions s ON s.id = m.recipient_session WHERE s.project_id = ?1 AND m.status = 'queued') OR EXISTS(SELECT 1 FROM workflow_runs WHERE project_id = ?1 AND status IN ('running', 'paused'))", [id], |r| r.get(0))?;
        if busy {
            return Err(CoreError::from(crate::ProjectError::QueuedWork));
        }
        tx.execute(
            "UPDATE projects SET archived_at = COALESCE(archived_at, ?2), updated_at = ?2 WHERE id = ?1",
            params![id, now()],
        )?;
        tx.commit()?;
        drop(conn);
        self.get_project(id)
    }
    pub fn restore_project(&self, id: &str, next: &[(String, String)]) -> Result<Project> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        for (task, at) in next {
            tx.execute(
                "UPDATE tasks SET next_run_at = ?2 WHERE id = ?1 AND project_id = ?3 AND enabled = 1",
                params![task, at, id],
            )?;
        }
        tx.execute("UPDATE projects SET archived_at = NULL, updated_at = ?2 WHERE id = ?1", params![id, now()])?;
        tx.commit()?;
        drop(conn);
        self.get_project(id)
    }
}

impl Store {
    pub fn entity_project(&self, table: &str, id: &str) -> Result<Option<String>> {
        let sql = match table {
            "messages" => "SELECT project_id FROM sessions WHERE id IN (SELECT sender_session FROM messages WHERE id = ?1 UNION SELECT recipient_session FROM messages WHERE id = ?1) LIMIT 1".to_string(),
            "threads" => "SELECT project_id FROM sessions WHERE id IN (SELECT participant_a FROM threads WHERE id = ?1 UNION SELECT participant_b FROM threads WHERE id = ?1) LIMIT 1".to_string(),
            "bots" | "sessions" | "tasks" | "workflows" | "workflow_runs" => format!("SELECT project_id FROM {table} WHERE id = ?1"),
            _ => return Err(CoreError::Internal("unknown project entity".into())),
        };
        Ok(self.conn.lock().unwrap().query_row(&sql, [id], |r| r.get(0)).optional()?)
    }
}

impl Store {
    pub fn skill_user_labels(&self, skill_id: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT CASE WHEN (SELECT COUNT(*) FROM projects) > 1 THEN b.handle || ' · ' || p.name ELSE b.handle END FROM bots b JOIN projects p ON p.id = b.project_id JOIN bot_skills bs ON bs.bot_id = b.id WHERE bs.skill_id = ?1 ORDER BY p.created_at, b.created_at, b.id")?;
        let labels = stmt.query_map([skill_id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(labels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        for migration in &MIGRATIONS[..12] {
            migration.apply(&conn).unwrap();
        }
        conn.pragma_update(None, "user_version", 12).unwrap();
        conn.execute_batch(r#"
INSERT INTO bots (id,handle,display_name,color,provider,instructions,created_at,updated_at) VALUES ('b','master','Master','#123456','codex','Custom instructions','old','old');
INSERT INTO sessions (id,bot_id,number,provider_session_id,session_config_version,created_at,last_activity_at,archived_at) VALUES ('s','b',1,'saved-provider-id',1,'old','old','completed');
INSERT INTO threads (id,participant_a,participant_b) VALUES ('t','s','user');
INSERT INTO messages (id,thread_id,sender,recipient,sender_session,kind,body,status,chain_id,hop,created_at) VALUES ('m','t','b','user','s','text','History','delivered','chain',0,'old');
INSERT INTO tasks (id,name,bot_id,prompt,every_days,at_time,starts_on,enabled,next_run_at,created_at,updated_at) VALUES ('task','Daily','b','Check',1,'09:00','2026-01-01',1,'2026-01-02T09:00:00Z','old','old');
INSERT INTO workflows VALUES ('w','Flow','','{"version":1,"start":null,"steps":[]}', 'old','old');
INSERT INTO workflow_runs (id,workflow_id,name,definition,request,status,created_at) VALUES ('run','w','Flow','{"version":1,"start":null,"steps":[]}','Request','finished','old');
INSERT INTO settings VALUES ('setup_done','true'),('project_folder','/unavailable/checkout'),('sonata_master','b');
"#).unwrap();
    }

    #[test]
    fn legacy_upgrade_keeps_history_ids_cli_resume_and_configuration_and_backs_up() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sonata.db");
        legacy(&path);
        let store = Store::open(&path).unwrap();
        let p = store.list_projects().unwrap().remove(0);
        assert_eq!(p.id, DEFAULT_PROJECT);
        assert_eq!(p.name, "Default");
        assert_eq!(p.default_provider, Provider::Codex);
        assert_eq!(p.default_directory.as_deref(), Some("/unavailable/checkout"));
        assert_eq!(p.master_id.as_deref(), Some("b"));
        let session = store.get_session("s").unwrap().unwrap();
        assert_eq!(session.project_id, DEFAULT_PROJECT);
        assert_eq!(session.provider_session_id.as_deref(), Some("saved-provider-id"));
        assert_eq!(session.archived_at.as_deref(), Some("completed"));
        assert_eq!(store.get_bot("b").unwrap().unwrap().instructions, "Custom instructions");
        assert_eq!(store.get_task("task").unwrap().unwrap().project_id, DEFAULT_PROJECT);
        assert_eq!(store.get_workflow("w").unwrap().unwrap().project_id, DEFAULT_PROJECT);
        assert_eq!(store.get_run("run").unwrap().unwrap().project_id, DEFAULT_PROJECT);
        assert_eq!(store.thread_messages("t").unwrap()[0].body, "History");
        let backup = Connection::open(tmp.path().join("sonata.db.projects.bak")).unwrap();
        assert_eq!(backup.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 12);
        assert_eq!(backup.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        drop(store);
        assert_eq!(Store::open(&path).unwrap().list_projects().unwrap().len(), 1);
    }

    #[test]
    fn failed_upgrade_rolls_back_the_schema_and_keeps_the_old_database_readable() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sonata.db");
        legacy(&path);
        Connection::open(&path).unwrap().execute("CREATE TABLE bots_project (unexpected TEXT)", []).unwrap();
        assert!(Store::open(&path).is_err());
        let conn = Connection::open(&path).unwrap();
        assert_eq!(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 12);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM bots", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='projects'", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row("SELECT value FROM settings WHERE key='sonata_master'", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "b"
        );
    }

    #[test]
    fn empty_install_and_legacy_without_a_master_use_claude_without_duplicates() {
        let store = Store::open_in_memory().unwrap();
        let project = store.get_project(DEFAULT_PROJECT).unwrap();
        assert_eq!(project.default_provider, Provider::Claude);
        assert_eq!(project.default_directory, None);
        assert_eq!(project.master_id, None);
    }

    #[test]
    fn legacy_deleted_master_stays_deleted_and_uses_the_fallback_provider() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sonata.db");
        legacy(&path);
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        conn.execute("DELETE FROM bots WHERE id = 'b'", []).unwrap();
        drop(conn);
        let store = Store::open(&path).unwrap();
        let project = store.get_project(DEFAULT_PROJECT).unwrap();
        assert_eq!(project.master_id.as_deref(), Some("b"));
        assert_eq!(project.default_provider, Provider::Claude);
        assert!(store.master_initialized().unwrap());
    }
}
