use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

mod orchestration;
mod projects;
mod work_items;
pub use projects::DEFAULT_PROJECT;

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::ids::{new_id, now};
use crate::model::*;
use crate::{CoreError, Result};

enum Migration {
    Sql(&'static str),
    Code(fn(&Connection) -> rusqlite::Result<()>),
}

impl Migration {
    fn apply(&self, conn: &Connection) -> rusqlite::Result<()> {
        match self {
            Migration::Sql(sql) => conn.execute_batch(sql),
            Migration::Code(step) => step(conn),
        }
    }
}

const MIGRATIONS: &[Migration] = &[
    Migration::Sql(
        r#"
CREATE TABLE bots (
  id TEXT PRIMARY KEY,
  handle TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL,
  color TEXT NOT NULL,
  provider TEXT NOT NULL,
  model TEXT,
  instructions TEXT NOT NULL,
  provider_session_id TEXT,
  config_version INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE threads (
  id TEXT PRIMARY KEY,
  participant_a TEXT NOT NULL,
  participant_b TEXT NOT NULL,
  UNIQUE (participant_a, participant_b)
);
CREATE TABLE messages (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  id TEXT NOT NULL UNIQUE,
  thread_id TEXT NOT NULL REFERENCES threads(id),
  sender TEXT NOT NULL,
  recipient TEXT NOT NULL,
  kind TEXT NOT NULL,
  body TEXT NOT NULL,
  system_code TEXT,
  system_params TEXT,
  attachments TEXT NOT NULL DEFAULT '[]',
  status TEXT NOT NULL,
  chain_id TEXT NOT NULL,
  hop INTEGER NOT NULL,
  in_reply_to TEXT,
  created_at TEXT NOT NULL,
  delivered_at TEXT
);
CREATE INDEX messages_recipient_status ON messages(recipient, status, seq);
CREATE INDEX messages_thread ON messages(thread_id, seq);
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
"#,
    ),
    Migration::Sql("ALTER TABLE messages ADD COLUMN turn_ended_at TEXT;"),
    Migration::Sql(
        "ALTER TABLE bots ADD COLUMN session_config_version INTEGER;
     UPDATE bots SET session_config_version = config_version WHERE provider_session_id IS NOT NULL;",
    ),
    Migration::Sql(
        "CREATE TABLE skills (
  id TEXT PRIMARY KEY,
  slug TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  description TEXT NOT NULL,
  source TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE bot_skills (
  bot_id TEXT NOT NULL REFERENCES bots(id) ON DELETE CASCADE,
  skill_id TEXT NOT NULL REFERENCES skills(id),
  PRIMARY KEY (bot_id, skill_id)
);",
    ),
    Migration::Code(sessions_from_bots),
    Migration::Sql("ALTER TABLE sessions ADD COLUMN archived_at TEXT;"),
    Migration::Sql(
        r#"
ALTER TABLE sessions ADD COLUMN waiting_question_id TEXT;
ALTER TABLE sessions ADD COLUMN waiting_since TEXT;
ALTER TABLE sessions ADD COLUMN open_request_id TEXT;
ALTER TABLE messages ADD COLUMN answered_by TEXT;
CREATE TABLE creations (
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  key TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  result_id TEXT NOT NULL,
  result_name TEXT NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (session_id, kind, key)
);
"#,
    ),
    Migration::Sql(
        r#"
CREATE TABLE tasks (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  bot_id TEXT NOT NULL REFERENCES bots(id) ON DELETE CASCADE,
  prompt TEXT NOT NULL,
  every_days INTEGER NOT NULL,
  at_time TEXT NOT NULL,
  starts_on TEXT NOT NULL,
  enabled INTEGER NOT NULL,
  next_run_at TEXT,
  last_run_at TEXT,
  last_run_session TEXT REFERENCES sessions(id) ON DELETE SET NULL,
  skipped_at TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX tasks_name ON tasks(name COLLATE NOCASE);
ALTER TABLE messages ADD COLUMN task_id TEXT;
ALTER TABLE messages ADD COLUMN task_name TEXT;
ALTER TABLE messages ADD COLUMN task_since TEXT;
ALTER TABLE messages ADD COLUMN task_done_at TEXT;
"#,
    ),
    Migration::Sql(
        r#"
ALTER TABLE sessions ADD COLUMN seen_at TEXT;
UPDATE sessions SET seen_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now');
CREATE INDEX messages_sender_session ON messages(sender_session, seq);
"#,
    ),
    Migration::Sql("ALTER TABLE sessions ADD COLUMN waiting_options TEXT;"),
    Migration::Sql(
        r#"
CREATE TABLE workflows (id TEXT PRIMARY KEY, name TEXT NOT NULL, when_to_use TEXT NOT NULL DEFAULT '',
  definition TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE UNIQUE INDEX workflows_name ON workflows(name COLLATE NOCASE);
CREATE TABLE workflow_runs (id TEXT PRIMARY KEY,
  workflow_id TEXT REFERENCES workflows(id) ON DELETE SET NULL,
  name TEXT NOT NULL, definition TEXT NOT NULL, request TEXT NOT NULL, request_files TEXT NOT NULL DEFAULT '[]',
  opener_session TEXT REFERENCES sessions(id) ON DELETE SET NULL,
  root_session TEXT REFERENCES sessions(id) ON DELETE SET NULL,
  status TEXT NOT NULL, pause TEXT, extra_visits TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL, ended_at TEXT);
CREATE TABLE workflow_run_sessions (run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
  bot_id TEXT NOT NULL, session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  seen_ord INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (run_id, bot_id));
CREATE TABLE workflow_visits (id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
  ord INTEGER NOT NULL, step_id TEXT NOT NULL, n INTEGER NOT NULL,
  session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
  request_message TEXT NOT NULL, result_message TEXT, outcome TEXT, reminded INTEGER NOT NULL DEFAULT 0,
  started_at TEXT NOT NULL, ended_at TEXT, UNIQUE (run_id, ord));
ALTER TABLE messages ADD COLUMN workflow_mark TEXT;
"#,
    ),
    Migration::Sql("ALTER TABLE messages ADD COLUMN workflow_request INTEGER NOT NULL DEFAULT 0;"),
    Migration::Code(projects::migrate),
    Migration::Sql("INSERT INTO settings (key, value) VALUES ('onboarding_pending', CASE WHEN EXISTS (SELECT 1 FROM settings WHERE key = 'setup_done' AND value = 'true') THEN 'false' ELSE 'true' END);"),
    Migration::Sql(orchestration::SCHEMA),
    Migration::Code(work_items::migrate),
];

fn sessions_from_bots(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  bot_id TEXT NOT NULL REFERENCES bots(id) ON DELETE CASCADE,
  number INTEGER NOT NULL,
  title TEXT,
  provider_session_id TEXT,
  session_config_version INTEGER,
  opened_by TEXT REFERENCES sessions(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL,
  last_activity_at TEXT NOT NULL,
  UNIQUE (bot_id, number)
);
ALTER TABLE messages ADD COLUMN sender_session TEXT;
ALTER TABLE messages ADD COLUMN recipient_session TEXT;
CREATE INDEX messages_recipient_session ON messages(recipient_session, status, seq);",
    )?;
    let bots: Vec<String> = conn
        .prepare("SELECT id FROM bots ORDER BY created_at, id")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut session_of = std::collections::HashMap::new();
    for bot_id in bots {
        let session_id = new_id();
        conn.execute(
            "INSERT INTO sessions (id, bot_id, number, title, provider_session_id, session_config_version, opened_by, \
             created_at, last_activity_at) \
             SELECT ?1, b.id, 1, NULL, b.provider_session_id, b.session_config_version, NULL, b.created_at, \
             COALESCE((SELECT MAX(m.created_at) FROM messages m WHERE m.sender = b.id OR m.recipient = b.id), \
             b.updated_at) FROM bots b WHERE b.id = ?2",
            params![session_id, bot_id],
        )?;
        session_of.insert(bot_id, session_id);
    }
    conn.execute_batch(
        "UPDATE messages SET sender_session = (SELECT s.id FROM sessions s WHERE s.bot_id = messages.sender);
         UPDATE messages SET recipient_session = (SELECT s.id FROM sessions s WHERE s.bot_id = messages.recipient);",
    )?;
    let threads: Vec<(String, String, String)> = conn
        .prepare("SELECT id, participant_a, participant_b FROM threads")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, a, b) in threads {
        let (new_a, new_b) = (session_of.get(&a).unwrap_or(&a), session_of.get(&b).unwrap_or(&b));
        if (new_a, new_b) == (&a, &b) {
            continue;
        }
        let (new_a, new_b) = if new_a <= new_b { (new_a, new_b) } else { (new_b, new_a) };
        conn.execute(
            "UPDATE threads SET participant_a = ?2, participant_b = ?3 WHERE id = ?1",
            params![id, new_a, new_b],
        )?;
    }
    conn.execute_batch(
        "ALTER TABLE bots DROP COLUMN provider_session_id;
         ALTER TABLE bots DROP COLUMN session_config_version;",
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActivityRows {
    pub in_flight: Option<Message>,
    pub queued: u32,
    pub question: Option<Message>,
    pub last_sent: Option<Message>,
    pub last_to: Option<Message>,
    pub last_reply: Option<Message>,
    pub last_for_user_at: Option<String>,
    pub run_of: Option<String>,
}

const LAST_SENT: &str = "sender_session = ?1 AND kind = 'text' AND status != 'failed'";
const LAST_REPLY: &str = "sender_session = ?1 AND recipient = 'user' AND kind != 'system'";
const LAST_FOR_USER_AT: &str = "SELECT MAX(created_at) FROM messages WHERE sender_session = ?1 AND recipient = 'user'";

fn latest_where(filter: &str) -> String {
    format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE {filter} ORDER BY seq DESC LIMIT 1")
}

const BOT_COLUMNS: &str =
    "id, handle, display_name, color, provider, model, instructions, config_version, created_at, updated_at, project_id";
const SESSION_COLUMNS: &str = "id, bot_id, number, title, provider_session_id, session_config_version, opened_by, \
     created_at, last_activity_at, archived_at, waiting_question_id, waiting_since, open_request_id, seen_at, \
     waiting_options, project_id";
const MESSAGE_COLUMNS: &str = "id, thread_id, sender, recipient, kind, body, system_code, \
     system_params, status, chain_id, hop, in_reply_to, created_at, delivered_at, attachments, \
     sender_session, recipient_session, task_id, task_name, task_since, workflow_mark, workflow_request";
const MESSAGE_INSERT_COLUMNS: &str = "id, thread_id, sender, recipient, kind, body, system_code, \
     system_params, status, chain_id, hop, in_reply_to, created_at, delivered_at, attachments, \
     sender_session, recipient_session";
const TASK_COLUMNS: &str = "id, name, bot_id, prompt, every_days, at_time, starts_on, enabled, next_run_at, \
     last_run_at, last_run_session, skipped_at, created_at, updated_at, project_id";
const WORKFLOW_COLUMNS: &str = "id, name, when_to_use, definition, created_at, updated_at, project_id";
const RUN_COLUMNS: &str = "id, workflow_id, name, definition, request, request_files, opener_session, root_session, \
     status, pause, extra_visits, created_at, ended_at, project_id";
const VISIT_COLUMNS: &str = "id, run_id, ord, step_id, n, session_id, request_message, result_message, outcome, \
     reminded, started_at, ended_at";
const SKILL_COLUMNS: &str = "id, slug, name, description, source, created_at, updated_at";

#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    pub message: Message,
    pub notice: Option<Message>,
    pub session: Session,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreationKind {
    Skill,
    Bot,
    Task,
    Workflow,
}

impl CreationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CreationKind::Skill => "skill",
            CreationKind::Bot => "bot",
            CreationKind::Task => "task",
            CreationKind::Workflow => "workflow",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NewCreation<'a> {
    pub session_id: &'a str,
    pub kind: CreationKind,
    pub key: &'a str,
    pub fingerprint: &'a str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Creation {
    pub fingerprint: String,
    pub result_id: String,
    pub result_name: String,
}

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
    project_id: Option<String>,
}

fn bad_value(column: &str, value: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidColumnName(format!("unexpected value {value:?} in column {column}"))
}

fn bot_from_row(r: &Row) -> rusqlite::Result<Bot> {
    let provider: String = r.get(4)?;
    Ok(Bot {
        project_id: r.get(10)?,
        id: r.get(0)?,
        handle: r.get(1)?,
        display_name: r.get(2)?,
        color: r.get(3)?,
        provider: Provider::parse(&provider).ok_or_else(|| bad_value("provider", &provider))?,
        model: r.get(5)?,
        instructions: r.get(6)?,
        config_version: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
        skill_ids: Vec::new(),
    })
}

fn session_from_row(r: &Row) -> rusqlite::Result<Session> {
    Ok(Session {
        project_id: r.get(15)?,
        id: r.get(0)?,
        bot_id: r.get(1)?,
        number: r.get(2)?,
        title: r.get(3)?,
        provider_session_id: r.get(4)?,
        session_config_version: r.get(5)?,
        opened_by: r.get(6)?,
        created_at: r.get(7)?,
        last_activity_at: r.get(8)?,
        archived_at: r.get(9)?,
        waiting_question_id: r.get(10)?,
        waiting_since: r.get(11)?,
        open_request_id: r.get(12)?,
        seen_at: r.get(13)?,
        waiting_options: r
            .get::<_, Option<String>>(14)?
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default(),
    })
}

fn skill_from_row(r: &Row) -> rusqlite::Result<Skill> {
    let source: String = r.get(4)?;
    Ok(Skill {
        id: r.get(0)?,
        slug: r.get(1)?,
        name: r.get(2)?,
        description: r.get(3)?,
        source: SkillSource::parse(&source).ok_or_else(|| bad_value("source", &source))?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

fn skill_ids_conn(conn: &Connection, bot_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT s.id FROM bot_skills b JOIN skills s ON s.id = b.skill_id WHERE b.bot_id = ?1 ORDER BY s.slug",
    )?;
    let ids = stmt.query_map(params![bot_id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(ids)
}

fn with_skills(conn: &Connection, mut bot: Bot) -> rusqlite::Result<Bot> {
    bot.skill_ids = skill_ids_conn(conn, &bot.id)?;
    Ok(bot)
}

fn get_bot_conn(conn: &Connection, id: &str) -> rusqlite::Result<Option<Bot>> {
    conn.query_row(&format!("SELECT {BOT_COLUMNS} FROM bots WHERE id = ?1"), params![id], bot_from_row)
        .optional()?
        .map(|bot| with_skills(conn, bot))
        .transpose()
}

fn set_bot_skills_conn(conn: &Connection, bot_id: &str, skill_ids: &[String]) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM bot_skills WHERE bot_id = ?1", params![bot_id])?;
    for skill_id in skill_ids {
        conn.execute("INSERT OR IGNORE INTO bot_skills (bot_id, skill_id) VALUES (?1, ?2)", params![bot_id, skill_id])?;
    }
    Ok(())
}

fn bots_using_conn(conn: &Connection, skill_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT b.handle FROM bot_skills bs JOIN bots b ON b.id = bs.bot_id \
         WHERE bs.skill_id = ?1 ORDER BY b.created_at, b.id",
    )?;
    let handles = stmt.query_map(params![skill_id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(handles)
}

fn id_set(ids: &[String]) -> Vec<String> {
    let mut set = ids.to_vec();
    set.sort();
    set.dedup();
    set
}

fn message_from_row(r: &Row) -> rusqlite::Result<Message> {
    let kind: String = r.get(4)?;
    let status: String = r.get(8)?;
    let params: Option<String> = r.get(7)?;
    let attachments: String = r.get(14)?;
    Ok(Message {
        id: r.get(0)?,
        thread_id: r.get(1)?,
        sender: r.get(2)?,
        recipient: r.get(3)?,
        sender_session: r.get(15)?,
        recipient_session: r.get(16)?,
        kind: MessageKind::parse(&kind).ok_or_else(|| bad_value("kind", &kind))?,
        body: r.get(5)?,
        system_code: r.get(6)?,
        system_params: params
            .map(|p| serde_json::from_str(&p).map_err(|_| bad_value("system_params", &p)))
            .transpose()?,
        status: MessageStatus::parse(&status).ok_or_else(|| bad_value("status", &status))?,
        chain_id: r.get(9)?,
        hop: r.get(10)?,
        in_reply_to: r.get(11)?,
        created_at: r.get(12)?,
        delivered_at: r.get(13)?,
        task_id: r.get(17)?,
        task_name: r.get(18)?,
        task_since: r.get(19)?,
        workflow_mark: r
            .get::<_, Option<String>>(20)?
            .map(|m| serde_json::from_str(&m).map_err(|_| bad_value("workflow_mark", &m)))
            .transpose()?,
        workflow_request: r.get(21)?,
        attachments: serde_json::from_str(&attachments).map_err(|_| bad_value("attachments", &attachments))?,
    })
}

fn task_from_row(r: &Row) -> rusqlite::Result<Task> {
    Ok(Task {
        project_id: r.get(14)?,
        id: r.get(0)?,
        name: r.get(1)?,
        bot_id: r.get(2)?,
        prompt: r.get(3)?,
        every_days: r.get(4)?,
        at: r.get(5)?,
        starts_on: r.get(6)?,
        enabled: r.get(7)?,
        next_run_at: r.get(8)?,
        last_run_at: r.get(9)?,
        last_run_session: r.get(10)?,
        skipped_at: r.get(11)?,
        created_at: r.get(12)?,
        updated_at: r.get(13)?,
    })
}

fn workflow_from_row(r: &Row) -> rusqlite::Result<Workflow> {
    let definition: String = r.get(3)?;
    Ok(Workflow {
        project_id: r.get(6)?,
        id: r.get(0)?,
        name: r.get(1)?,
        when_to_use: r.get(2)?,
        def: serde_json::from_str(&definition).map_err(|_| bad_value("definition", &definition))?,
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
    })
}

fn get_workflow_conn(conn: &Connection, id: &str) -> rusqlite::Result<Option<Workflow>> {
    conn.query_row(&format!("SELECT {WORKFLOW_COLUMNS} FROM workflows WHERE id = ?1"), params![id], workflow_from_row)
        .optional()
}

fn insert_workflow_conn(conn: &Connection, new: &NewWorkflow, project_id: &str) -> Result<Workflow> {
    let id = new_id();
    let at = now();
    let definition = serde_json::to_string(&new.def).map_err(|e| CoreError::Internal(e.to_string()))?;
    conn.execute(
        &format!("INSERT INTO workflows ({WORKFLOW_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)"),
        params![id, new.name, new.when_to_use, definition, at, project_id],
    )?;
    Ok(get_workflow_conn(conn, &id)?.expect("workflow just inserted"))
}

fn from_json<T: serde::de::DeserializeOwned>(column: &str, text: &str) -> rusqlite::Result<T> {
    serde_json::from_str(text).map_err(|_| bad_value(column, text))
}

fn run_from_row(r: &Row) -> rusqlite::Result<WorkflowRun> {
    let status: String = r.get(8)?;
    Ok(WorkflowRun {
        project_id: r.get(13)?,
        id: r.get(0)?,
        workflow_id: r.get(1)?,
        name: r.get(2)?,
        def: from_json("definition", &r.get::<_, String>(3)?)?,
        request: r.get(4)?,
        request_files: from_json("request_files", &r.get::<_, String>(5)?)?,
        opener_session: r.get(6)?,
        root_session: r.get(7)?,
        status: RunStatus::parse(&status).ok_or_else(|| bad_value("status", &status))?,
        pause: r.get::<_, Option<String>>(9)?.map(|p| from_json("pause", &p)).transpose()?,
        extra_visits: from_json("extra_visits", &r.get::<_, String>(10)?)?,
        created_at: r.get(11)?,
        ended_at: r.get(12)?,
    })
}

fn visit_from_row(r: &Row) -> rusqlite::Result<Visit> {
    Ok(Visit {
        id: r.get(0)?,
        run_id: r.get(1)?,
        ord: r.get(2)?,
        step_id: r.get(3)?,
        n: r.get(4)?,
        session_id: r.get(5)?,
        request_message: r.get(6)?,
        result_message: r.get(7)?,
        outcome: r.get(8)?,
        reminded: r.get(9)?,
        started_at: r.get(10)?,
        ended_at: r.get(11)?,
    })
}

fn get_run_conn(conn: &Connection, id: &str) -> rusqlite::Result<Option<WorkflowRun>> {
    conn.query_row(&format!("SELECT {RUN_COLUMNS} FROM workflow_runs WHERE id = ?1"), params![id], run_from_row)
        .optional()
}

fn get_visit_conn(conn: &Connection, id: &str) -> rusqlite::Result<Option<Visit>> {
    conn.query_row(&format!("SELECT {VISIT_COLUMNS} FROM workflow_visits WHERE id = ?1"), params![id], visit_from_row)
        .optional()
}

fn visit_delivered_conn(conn: &Connection, visit: &Visit) -> rusqlite::Result<bool> {
    let Some(session) = &visit.session_id else { return Ok(false) };
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM messages WHERE recipient_session = ?1 AND delivered_at IS NOT NULL \
         AND json_extract(workflow_mark, '$.visit_id') = ?2 AND json_extract(workflow_mark, '$.role') = 'request')",
        params![session, visit.id],
        |r| r.get(0),
    )
}

fn open_visit_conn(conn: &Connection, run_id: &str, ord: i64, open: &VisitOpen) -> Result<Routed> {
    let (session, created) = match &open.session {
        Some(id) => (open_session_conn(conn, id)?, false),
        None => {
            let session = create_session_conn(conn, &open.bot_id, Some(&open.title), open.opened_by.as_deref())?;
            conn.execute(
                "INSERT INTO workflow_run_sessions (run_id, bot_id, session_id) VALUES (?1, ?2, ?3)",
                params![run_id, open.bot_id, session.id],
            )?;
            conn.execute(
                "UPDATE workflow_runs SET root_session = ?2 WHERE id = ?1 AND root_session IS NULL",
                params![run_id, session.id],
            )?;
            (session, true)
        }
    };
    conn.execute(
        "UPDATE workflow_run_sessions SET seen_ord = ?3 WHERE run_id = ?1 AND bot_id = ?2",
        params![run_id, open.bot_id, open.seen_ord],
    )?;
    conn.execute(
        &format!(
            "INSERT INTO workflow_visits ({VISIT_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, 0, ?8, NULL)"
        ),
        params![open.visit_id, run_id, ord, open.step_id, open.n, session.id, open.message_id, now()],
    )?;
    let root: Option<String> =
        conn.query_row("SELECT root_session FROM workflow_runs WHERE id=?1", [run_id], |r| r.get(0))?;
    if let Some(root) = root {
        if let Some(item) = work_items::owner(conn, &root)? {
            conn.execute("INSERT OR REPLACE INTO work_item_sessions VALUES (?1,?2)", params![session.id, item])?;
            conn.execute(
                "UPDATE work_items SET workflow_run_id=?2 WHERE id=?1 AND primary_session_id=?3",
                params![item, run_id, root],
            )?;
        }
    }
    let message = NewMessage { recipient_session: Some(session.id.clone()), ..open.message.clone() };
    let message = insert_marked_conn(conn, &open.message_id, &message, &open.mark)?;
    let session = get_session_conn(conn, &session.id)?.expect("session just used");
    Ok(Routed { message, session, created })
}

fn insert_marked_conn(conn: &Connection, id: &str, m: &NewMessage, mark: &WorkflowMark) -> Result<Message> {
    store_message_conn(conn, id, m, false)?;
    conn.execute("UPDATE messages SET workflow_mark = ?2 WHERE id = ?1", params![id, serde_json::to_string(mark)?])?;
    Ok(get_message_conn(conn, id)?.expect("message just inserted"))
}

fn run_then_conn(
    conn: &Connection,
    run_id: &str,
    then: &RunThen,
    at: &str,
) -> Result<(Option<Routed>, Option<Message>)> {
    Ok(match then {
        RunThen::Visit(open) => {
            let ord: i64 = conn.query_row(
                "SELECT COALESCE(MAX(ord), 0) + 1 FROM workflow_visits WHERE run_id = ?1",
                params![run_id],
                |r| r.get(0),
            )?;
            (Some(open_visit_conn(conn, run_id, ord, open)?), None)
        }
        RunThen::Pause(reason) => {
            conn.execute(
                "UPDATE workflow_runs SET status = ?2, pause = ?3 WHERE id = ?1",
                params![run_id, RunStatus::Paused.as_str(), serde_json::to_string(reason)?],
            )?;
            (None, None)
        }
        RunThen::Finish { to_opener, notice } => {
            conn.execute(
                "UPDATE workflow_runs SET status = ?2, pause = NULL, ended_at = ?3 WHERE id = ?1",
                params![run_id, RunStatus::Finished.as_str(), at],
            )?;
            let finished = match (to_opener, notice) {
                (Some((id, message, mark)), _) => Some(insert_marked_conn(conn, id, message, mark)?),
                (None, Some(notice)) => Some(insert_message_conn(conn, &new_id(), notice)?),
                (None, None) => None,
            };
            orchestration::workflow_finished_conn(conn, run_id, finished.as_ref())?;
            if let Some(result) = &finished {
                let primary: Option<String> = conn
                    .query_row("SELECT primary_session_id FROM work_items WHERE workflow_run_id=?1", [run_id], |r| {
                        r.get(0)
                    })
                    .optional()?
                    .flatten();
                if let Some(primary) = primary {
                    let coordinated: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM work_items WHERE workflow_run_id=?1 AND orchestration_id IS NOT NULL)", [run_id], |r| r.get(0))?;
                    if !coordinated {
                        work_items::complete(conn, &primary, result)?;
                    }
                }
            }
            let finished = finished.map(|m| get_message_conn(conn, &m.id)).transpose()?.flatten();
            (None, finished)
        }
        RunThen::Remind { id, message, mark } => {
            let message = insert_marked_conn(conn, id, message, mark)?;
            let session = message
                .recipient_session
                .as_deref()
                .map(|session| get_session_conn(conn, session))
                .transpose()?
                .flatten()
                .ok_or_else(|| CoreError::Internal("a reminder names the session it goes to".into()))?;
            (Some(Routed { message, session, created: false }), None)
        }
    })
}

fn stop_run_conn(conn: &Connection, run_id: &str, at: &str) -> Result<Option<WorkflowRun>> {
    let changed = conn.execute(
        "UPDATE workflow_runs SET status = ?2, pause = NULL, ended_at = ?3 WHERE id = ?1 AND status IN (?4, ?5)",
        params![run_id, RunStatus::Stopped.as_str(), at, RunStatus::Running.as_str(), RunStatus::Paused.as_str()],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    conn.execute(
        "UPDATE workflow_visits SET ended_at = ?2 WHERE run_id = ?1 AND ended_at IS NULL",
        params![run_id, at],
    )?;
    orchestration::workflow_stopped_conn(conn, run_id)?;
    Ok(get_run_conn(conn, run_id)?)
}

fn get_task_conn(conn: &Connection, id: &str) -> rusqlite::Result<Option<Task>> {
    conn.query_row(&format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1"), params![id], task_from_row).optional()
}

fn insert_task_conn(conn: &Connection, row: &TaskRow) -> rusqlite::Result<Task> {
    let id = new_id();
    let t = now();
    conn.execute(
        &format!("INSERT INTO tasks ({TASK_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, NULL, NULL, ?10, ?10, (SELECT project_id FROM bots WHERE id = ?3))"),
        params![
            id,
            row.name,
            row.bot_id,
            row.prompt,
            row.every_days,
            row.at,
            row.starts_on,
            row.enabled,
            row.next_run_at,
            t
        ],
    )?;
    Ok(get_task_conn(conn, &id)?.expect("task just inserted"))
}

fn thread_for_conn(conn: &Connection, a: &str, b: &str) -> rusqlite::Result<String> {
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    if let Some(id) = conn
        .query_row("SELECT id FROM threads WHERE participant_a = ?1 AND participant_b = ?2", params![a, b], |r| {
            r.get(0)
        })
        .optional()?
    {
        return Ok(id);
    }
    let id = new_id();
    conn.execute("INSERT INTO threads (id, participant_a, participant_b) VALUES (?1, ?2, ?3)", params![id, a, b])?;
    Ok(id)
}

fn get_message_conn(conn: &Connection, id: &str) -> rusqlite::Result<Option<Message>> {
    conn.query_row(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?1"), params![id], message_from_row)
        .optional()
}

fn get_session_conn(conn: &Connection, id: &str) -> rusqlite::Result<Option<Session>> {
    conn.query_row(&format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id = ?1"), params![id], session_from_row)
        .optional()
}

fn sessions_of_conn(conn: &Connection, bot_id: &str) -> rusqlite::Result<Vec<Session>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SESSION_COLUMNS} FROM sessions WHERE bot_id = ?1 ORDER BY last_activity_at DESC, number DESC"
    ))?;
    let sessions = stmt.query_map(params![bot_id], session_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(sessions)
}

fn create_session_conn(
    conn: &Connection,
    bot_id: &str,
    title: Option<&str>,
    opened_by: Option<&str>,
) -> rusqlite::Result<Session> {
    let id = new_id();
    let t = now();
    conn.execute(
        "INSERT INTO sessions (id, bot_id, number, title, provider_session_id, session_config_version, opened_by, \
         created_at, last_activity_at, project_id) \
         SELECT ?1, ?2, COALESCE(MAX(number), 0) + 1, ?3, NULL, NULL, ?4, ?5, ?5, (SELECT project_id FROM bots WHERE id = ?2) FROM sessions WHERE bot_id = ?2",
        params![id, bot_id, title, opened_by, t],
    )?;
    let session = get_session_conn(conn, &id)?.expect("session just inserted");
    work_items::attach_session(conn, &session)?;
    Ok(session)
}

fn shared_session_conn(conn: &Connection, bot_id: &str, with: &str) -> rusqlite::Result<Option<Session>> {
    conn.query_row(
        &format!(
            "SELECT {SESSION_COLUMNS} FROM sessions WHERE bot_id = ?1 AND (opened_by = ?2 OR id IN ( \
             SELECT CASE WHEN t.participant_a = ?2 THEN t.participant_b ELSE t.participant_a END FROM threads t \
             WHERE (t.participant_a = ?2 OR t.participant_b = ?2) \
             AND EXISTS (SELECT 1 FROM messages m WHERE m.thread_id = t.id))) AND archived_at IS NULL \
             ORDER BY last_activity_at DESC, number DESC LIMIT 1"
        ),
        params![bot_id, with],
        session_from_row,
    )
    .optional()
}

fn bot_exists_conn(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    Ok(conn.query_row("SELECT 1 FROM bots WHERE id = ?1", params![id], |_| Ok(())).optional()?.is_some())
}

fn open_session_conn(conn: &Connection, id: &str) -> Result<Session> {
    match get_session_conn(conn, id)? {
        None => Err(crate::Refusal::SessionNotFound(id.into()).into()),
        Some(session) if session.archived_at.is_some() => Err(crate::Refusal::SessionArchived(id.into()).into()),
        Some(session) => Ok(session),
    }
}

fn insert_message_conn(conn: &Connection, id: &str, m: &NewMessage) -> Result<Message> {
    store_message_conn(conn, id, m, m.system_code.as_deref() != Some("orchestration_event"))
}

fn store_message_conn(conn: &Connection, id: &str, m: &NewMessage, answers: bool) -> Result<Message> {
    for (end, party, session) in
        [("sender", &m.sender, &m.sender_session), ("recipient", &m.recipient, &m.recipient_session)]
    {
        match session {
            None if party != USER && bot_exists_conn(conn, party)? => {
                return Err(CoreError::Internal(format!(
                    "the message's {end} is bot {party}, but it names no session"
                )));
            }
            Some(session) => {
                open_session_conn(conn, session)?;
            }
            None => {}
        }
    }
    let thread_id = thread_for_conn(
        conn,
        m.sender_session.as_deref().unwrap_or(&m.sender),
        m.recipient_session.as_deref().unwrap_or(&m.recipient),
    )?;
    let created_at = now();
    let delivered_at = (m.status == MessageStatus::Delivered).then(|| created_at.clone());
    let params_json = m.system_params.as_ref().map(serde_json::to_string).transpose()?;
    let attachments_json = serde_json::to_string(&m.attachments)?;
    conn.execute(
        &format!(
            "INSERT INTO messages ({MESSAGE_INSERT_COLUMNS}) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)"
        ),
        params![
            id,
            thread_id,
            m.sender,
            m.recipient,
            m.kind.as_str(),
            m.body,
            m.system_code,
            params_json,
            m.status.as_str(),
            m.chain_id,
            m.hop,
            m.in_reply_to,
            created_at,
            delivered_at,
            attachments_json,
            m.sender_session,
            m.recipient_session
        ],
    )?;
    if m.status != MessageStatus::Failed {
        let ends = [
            m.recipient_session.as_ref(),
            (m.kind != MessageKind::System).then_some(m.sender_session.as_ref()).flatten(),
        ];
        for session in ends.into_iter().flatten() {
            conn.execute("UPDATE sessions SET last_activity_at = ?2 WHERE id = ?1", params![session, created_at])?;
        }
    }
    if m.sender == USER && m.system_code.is_none() && answers && m.status == MessageStatus::Queued {
        if let Some(session) = &m.recipient_session {
            work_items::reopen(conn, session, id)?;
            if let Some(item) = work_items::owner(conn, session)? {
                conn.execute(
                    "UPDATE work_items SET instructions=?2 WHERE id=?1 AND instructions=''",
                    params![item, m.body],
                )?;
                conn.execute("UPDATE work_items SET state='working', finished_at=NULL, outcome=NULL, result_message_id=NULL WHERE id=?1 AND orchestration_id IS NULL", [&item])?;
            }
        }
    }
    if let Some(session) = m.recipient_session.as_ref().filter(|_| answers) {
        if m.kind != MessageKind::System && m.status != MessageStatus::Failed {
            conn.execute(
                "UPDATE sessions SET waiting_question_id = NULL, waiting_since = NULL, waiting_options = NULL \
                 WHERE id = ?1 AND waiting_question_id IS NOT NULL AND (?2 = ?5 OR ?4 IS NULL AND ?3 = \
                   (SELECT r.sender_session FROM messages r WHERE r.id = sessions.open_request_id))",
                params![session, m.sender, m.sender_session, m.in_reply_to, USER],
            )?;
        }
    }
    Ok(get_message_conn(conn, id)?.expect("message just inserted"))
}

fn insert_bot_conn(tx: &Connection, new: &NewBot, handle: &str, project_id: &str) -> Result<Bot> {
    let id = new_id();
    let t = now();
    tx.execute(
        &format!("INSERT INTO bots ({BOT_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"),
        params![
            id,
            handle,
            new.display_name,
            new.color,
            new.provider.as_str(),
            new.model,
            new.instructions,
            1_i64,
            t,
            t,
            project_id
        ],
    )?;
    set_bot_skills_conn(tx, &id, &new.skill_ids)?;
    Ok(get_bot_conn(tx, &id)?.expect("bot just inserted"))
}

fn insert_skill_conn(
    conn: &Connection,
    slug: &str,
    name: &str,
    description: &str,
    source: SkillSource,
) -> Result<Skill> {
    let t = now();
    let skill = Skill {
        id: new_id(),
        slug: slug.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        source,
        created_at: t.clone(),
        updated_at: t,
    };
    conn.execute(
        &format!("INSERT INTO skills ({SKILL_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"),
        params![
            skill.id,
            skill.slug,
            skill.name,
            skill.description,
            skill.source.as_str(),
            skill.created_at,
            skill.updated_at
        ],
    )?;
    Ok(skill)
}

fn set_setting_conn(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn record_creation_conn(
    conn: &Connection,
    record: &NewCreation,
    result_id: &str,
    result_name: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO creations (session_id, kind, key, fingerprint, result_id, result_name, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![record.session_id, record.kind.as_str(), record.key, record.fingerprint, result_id, result_name, now()],
    )?;
    Ok(())
}

fn tree_conn(conn: &Connection, id: &str) -> rusqlite::Result<Vec<Session>> {
    let found: Vec<Session> = conn
        .prepare(&format!(
            "WITH RECURSIVE tree(id) AS (SELECT id FROM sessions WHERE id = ?1 \
             UNION SELECT s.id FROM sessions s JOIN tree t ON s.opened_by = t.id) \
             SELECT {SESSION_COLUMNS} FROM sessions WHERE id IN (SELECT id FROM tree) ORDER BY created_at, id"
        ))?
        .query_map(params![id], session_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(openers_first(id, found))
}

fn placeholders(n: usize) -> String {
    (1..=n).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ")
}

fn openers_first(root: &str, mut rest: Vec<Session>) -> Vec<Session> {
    let mut ordered: Vec<Session> = Vec::new();
    let Some(at) = rest.iter().position(|s| s.id == root) else { return ordered };
    ordered.push(rest.remove(at));
    let mut next = 0;
    while next < ordered.len() && !rest.is_empty() {
        let opener = ordered[next].id.clone();
        let (opened, others): (Vec<Session>, Vec<Session>) =
            rest.into_iter().partition(|s| s.opened_by.as_deref() == Some(opener.as_str()));
        ordered.extend(opened);
        rest = others;
        next += 1;
    }
    ordered.extend(rest);
    ordered
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route<'a> {
    New { title: Option<&'a str> },
    Latest { title: Option<&'a str> },
    Shared,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Routed {
    pub message: Message,
    pub session: Session,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewRun {
    pub id: String,
    pub workflow_id: String,
    pub name: String,
    pub def: WorkflowDef,
    pub request: String,
    pub request_files: Vec<String>,
    pub opener_session: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VisitOpen {
    pub visit_id: String,
    pub step_id: String,
    pub n: u32,
    pub bot_id: String,
    pub session: Option<String>,
    pub title: String,
    pub opened_by: Option<String>,
    pub message_id: String,
    pub message: NewMessage,
    pub mark: WorkflowMark,
    pub seen_ord: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunMoved {
    pub run: WorkflowRun,
    pub result: Option<Message>,
    pub opened: Option<Routed>,
    pub finished: Option<Message>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VisitClose {
    pub visit_id: String,
    pub result_id: String,
    pub result: NewMessage,
    pub mark: WorkflowMark,
    pub outcome: Option<String>,
    pub answered: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunThen {
    Visit(VisitOpen),
    Pause(PauseReason),
    Finish { to_opener: Option<(String, NewMessage, WorkflowMark)>, notice: Option<NewMessage> },
    Remind { id: String, message: NewMessage, mark: WorkflowMark },
}

#[derive(Debug, Clone, Copy)]
pub struct Scheduled<'a> {
    pub read: &'a Task,
    pub next_run_at: &'a str,
}

fn v4_backup_path(db: &Path) -> PathBuf {
    let mut name = db.as_os_str().to_owned();
    name.push(".v4.bak");
    PathBuf::from(name)
}

fn back_up(conn: &Connection, backup: &Path) -> Result<()> {
    if let Ok(meta) = std::fs::symlink_metadata(backup) {
        if meta.is_file() {
            return Ok(());
        }
        return Err(std::io::Error::other(format!("{} is not a regular backup file", backup.display())).into());
    }
    let target = backup.to_str().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{}: not UTF-8", backup.display()))
    })?;
    conn.execute("VACUUM INTO ?1", params![target]).map_err(|e| {
        std::io::Error::other(format!("could not copy the database to {} before updating it: {e}", backup.display()))
    })?;
    tracing::info!(backup = %backup.display(), "copied the database before updating its schema");
    Ok(())
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?, Some(&v4_backup_path(path)))
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?, None)
    }

    fn init(conn: Connection, v4_backup: Option<&Path>) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if current > MIGRATIONS.len() as i64 {
            return Err(crate::Refusal::DatabaseTooNew.into());
        }
        for (i, migration) in MIGRATIONS.iter().enumerate().skip(current as usize) {
            if let (4, Some(backup)) = (i, v4_backup.filter(|_| current > 0)) {
                back_up(&conn, backup)?;
            }
            if i == 15 && current > 0 {
                if let Some(old_backup) = v4_backup {
                    let db = old_backup.to_string_lossy();
                    back_up(&conn, Path::new(&format!("{}.work-items.bak", db.trim_end_matches(".v4.bak"))))?;
                }
            }
            let projects = i == 12;
            if projects {
                if current > 0 {
                    if let Some(old_backup) = v4_backup {
                        let db = old_backup.to_string_lossy();
                        back_up(&conn, Path::new(&format!("{}.projects.bak", db.trim_end_matches(".v4.bak"))))?;
                    }
                }
                conn.pragma_update(None, "foreign_keys", false)?;
            }
            let tx = conn.unchecked_transaction()?;
            migration.apply(&tx)?;
            if projects {
                let violations: i64 =
                    tx.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r.get(0))?;
                if violations != 0 {
                    return Err(CoreError::Internal("project migration failed its foreign key check".into()));
                }
            }
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
            tx.commit()?;
            if projects {
                conn.pragma_update(None, "foreign_keys", true)?;
            }
        }
        Ok(Self { conn: Arc::new(Mutex::new(conn)), project_id: None })
    }

    pub fn insert_bot(&self, new: &NewBot, handle: &str) -> Result<Bot> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let bot = insert_bot_conn(&tx, new, handle, self.project_key())?;
        tx.commit()?;
        Ok(bot)
    }

    pub fn insert_bot_recorded_as(&self, new: &NewBot, handle: &str, setting: &str) -> Result<Bot> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let bot = insert_bot_conn(&tx, new, handle, self.project_key())?;
        if setting == "sonata_master" {
            tx.execute("UPDATE projects SET master_id = ?1 WHERE id = ?2", params![bot.id, self.project_key()])?;
        } else {
            set_setting_conn(&tx, setting, &bot.id)?;
        }
        tx.commit()?;
        Ok(bot)
    }

    pub fn insert_bot_recorded(&self, new: &NewBot, handle: &str, record: &NewCreation) -> Result<Bot> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let bot = insert_bot_conn(&tx, new, handle, self.project_key())?;
        record_creation_conn(&tx, record, &bot.id, &bot.handle)?;
        tx.commit()?;
        Ok(bot)
    }

    pub fn insert_skill_recorded(
        &self,
        slug: &str,
        name: &str,
        description: &str,
        source: SkillSource,
        record: &NewCreation,
    ) -> Result<Skill> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let skill = insert_skill_conn(&tx, slug, name, description, source)?;
        record_creation_conn(&tx, record, &skill.id, &skill.slug)?;
        tx.commit()?;
        Ok(skill)
    }

    pub fn insert_task(&self, row: &TaskRow) -> Result<Task> {
        let conn = self.conn.lock().unwrap();
        Ok(insert_task_conn(&conn, row)?)
    }

    pub fn insert_task_recorded(&self, row: &TaskRow, record: &NewCreation) -> Result<Task> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let task = insert_task_conn(&tx, row)?;
        record_creation_conn(&tx, record, &task.id, &task.name)?;
        tx.commit()?;
        Ok(task)
    }

    pub fn get_task(&self, id: &str) -> Result<Option<Task>> {
        Ok(get_task_conn(&self.conn.lock().unwrap(), id)?.filter(|row| self.includes(&row.project_id)))
    }

    pub fn get_task_by_name(&self, name: &str) -> Result<Option<Task>> {
        Ok(self
            .conn
            .lock()
            .unwrap()
            .query_row(
                &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE project_id = ?2 AND name = ?1 COLLATE NOCASE"),
                params![name, self.project_key()],
                task_from_row,
            )
            .optional()?)
    }

    pub fn list_tasks(&self) -> Result<Vec<Task>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE (?1 IS NULL OR project_id = ?1) ORDER BY created_at, id"
        ))?;
        let tasks = stmt.query_map(params![self.project_id], task_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(tasks)
    }

    pub fn tasks_of_bot(&self, bot_id: &str) -> Result<Vec<Task>> {
        self.check_owner("bots", bot_id)?;
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare(&format!("SELECT {TASK_COLUMNS} FROM tasks WHERE bot_id = ?1 ORDER BY created_at, id"))?;
        let tasks = stmt.query_map(params![bot_id], task_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(tasks)
    }

    pub fn update_task(&self, id: &str, row: &TaskRow) -> Result<Option<Task>> {
        self.check_owner("tasks", id)?;
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(
            "UPDATE tasks SET name = ?2, bot_id = ?3, prompt = ?4, every_days = ?5, at_time = ?6, starts_on = ?7, \
             enabled = ?8, next_run_at = ?9, updated_at = ?10 WHERE id = ?1",
            params![
                id,
                row.name,
                row.bot_id,
                row.prompt,
                row.every_days,
                row.at,
                row.starts_on,
                row.enabled,
                row.next_run_at,
                now()
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        Ok(get_task_conn(&conn, id)?)
    }

    pub fn delete_task(&self, id: &str) -> Result<Option<Task>> {
        self.check_owner("tasks", id)?;
        let conn = self.conn.lock().unwrap();
        let Some(task) = get_task_conn(&conn, id)? else {
            return Ok(None);
        };
        conn.execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
        Ok(Some(task))
    }

    pub fn insert_workflow(&self, new: &NewWorkflow) -> Result<Workflow> {
        let conn = self.conn.lock().unwrap();
        insert_workflow_conn(&conn, new, self.project_key())
    }

    pub fn insert_workflow_recorded(&self, new: &NewWorkflow, record: &NewCreation) -> Result<Workflow> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let workflow = insert_workflow_conn(&tx, new, self.project_key())?;
        record_creation_conn(&tx, record, &workflow.id, &workflow.name)?;
        tx.commit()?;
        Ok(workflow)
    }

    pub fn get_workflow(&self, id: &str) -> Result<Option<Workflow>> {
        Ok(get_workflow_conn(&self.conn.lock().unwrap(), id)?.filter(|row| self.includes(&row.project_id)))
    }

    pub fn get_workflow_by_name(&self, name: &str) -> Result<Option<Workflow>> {
        Ok(self
            .conn
            .lock()
            .unwrap()
            .query_row(
                &format!("SELECT {WORKFLOW_COLUMNS} FROM workflows WHERE project_id = ?2 AND name = ?1 COLLATE NOCASE"),
                params![name, self.project_key()],
                workflow_from_row,
            )
            .optional()?)
    }

    pub fn list_workflows(&self) -> Result<Vec<Workflow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare(&format!("SELECT {WORKFLOW_COLUMNS} FROM workflows WHERE (?1 IS NULL OR project_id = ?1) ORDER BY name COLLATE NOCASE, id"))?;
        let workflows =
            stmt.query_map(params![self.project_id], workflow_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(workflows)
    }

    pub fn update_workflow(&self, id: &str, new: &NewWorkflow) -> Result<Option<Workflow>> {
        self.check_owner("workflows", id)?;
        let conn = self.conn.lock().unwrap();
        let definition = serde_json::to_string(&new.def).map_err(|e| CoreError::Internal(e.to_string()))?;
        let changed = conn.execute(
            "UPDATE workflows SET name = ?2, when_to_use = ?3, definition = ?4, updated_at = ?5 WHERE id = ?1",
            params![id, new.name, new.when_to_use, definition, now()],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        Ok(get_workflow_conn(&conn, id)?)
    }

    pub fn delete_workflow(&self, id: &str) -> Result<Option<Workflow>> {
        self.check_owner("workflows", id)?;
        let conn = self.conn.lock().unwrap();
        let Some(workflow) = get_workflow_conn(&conn, id)? else {
            return Ok(None);
        };
        conn.execute("DELETE FROM workflows WHERE id = ?1", params![id])?;
        Ok(Some(workflow))
    }

    pub fn start_workflow_run(&self, run: &NewRun, first: &VisitOpen) -> Result<RunMoved> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        if get_workflow_conn(&tx, &run.workflow_id)?.is_none() {
            return Err(crate::Refusal::WorkflowNotFound(run.workflow_id.clone()).into());
        }
        if let Some(opener) = &run.opener_session {
            open_session_conn(&tx, opener)?;
        }
        tx.execute(
            "INSERT INTO workflow_runs (id, workflow_id, name, definition, request, request_files, opener_session, \
             status, created_at, project_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, (SELECT project_id FROM workflows WHERE id = ?2))",
            params![
                run.id,
                run.workflow_id,
                run.name,
                serde_json::to_string(&run.def)?,
                run.request,
                serde_json::to_string(&run.request_files)?,
                run.opener_session,
                RunStatus::Running.as_str(),
                now()
            ],
        )?;
        let opened = open_visit_conn(&tx, &run.id, 1, first)?;
        let stored = get_run_conn(&tx, &run.id)?.expect("run just inserted");
        tx.commit()?;
        Ok(RunMoved { run: stored, result: None, opened: Some(opened), finished: None })
    }

    pub fn get_run(&self, id: &str) -> Result<Option<WorkflowRun>> {
        Ok(get_run_conn(&self.conn.lock().unwrap(), id)?.filter(|row| self.includes(&row.project_id)))
    }

    pub fn run_visits(&self, run_id: &str) -> Result<Vec<Visit>> {
        self.check_owner("workflow_runs", run_id)?;
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare(&format!("SELECT {VISIT_COLUMNS} FROM workflow_visits WHERE run_id = ?1 ORDER BY ord"))?;
        let visits = stmt.query_map(params![run_id], visit_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(visits)
    }

    pub fn run_session_of(&self, run_id: &str, bot_id: &str) -> Result<Option<(String, i64)>> {
        self.check_owner("bots", bot_id)?;
        self.check_owner("workflow_runs", run_id)?;
        Ok(self
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT session_id, seen_ord FROM workflow_run_sessions WHERE run_id = ?1 AND bot_id = ?2",
                params![run_id, bot_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    pub fn open_visit_of_session(&self, session_id: &str) -> Result<Option<(WorkflowRun, Visit)>> {
        self.check_owner("sessions", session_id)?;
        let conn = self.conn.lock().unwrap();
        let visit = conn
            .query_row(
                &format!(
                    "SELECT {VISIT_COLUMNS} FROM workflow_visits WHERE session_id = ?1 AND ended_at IS NULL \
                     AND run_id IN (SELECT id FROM workflow_runs WHERE status IN (?2, ?3)) ORDER BY ord DESC LIMIT 1"
                ),
                params![session_id, RunStatus::Running.as_str(), RunStatus::Paused.as_str()],
                visit_from_row,
            )
            .optional()?;
        let Some(visit) = visit else { return Ok(None) };
        if !visit_delivered_conn(&conn, &visit)? {
            return Ok(None);
        }
        Ok(get_run_conn(&conn, &visit.run_id)?.map(|run| (run, visit)))
    }

    pub fn advance_workflow_run(&self, close: &VisitClose, then: &RunThen) -> Result<RunMoved> {
        if let RunThen::Remind { .. } = then {
            return Err(CoreError::Internal("a visit that closes owes no result to be reminded of".into()));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let run_id = &close.mark.run_id;
        let not_found = || crate::Refusal::WorkflowRunNotFound(run_id.clone());
        let visit = get_visit_conn(&tx, &close.visit_id)?
            .filter(|v| v.ended_at.is_none() && v.run_id == *run_id)
            .ok_or_else(not_found)?;
        if !visit_delivered_conn(&tx, &visit)? {
            return Err(not_found().into());
        }
        let run = get_run_conn(&tx, run_id)?.ok_or_else(not_found)?;
        match (run.status, &run.pause) {
            (RunStatus::Running, _) => {}
            (RunStatus::Paused, Some(PauseReason::NoResult { step_id })) if *step_id == visit.step_id => {
                tx.execute(
                    "UPDATE workflow_runs SET status = ?2, pause = NULL WHERE id = ?1",
                    params![run_id, RunStatus::Running.as_str()],
                )?;
            }
            _ => return Err(not_found().into()),
        }
        let ended = now();
        let result = insert_marked_conn(&tx, &close.result_id, &close.result, &close.mark)?;
        if let Some(session) = &visit.session_id {
            orchestration::finish_step_graph(&tx, session, &result)?;
        }
        if let Some(answered) = &close.answered {
            tx.execute("UPDATE messages SET answered_by = ?2 WHERE id = ?1", params![answered, close.result_id])?;
        }
        tx.execute(
            "UPDATE workflow_visits SET result_message = ?2, outcome = ?3, ended_at = ?4 WHERE id = ?1",
            params![visit.id, close.result_id, close.outcome, ended],
        )?;
        tx.execute("DELETE FROM work_item_pending_results WHERE session_id=?1", [&visit.session_id])?;
        let (opened, finished) = run_then_conn(&tx, run_id, then, &ended)?;
        let result = get_message_conn(&tx, &close.result_id)?;
        let run = get_run_conn(&tx, run_id)?.expect("run just moved");
        tx.commit()?;
        Ok(RunMoved { run, result, opened, finished })
    }

    pub fn resume_workflow_run(&self, run_id: &str, extra_for: Option<&str>, then: &RunThen) -> Result<RunMoved> {
        self.check_owner("workflow_runs", run_id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut run =
            get_run_conn(&tx, run_id)?.ok_or_else(|| crate::Refusal::WorkflowRunNotFound(run_id.to_string()))?;
        if run.status != RunStatus::Paused {
            return Err(crate::Refusal::WorkflowRunNotPaused.into());
        }
        if let Some(step) = extra_for {
            *run.extra_visits.entry(step.to_string()).or_default() += 1;
        }
        tx.execute(
            "UPDATE workflow_runs SET status = ?2, pause = NULL, extra_visits = ?3 WHERE id = ?1",
            params![run_id, RunStatus::Running.as_str(), serde_json::to_string(&run.extra_visits)?],
        )?;
        let (opened, finished) = run_then_conn(&tx, run_id, then, &now())?;
        let run = get_run_conn(&tx, run_id)?.expect("run just resumed");
        tx.commit()?;
        Ok(RunMoved { run, result: None, opened, finished })
    }

    pub fn pause_workflow_run(&self, run_id: &str, reason: &PauseReason) -> Result<Option<WorkflowRun>> {
        self.check_owner("workflow_runs", run_id)?;
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(
            "UPDATE workflow_runs SET status = ?2, pause = ?3 WHERE id = ?1 AND status = ?4",
            params![run_id, RunStatus::Paused.as_str(), serde_json::to_string(reason)?, RunStatus::Running.as_str()],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        Ok(get_run_conn(&conn, run_id)?)
    }

    pub fn stop_workflow_run(&self, run_id: &str) -> Result<Option<WorkflowRun>> {
        self.check_owner("workflow_runs", run_id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let run = stop_run_conn(&tx, run_id, &now())?;
        tx.commit()?;
        Ok(run)
    }

    pub fn stop_runs_of_sessions(&self, session_ids: &[String]) -> Result<Vec<WorkflowRun>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let marks = placeholders(session_ids.len());
        let ids: Vec<String> = tx
            .prepare(&format!(
                "SELECT id FROM workflow_runs WHERE status IN ('{}', '{}') AND (opener_session IN ({marks}) \
                 OR id IN (SELECT run_id FROM workflow_run_sessions WHERE session_id IN ({marks}))) \
                 ORDER BY created_at, id",
                RunStatus::Running.as_str(),
                RunStatus::Paused.as_str()
            ))?
            .query_map(rusqlite::params_from_iter(session_ids), |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let at = now();
        let mut stopped = Vec::new();
        for id in &ids {
            stopped.extend(stop_run_conn(&tx, id, &at)?);
        }
        tx.commit()?;
        Ok(stopped)
    }

    pub fn run_session_ids(&self, run_id: &str) -> Result<Vec<String>> {
        self.check_owner("workflow_runs", run_id)?;
        let conn = self.conn.lock().unwrap();
        let mut ids: Vec<String> = conn
            .prepare("SELECT session_id FROM workflow_run_sessions WHERE run_id = ?1 ORDER BY rowid")?
            .query_map(params![run_id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let opener: Option<String> = conn
            .query_row("SELECT opener_session FROM workflow_runs WHERE id = ?1", params![run_id], |r| r.get(0))
            .optional()?
            .flatten();
        ids.extend(opener.filter(|o| !ids.contains(o)));
        Ok(ids)
    }

    pub fn latest_run_of_session(&self, session_id: &str) -> Result<Option<WorkflowRun>> {
        self.check_owner("sessions", session_id)?;
        Ok(self
            .conn
            .lock()
            .unwrap()
            .query_row(
                &format!(
                    "SELECT {RUN_COLUMNS} FROM workflow_runs WHERE opener_session = ?1 \
                     OR id IN (SELECT run_id FROM workflow_run_sessions WHERE session_id = ?1) \
                     ORDER BY created_at DESC, id DESC LIMIT 1"
                ),
                params![session_id],
                run_from_row,
            )
            .optional()?)
    }

    pub fn pause_runs_missing_a_session(&self) -> Result<Vec<WorkflowRun>> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let found: Vec<(String, String)> = tx
            .prepare(
                "SELECT r.id, v.step_id FROM workflow_runs r JOIN workflow_visits v ON v.run_id = r.id \
                 WHERE r.status IN (?1, ?2) AND v.ended_at IS NULL AND v.session_id IS NULL \
                 AND COALESCE(json_extract(r.pause, '$.kind'), '') != 'bot_missing' ORDER BY r.created_at, r.id",
            )?
            .query_map(params![RunStatus::Running.as_str(), RunStatus::Paused.as_str()], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut paused = Vec::new();
        for (id, step_id) in &found {
            let reason = PauseReason::BotMissing { step_id: step_id.clone() };
            tx.execute(
                "UPDATE workflow_runs SET status = ?2, pause = ?3 WHERE id = ?1",
                params![id, RunStatus::Paused.as_str(), serde_json::to_string(&reason)?],
            )?;
            paused.extend(get_run_conn(&tx, id)?);
        }
        tx.commit()?;
        Ok(paused)
    }

    pub fn session_of_users_run(&self, session_id: &str) -> Result<bool> {
        self.check_owner("sessions", session_id)?;
        Ok(self.conn.lock().unwrap().query_row(
            "SELECT EXISTS (SELECT 1 FROM workflow_run_sessions s JOIN workflow_runs r ON r.id = s.run_id \
             WHERE s.session_id = ?1 AND r.opener_session IS NULL)",
            params![session_id],
            |r| r.get(0),
        )?)
    }

    pub fn visit_is_over(&self, mark: &WorkflowMark) -> Result<bool> {
        Ok(!self.conn.lock().unwrap().query_row(
            "SELECT EXISTS (SELECT 1 FROM workflow_visits v JOIN workflow_runs r ON r.id = v.run_id \
             WHERE v.id = ?1 AND v.run_id = ?2 AND v.ended_at IS NULL AND r.status = ?3)",
            params![mark.visit_id, mark.run_id, RunStatus::Running.as_str()],
            |r| r.get::<_, bool>(0),
        )?)
    }

    pub fn mark_visit_reminded(&self, visit_id: &str) -> Result<()> {
        self.conn
            .lock()
            .unwrap()
            .execute("UPDATE workflow_visits SET reminded = 1 WHERE id = ?1", params![visit_id])?;
        Ok(())
    }

    pub fn insert_marked(&self, id: &str, m: &NewMessage, mark: &WorkflowMark) -> Result<Message> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let message = insert_marked_conn(&tx, id, m, mark)?;
        tx.commit()?;
        Ok(message)
    }

    pub fn owed_answers(&self, session_id: &str, since: &str) -> Result<u32> {
        self.check_owner("sessions", session_id)?;
        Ok(self.conn.lock().unwrap().query_row(
            "SELECT COUNT(*) FROM messages m WHERE m.sender_session = ?1 AND m.created_at >= ?2 \
             AND m.recipient <> ?3 AND m.in_reply_to IS NULL AND m.answered_by IS NULL \
             AND m.status NOT IN ('cancelled', 'failed', 'retried') \
             AND NOT EXISTS (SELECT 1 FROM messages r WHERE r.in_reply_to = m.id)",
            params![session_id, since, USER],
            |r| r.get(0),
        )?)
    }

    pub fn last_reply_since(&self, session_id: &str, since: &str) -> Result<Option<Message>> {
        self.check_owner("sessions", session_id)?;
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row(
                &format!(
                    "SELECT {MESSAGE_COLUMNS} FROM messages WHERE sender_session = ?1 AND recipient = ?2 \
                     AND created_at >= ?3 AND kind IN (?4, ?5) AND workflow_mark IS NULL ORDER BY seq DESC LIMIT 1"
                ),
                params![session_id, USER, since, MessageKind::Text.as_str(), MessageKind::RawFallback.as_str()],
                message_from_row,
            )
            .optional()?)
    }

    pub fn tasks_due(&self, now: &str) -> Result<Vec<Task>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE project_id IN (SELECT id FROM projects WHERE archived_at IS NULL) AND enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?1 \
             ORDER BY next_run_at, created_at, id"
        ))?;
        let tasks = stmt.query_map(params![now], task_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(tasks)
    }

    pub fn skip_task_slot(&self, slot: Scheduled<'_>, skipped: &str) -> Result<Option<Task>> {
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(
            "UPDATE tasks SET skipped_at = ?2, next_run_at = ?3 \
             WHERE id = ?1 AND enabled = 1 AND next_run_at IS ?4 AND last_run_at IS ?5",
            params![slot.read.id, skipped, slot.next_run_at, slot.read.next_run_at, slot.read.last_run_at],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        Ok(get_task_conn(&conn, &slot.read.id)?)
    }

    pub fn start_task_run(
        &self,
        task_id: &str,
        title: &str,
        body: &str,
        since: Option<&str>,
        scheduled: Option<Scheduled<'_>>,
    ) -> Result<Option<(Routed, Task)>> {
        self.check_owner("tasks", task_id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let Some(task) = get_task_conn(&tx, task_id)? else {
            return Ok(None);
        };
        if let Some(slot) = scheduled {
            let read = slot.read;
            if !(task.enabled && task.next_run_at == read.next_run_at && task.last_run_at == read.last_run_at) {
                return Ok(None);
            }
        }
        let next_run_at = scheduled.map(|slot| slot.next_run_at);
        let session = create_session_conn(&tx, &task.bot_id, Some(title), None)?;
        let message_id = new_id();
        let new = NewMessage {
            sender: USER.into(),
            recipient: task.bot_id.clone(),
            sender_session: None,
            recipient_session: Some(session.id.clone()),
            kind: MessageKind::Text,
            body: body.into(),
            attachments: vec![],
            system_code: None,
            system_params: None,
            status: MessageStatus::Queued,
            chain_id: new_id(),
            hop: 0,
            in_reply_to: None,
        };
        insert_message_conn(&tx, &message_id, &new)?;
        tx.execute(
            "UPDATE messages SET task_id = ?2, task_name = ?3, task_since = ?4 WHERE id = ?1",
            params![message_id, task.id, task.name, since],
        )?;
        let t = now();
        tx.execute(
            "UPDATE tasks SET last_run_at = ?2, last_run_session = ?3, next_run_at = COALESCE(?4, next_run_at) \
             WHERE id = ?1",
            params![task.id, t, session.id, next_run_at],
        )?;
        let message = get_message_conn(&tx, &message_id)?.expect("message just inserted");
        let session = get_session_conn(&tx, &session.id)?.expect("session just created");
        let task = get_task_conn(&tx, &task.id)?.expect("task just updated");
        tx.commit()?;
        Ok(Some((Routed { message, session, created: true }, task)))
    }

    pub fn last_completed_run(&self, task_id: &str) -> Result<Option<String>> {
        self.check_owner("tasks", task_id)?;
        Ok(self.conn.lock().unwrap().query_row(
            "SELECT MAX(s.created_at) FROM messages m JOIN sessions s ON s.id = m.recipient_session \
             WHERE m.task_id = ?1 AND m.task_done_at IS NOT NULL",
            params![task_id],
            |r| r.get(0),
        )?)
    }

    pub fn mark_task_done(&self, message_id: &str) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE messages SET task_done_at = ?2 WHERE id = ?1 AND task_id IS NOT NULL",
            params![message_id, now()],
        )?;
        Ok(())
    }

    pub fn messages_of_session(&self, session_id: &str) -> Result<Vec<Message>> {
        self.check_owner("sessions", session_id)?;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages WHERE sender_session = ?1 OR recipient_session = ?1 ORDER BY seq"
        ))?;
        let msgs = stmt.query_map(params![session_id], message_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(msgs)
    }

    pub fn creation(&self, session_id: &str, kind: CreationKind, key: &str) -> Result<Option<Creation>> {
        self.check_owner("sessions", session_id)?;
        Ok(self
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT fingerprint, result_id, result_name FROM creations WHERE session_id = ?1 AND kind = ?2 AND key = ?3",
                params![session_id, kind.as_str(), key],
                |r| Ok(Creation { fingerprint: r.get(0)?, result_id: r.get(1)?, result_name: r.get(2)? }),
            )
            .optional()?)
    }

    pub fn get_bot(&self, id: &str) -> Result<Option<Bot>> {
        Ok(get_bot_conn(&self.conn.lock().unwrap(), id)?.filter(|row| self.includes(&row.project_id)))
    }

    pub fn get_bot_by_handle(&self, handle: &str) -> Result<Option<Bot>> {
        let conn = self.conn.lock().unwrap();
        let bot = conn
            .query_row(
                &format!("SELECT {BOT_COLUMNS} FROM bots WHERE project_id = ?2 AND handle = ?1"),
                params![handle, self.project_key()],
                bot_from_row,
            )
            .optional()?;
        Ok(bot.map(|bot| with_skills(&conn, bot)).transpose()?)
    }

    pub fn list_bots(&self) -> Result<Vec<Bot>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {BOT_COLUMNS} FROM bots WHERE (?1 IS NULL OR project_id = ?1) ORDER BY created_at, id"
        ))?;
        let bots = stmt.query_map(params![self.project_id], bot_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(bots.into_iter().map(|bot| with_skills(&conn, bot)).collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn bot_handles(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT handle FROM bots WHERE (?1 IS NULL OR project_id = ?1) ORDER BY created_at, id")?;
        let handles =
            stmt.query_map(params![self.project_id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(handles)
    }

    pub fn create_session(&self, bot_id: &str, title: Option<&str>, opened_by: Option<&str>) -> Result<Session> {
        self.check_owner("bots", bot_id)?;
        Ok(create_session_conn(&self.conn.lock().unwrap(), bot_id, title, opened_by)?)
    }

    pub fn get_session(&self, id: &str) -> Result<Option<Session>> {
        Ok(get_session_conn(&self.conn.lock().unwrap(), id)?.filter(|row| self.includes(&row.project_id)))
    }

    pub fn set_session_seen(&self, session_id: &str) -> Result<Option<Session>> {
        self.check_owner("sessions", session_id)?;
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE sessions SET seen_at = ?2 WHERE id = ?1", params![session_id, now()])?;
        Ok(get_session_conn(&conn, session_id)?)
    }

    pub fn sessions_of(&self, bot_id: &str) -> Result<Vec<Session>> {
        self.check_owner("bots", bot_id)?;
        Ok(sessions_of_conn(&self.conn.lock().unwrap(), bot_id)?)
    }

    pub fn latest_session(&self, bot_id: &str) -> Result<Option<Session>> {
        self.check_owner("bots", bot_id)?;
        Ok(sessions_of_conn(&self.conn.lock().unwrap(), bot_id)?.into_iter().find(|s| s.archived_at.is_none()))
    }

    pub fn set_session_cli_id(
        &self,
        session_id: &str,
        cli_id: Option<&str>,
        config_version: i64,
        provider: Provider,
    ) -> Result<()> {
        self.check_owner("sessions", session_id)?;
        self.conn.lock().unwrap().execute(
            "UPDATE sessions SET provider_session_id = ?2, \
             session_config_version = CASE WHEN ?2 IS NULL THEN NULL ELSE ?3 END \
             WHERE id = ?1 AND bot_id IN (SELECT id FROM bots WHERE provider = ?4)",
            params![session_id, cli_id, config_version, provider.as_str()],
        )?;
        Ok(())
    }

    pub fn fill_session_cli_id(
        &self,
        session_id: &str,
        cli_id: &str,
        config_version: i64,
        provider: Provider,
    ) -> Result<bool> {
        self.check_owner("sessions", session_id)?;
        let changed = self.conn.lock().unwrap().execute(
            "UPDATE sessions SET provider_session_id = ?2, session_config_version = ?3 \
             WHERE id = ?1 AND provider_session_id IS NULL \
             AND bot_id IN (SELECT id FROM bots WHERE provider = ?4)",
            params![session_id, cli_id, config_version, provider.as_str()],
        )?;
        Ok(changed > 0)
    }

    pub fn set_session_title(&self, session_id: &str, title: Option<&str>) -> Result<()> {
        self.check_owner("sessions", session_id)?;
        self.conn
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET title = ?2 WHERE id = ?1", params![session_id, title])?;
        Ok(())
    }

    pub fn touch_session(&self, session_id: &str) -> Result<()> {
        self.check_owner("sessions", session_id)?;
        self.conn
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET last_activity_at = ?2 WHERE id = ?1", params![session_id, now()])?;
        Ok(())
    }

    pub fn delete_bot(&self, id: &str) -> Result<Option<(Bot, Vec<Message>)>> {
        self.check_owner("bots", id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let Some(bot) = get_bot_conn(&tx, id)? else {
            return Ok(None);
        };
        let queued: Vec<String> = tx
            .prepare(
                "SELECT id FROM messages WHERE status IN ('queued', 'interrupted') AND (recipient = ?1 OR sender = ?1) ORDER BY seq",
            )?
            .query_map(params![id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        tx.execute(
            "UPDATE messages SET status = 'cancelled' WHERE status IN ('queued', 'interrupted') AND (recipient = ?1 OR sender = ?1)",
            params![id],
        )?;
        tx.execute("DELETE FROM bots WHERE id = ?1", params![id])?;
        let mut cancelled = Vec::new();
        for message_id in &queued {
            cancelled.extend(get_message_conn(&tx, message_id)?);
        }
        tx.commit()?;
        Ok(Some((bot, cancelled)))
    }

    pub fn remove_session(
        &self,
        id: &str,
        check: impl FnOnce(&[Session]) -> Result<()>,
    ) -> Result<Option<RemovedSessions>> {
        self.check_owner("sessions", id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let sessions = tree_conn(&tx, id)?;
        if sessions.is_empty() {
            return Ok(None);
        }
        check(&sessions)?;
        let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        let marks = placeholders(ids.len());
        let threads: Vec<(String, String, String)> = tx
            .prepare(&format!(
                "SELECT id, participant_a, participant_b FROM threads \
                 WHERE participant_a IN ({marks}) OR participant_b IN ({marks}) ORDER BY id"
            ))?
            .query_map(rusqlite::params_from_iter(&ids), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let thread_ids: Vec<String> = threads.iter().map(|t| t.0.clone()).collect();
        let thread_marks = placeholders(thread_ids.len());
        let messages: Vec<Message> = tx
            .prepare(&format!(
                "SELECT {MESSAGE_COLUMNS} FROM messages WHERE thread_id IN ({thread_marks}) ORDER BY seq"
            ))?
            .query_map(rusqlite::params_from_iter(&thread_ids), message_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        tx.execute(
            &format!("DELETE FROM messages WHERE thread_id IN ({thread_marks})"),
            rusqlite::params_from_iter(&thread_ids),
        )?;
        tx.execute(
            &format!("DELETE FROM threads WHERE id IN ({thread_marks})"),
            rusqlite::params_from_iter(&thread_ids),
        )?;
        tx.execute(&format!("DELETE FROM sessions WHERE id IN ({marks})"), rusqlite::params_from_iter(&ids))?;
        tx.commit()?;
        let sessions = sessions
            .into_iter()
            .map(|session| {
                let thread_ids = threads
                    .iter()
                    .filter(|(_, a, b)| *a == session.id || *b == session.id)
                    .map(|(id, _, _)| id.clone())
                    .collect();
                RemovedSession { session, thread_ids }
            })
            .collect();
        Ok(Some(RemovedSessions { sessions, thread_ids, messages, drafts: Vec::new() }))
    }

    pub fn session_tree(&self, id: &str) -> Result<Vec<Session>> {
        self.check_owner("sessions", id)?;
        Ok(tree_conn(&self.conn.lock().unwrap(), id)?)
    }

    pub fn archive_session(
        &self,
        id: &str,
        check: impl FnOnce(&[Session]) -> Result<()>,
    ) -> Result<Option<Vec<Session>>> {
        self.check_owner("sessions", id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let tree = tree_conn(&tx, id)?;
        if tree.is_empty() {
            return Ok(None);
        }
        check(&tree)?;
        let at = now();
        let mut archived = Vec::new();
        for session in tree.into_iter().filter(|s| s.archived_at.is_none()) {
            tx.execute(
                "UPDATE sessions SET archived_at = ?2, waiting_question_id = NULL, waiting_since = NULL, \
                 waiting_options = NULL, open_request_id = NULL WHERE id = ?1",
                params![session.id, at],
            )?;
            archived.push(Session {
                archived_at: Some(at.clone()),
                waiting_question_id: None,
                waiting_since: None,
                waiting_options: vec![],
                open_request_id: None,
                ..session
            });
        }
        tx.commit()?;
        Ok(Some(archived))
    }

    pub fn restore_session(&self, id: &str) -> Result<Option<Vec<Session>>> {
        self.check_owner("sessions", id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let tree = tree_conn(&tx, id)?;
        let Some(mark) = tree.first().map(|root| root.archived_at.clone()) else { return Ok(None) };
        let mut restored = Vec::new();
        for session in tree.into_iter().filter(|s| mark.is_some() && s.archived_at == mark) {
            tx.execute("UPDATE sessions SET archived_at = NULL WHERE id = ?1", params![session.id])?;
            restored.push(Session { archived_at: None, ..session });
        }
        tx.commit()?;
        Ok(Some(restored))
    }

    pub fn update_bot(&self, id: &str, edit: &NewBot) -> Result<Option<Bot>> {
        self.check_owner("bots", id)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let Some(old) = get_bot_conn(&tx, id)? else { return Ok(None) };
        let provider_changed = old.provider != edit.provider;
        let skills = id_set(&edit.skill_ids);
        let config_changed = provider_changed
            || old.display_name != edit.display_name
            || old.model != edit.model
            || old.instructions != edit.instructions
            || id_set(&old.skill_ids) != skills;
        tx.execute(
            "UPDATE bots SET display_name = ?2, color = ?3, provider = ?4, model = ?5, instructions = ?6, \
             config_version = config_version + ?7, updated_at = ?8 WHERE id = ?1",
            params![
                id,
                edit.display_name,
                edit.color,
                edit.provider.as_str(),
                edit.model,
                edit.instructions,
                config_changed as i64,
                now()
            ],
        )?;
        if provider_changed {
            tx.execute(
                "UPDATE sessions SET provider_session_id = NULL, session_config_version = NULL WHERE bot_id = ?1",
                params![id],
            )?;
        }
        set_bot_skills_conn(&tx, id, &skills)?;
        let bot = get_bot_conn(&tx, id)?;
        tx.commit()?;
        Ok(bot)
    }

    pub fn forget_cli_sessions(&self, bot_id: &str) -> Result<()> {
        self.check_owner("bots", bot_id)?;
        self.conn.lock().unwrap().execute(
            "UPDATE sessions SET provider_session_id = NULL, session_config_version = NULL WHERE bot_id = ?1",
            params![bot_id],
        )?;
        Ok(())
    }

    pub fn thread_for(&self, a: &str, b: &str) -> Result<String> {
        Ok(thread_for_conn(&self.conn.lock().unwrap(), a, b)?)
    }

    pub fn insert_question(
        &self,
        id: &str,
        question: &NewMessage,
        options: &[String],
        open_request: Option<&str>,
        notice: Option<&NewMessage>,
        answered: Option<&str>,
    ) -> Result<Question> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let message = insert_message_conn(&tx, id, question)?;
        let session_id = question
            .sender_session
            .as_deref()
            .ok_or_else(|| CoreError::Internal("a question names the session that asks it".into()))?;
        tx.execute(
            "UPDATE sessions SET waiting_question_id = ?2, waiting_since = ?3, waiting_options = ?4 WHERE id = ?1",
            params![session_id, id, now(), (!options.is_empty()).then(|| serde_json::to_string(options)).transpose()?],
        )?;
        if let Some(request) = open_request {
            tx.execute("UPDATE sessions SET open_request_id = ?2 WHERE id = ?1", params![session_id, request])?;
        }
        let notice = notice.map(|n| insert_message_conn(&tx, &new_id(), n)).transpose()?;
        if let Some(answered) = answered {
            tx.execute("UPDATE messages SET answered_by = ?2 WHERE id = ?1", params![answered, id])?;
        }
        let session = get_session_conn(&tx, session_id)?.expect("session just marked");
        tx.commit()?;
        Ok(Question { message, notice, session })
    }

    pub fn insert_result(&self, id: &str, result: &NewMessage, answered: Option<&str>) -> Result<(Message, Session)> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let message = insert_message_conn(&tx, id, result)?;
        let session_id = result
            .sender_session
            .as_deref()
            .ok_or_else(|| CoreError::Internal("a result names the session that sends it".into()))?;
        tx.execute("UPDATE sessions SET open_request_id = NULL WHERE id = ?1", params![session_id])?;
        if let Some(answered) = answered {
            tx.execute("UPDATE messages SET answered_by = ?2 WHERE id = ?1", params![answered, id])?;
        }
        let session = get_session_conn(&tx, session_id)?.expect("sender session exists");
        if !orchestration::result_conn(&tx, session_id, &message)? {
            let owner = work_items::owner(&tx, session_id)?;
            if owner.as_deref() == Some(session_id) {
                work_items::complete(&tx, session_id, &message)?;
            }
        }
        let message = get_message_conn(&tx, id)?.expect("result just inserted");
        tx.commit()?;
        Ok((message, session))
    }

    pub fn answered_by(&self, message_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT answered_by FROM messages WHERE id = ?1", params![message_id], |r| r.get(0))
            .optional()?
            .flatten())
    }

    pub fn chain_has_party(&self, chain_id: &str, bot_id: &str) -> Result<bool> {
        self.check_owner("bots", bot_id)?;
        Ok(self.conn.lock().unwrap().query_row(
            "SELECT EXISTS (SELECT 1 FROM messages WHERE chain_id = ?1 AND (sender = ?2 OR recipient = ?2))",
            params![chain_id, bot_id],
            |r| r.get(0),
        )?)
    }

    pub fn insert_message(&self, m: &NewMessage) -> Result<Message> {
        self.insert_message_with_id(&new_id(), m)
    }

    pub fn insert_message_with_id(&self, id: &str, m: &NewMessage) -> Result<Message> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let message = insert_message_conn(&tx, id, m)?;
        tx.commit()?;
        Ok(message)
    }

    pub fn insert_routed(&self, id: &str, m: &NewMessage, route: Route) -> Result<Routed> {
        self.check_owner("bots", &m.recipient)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        if let Some(sender) = &m.sender_session {
            open_session_conn(&tx, sender)?;
        }
        let (found, title) = match route {
            Route::New { title } => (None, title),
            Route::Latest { title } => {
                (sessions_of_conn(&tx, &m.recipient)?.into_iter().find(|s| s.archived_at.is_none()), title)
            }
            Route::Shared => match &m.sender_session {
                Some(asker) => (shared_session_conn(&tx, &m.recipient, asker)?, None),
                None => {
                    return Err(CoreError::Internal("a message to a shared session names no sender session".into()))
                }
            },
        };
        let created = found.is_none();
        let session = match found {
            Some(session) => session,
            None => create_session_conn(&tx, &m.recipient, title, m.sender_session.as_deref())?,
        };
        let message =
            insert_message_conn(&tx, id, &NewMessage { recipient_session: Some(session.id.clone()), ..m.clone() })?;
        let session = get_session_conn(&tx, &session.id)?.expect("session just used");
        tx.commit()?;
        Ok(Routed { message, session, created })
    }

    pub fn insert_workflow_request(&self, id: &str, m: &NewMessage, title: Option<&str>) -> Result<Routed> {
        self.check_owner("bots", &m.recipient)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let session = create_session_conn(&tx, &m.recipient, title, None)?;
        insert_message_conn(&tx, id, &NewMessage { recipient_session: Some(session.id.clone()), ..m.clone() })?;
        tx.execute("UPDATE messages SET workflow_request = 1 WHERE id = ?1", params![id])?;
        let message = get_message_conn(&tx, id)?.expect("message just inserted");
        let session = get_session_conn(&tx, &session.id)?.expect("session just created");
        tx.commit()?;
        Ok(Routed { message, session, created: true })
    }

    /// Queue the generation and persist its authority and recovery state together.
    pub fn insert_onboarding_request(
        &self,
        id: &str,
        m: &NewMessage,
        key: &str,
        mut state: serde_json::Value,
    ) -> Result<Routed> {
        self.check_owner("bots", &m.recipient)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let session = create_session_conn(&tx, &m.recipient, Some("Personalize workflows"), None)?;
        let message =
            insert_message_conn(&tx, id, &NewMessage { recipient_session: Some(session.id.clone()), ..m.clone() })?;
        state["session_id"] = serde_json::json!(session.id);
        state["message_id"] = serde_json::json!(message.id);
        set_setting_conn(&tx, key, &serde_json::to_string(&state)?)?;
        set_setting_conn(&tx, &format!("onboarding.session.{}", session.id), "true")?;
        tx.commit()?;
        Ok(Routed { message, session, created: true })
    }

    pub fn retry_interrupted(&self, id: &str, copy: &NewMessage) -> Result<Option<(Message, Message)>> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let changed =
            tx.execute("UPDATE messages SET status = 'retried' WHERE id = ?1 AND status = 'interrupted'", params![id])?;
        if changed == 0 {
            return Ok(None);
        }
        let copy = insert_message_conn(&tx, &new_id(), copy)?;
        tx.execute(
            "UPDATE messages SET (task_id, task_name, task_since) = \
             (SELECT task_id, task_name, task_since FROM messages WHERE id = ?2) WHERE id = ?1",
            params![copy.id, id],
        )?;
        tx.execute(
            "UPDATE messages SET (workflow_mark, workflow_request) = \
             (SELECT workflow_mark, workflow_request FROM messages WHERE id = ?2) WHERE id = ?1",
            params![copy.id, id],
        )?;
        let copy = get_message_conn(&tx, &copy.id)?.expect("message just inserted");
        let original = get_message_conn(&tx, id)?.ok_or_else(|| crate::Refusal::MessageNotFound(id.to_string()))?;
        tx.commit()?;
        Ok(Some((original, copy)))
    }

    pub fn get_message(&self, id: &str) -> Result<Option<Message>> {
        self.check_owner("messages", id)?;
        Ok(get_message_conn(&self.conn.lock().unwrap(), id)?)
    }

    pub fn activity_rows(&self, session_id: &str, in_flight: Option<&str>) -> Result<Option<ActivityRows>> {
        self.check_owner("sessions", session_id)?;
        let conn = self.conn.lock().unwrap();
        let Some(session) = get_session_conn(&conn, session_id)? else {
            return Ok(None);
        };
        let one = |filter: &str| -> Result<Option<Message>> {
            Ok(conn.query_row(&latest_where(filter), params![session_id], message_from_row).optional()?)
        };
        let in_flight = in_flight.map(|id| get_message_conn(&conn, id)).transpose()?.flatten();
        let question =
            session.waiting_question_id.as_deref().map(|id| get_message_conn(&conn, id)).transpose()?.flatten();
        let queued: u32 = conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE recipient_session = ?1 AND status = 'queued'",
            params![session_id],
            |r| r.get(0),
        )?;
        let last_for_user_at: Option<String> = conn.query_row(LAST_FOR_USER_AT, params![session_id], |r| r.get(0))?;
        let run_of: Option<String> = conn
            .query_row(
                "SELECT task_name FROM messages WHERE recipient_session = ?1 AND task_id IS NOT NULL \
                 ORDER BY seq LIMIT 1",
                params![session_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        Ok(Some(ActivityRows {
            in_flight,
            queued,
            question,
            last_sent: one(LAST_SENT)?,
            last_to: one("recipient_session = ?1 AND kind != 'system' AND status != 'cancelled'")?,
            last_reply: one(LAST_REPLY)?,
            last_for_user_at,
            run_of,
        }))
    }

    pub fn thread_messages(&self, thread_id: &str) -> Result<Vec<Message>> {
        self.check_owner("threads", thread_id)?;
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE thread_id = ?1 ORDER BY seq"))?;
        let msgs = stmt.query_map(params![thread_id], message_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(msgs)
    }

    pub fn bot_threads(&self) -> Result<Vec<ThreadSummary>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT t.id, COALESCE(sa.bot_id, t.participant_a), COALESCE(sb.bot_id, t.participant_b), \
             MAX(m.created_at), sa.id, sb.id FROM threads t \
             JOIN messages m ON m.thread_id = t.id \
             LEFT JOIN sessions sa ON sa.id = t.participant_a \
             LEFT JOIN sessions sb ON sb.id = t.participant_b \
             WHERE t.participant_a <> ?1 AND t.participant_b <> ?1 AND (?2 IS NULL OR sa.project_id = ?2 OR sb.project_id = ?2) \
             GROUP BY t.id ORDER BY MAX(m.seq) DESC",
        )?;
        let threads = stmt
            .query_map(params![USER, self.project_id], |r| {
                let (a, b): (String, String) = (r.get(1)?, r.get(2)?);
                let (sa, sb): (Option<String>, Option<String>) = (r.get(4)?, r.get(5)?);
                let ((a, sa), (b, sb)) = if a <= b { ((a, sa), (b, sb)) } else { ((b, sb), (a, sa)) };
                Ok(ThreadSummary {
                    id: r.get(0)?,
                    participant_a: a,
                    participant_b: b,
                    session_a: sa,
                    session_b: sb,
                    last_message_at: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(threads)
    }

    pub fn queued_for_session(&self, session_id: &str) -> Result<Vec<Message>> {
        self.check_owner("sessions", session_id)?;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages WHERE recipient_session = ?1 AND status = 'queued' AND (COALESCE(system_code, '') != 'orchestration_event' OR NOT EXISTS(SELECT 1 FROM sessions WHERE id = ?1 AND waiting_question_id IS NOT NULL)) ORDER BY seq"
        ))?;
        let msgs = stmt.query_map(params![session_id], message_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(msgs)
    }

    pub fn last_delivered_to_session(&self, session_id: &str) -> Result<Option<Message>> {
        self.check_owner("sessions", session_id)?;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages WHERE recipient_session = ?1 AND status = 'delivered' \
             ORDER BY seq DESC LIMIT 1"
        ))?;
        Ok(stmt.query_row(params![session_id], message_from_row).optional()?)
    }

    pub fn unanswered_deliveries(&self) -> Result<Vec<Message>> {
        let conn = self.conn.lock().unwrap();
        let columns: Vec<String> = MESSAGE_COLUMNS.split(',').map(|c| format!("m.{}", c.trim())).collect();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM messages m LEFT JOIN messages r ON r.in_reply_to = m.id \
             WHERE m.status = 'delivered' AND m.recipient_session IN (SELECT id FROM sessions) AND r.seq IS NULL \
             AND m.turn_ended_at IS NULL AND m.answered_by IS NULL ORDER BY m.seq",
            columns.join(", ")
        ))?;
        let msgs = stmt.query_map([], message_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(msgs)
    }

    pub fn sessions_with_queued(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT recipient_session FROM messages \
             WHERE status = 'queued' AND recipient_session IN (SELECT id FROM sessions WHERE archived_at IS NULL AND project_id IN (SELECT id FROM projects WHERE archived_at IS NULL)) \
             GROUP BY recipient_session ORDER BY MIN(seq)",
        )?;
        let ids = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids)
    }

    pub fn set_message_status(&self, id: &str, status: MessageStatus) -> Result<Message> {
        self.check_owner("messages", id)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET status = ?2, \
             delivered_at = CASE WHEN ?2 = 'delivered' AND delivered_at IS NULL THEN ?3 ELSE delivered_at END \
             WHERE id = ?1",
            params![id, status.as_str(), now()],
        )?;
        get_message_conn(&conn, id)?.ok_or_else(|| crate::Refusal::MessageNotFound(id.to_string()).into())
    }

    pub fn mark_turn_ended(&self, id: &str) -> Result<()> {
        self.conn
            .lock()
            .unwrap()
            .execute("UPDATE messages SET turn_ended_at = ?2 WHERE id = ?1", params![id, now()])?;
        Ok(())
    }

    pub fn turn_has_ended(&self, id: &str) -> Result<bool> {
        self.check_owner("messages", id)?;
        Ok(self.conn.lock().unwrap().query_row(
            "SELECT turn_ended_at IS NOT NULL FROM messages WHERE id = ?1",
            [id],
            |r| r.get(0),
        )?)
    }

    pub fn mark_delivered(&self, id: &str) -> Result<Option<Message>> {
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(
            "UPDATE messages SET status = 'delivered', delivered_at = COALESCE(delivered_at, ?2) \
             WHERE id = ?1 AND status = 'queued'",
            params![id, now()],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        Ok(get_message_conn(&conn, id)?)
    }

    pub fn cancel_queued(&self, id: &str) -> Result<Option<Message>> {
        let conn = self.conn.lock().unwrap();
        let changed =
            conn.execute("UPDATE messages SET status = 'cancelled' WHERE id = ?1 AND status = 'queued'", params![id])?;
        if changed == 0 {
            return Ok(None);
        }
        Ok(get_message_conn(&conn, id)?)
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        if key == "project_folder" || key == "sonata_master" {
            return self.project_setting(key);
        }
        Ok(self
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        if key == "project_folder" || key == "sonata_master" {
            return self.set_project_setting(key, Some(value));
        }
        Ok(set_setting_conn(&self.conn.lock().unwrap(), key, value)?)
    }

    pub fn delete_setting(&self, key: &str) -> Result<()> {
        if key == "project_folder" || key == "sonata_master" {
            return self.set_project_setting(key, None);
        }
        self.conn.lock().unwrap().execute("DELETE FROM settings WHERE key = ?1", params![key])?;
        Ok(())
    }

    pub fn insert_skill(&self, slug: &str, name: &str, description: &str, source: SkillSource) -> Result<Skill> {
        insert_skill_conn(&self.conn.lock().unwrap(), slug, name, description, source)
    }

    pub fn get_skill(&self, id: &str) -> Result<Option<Skill>> {
        Ok(self
            .conn
            .lock()
            .unwrap()
            .query_row(&format!("SELECT {SKILL_COLUMNS} FROM skills WHERE id = ?1"), params![id], skill_from_row)
            .optional()?)
    }

    pub fn get_skill_by_slug(&self, slug: &str) -> Result<Option<Skill>> {
        Ok(self
            .conn
            .lock()
            .unwrap()
            .query_row(&format!("SELECT {SKILL_COLUMNS} FROM skills WHERE slug = ?1"), params![slug], skill_from_row)
            .optional()?)
    }

    pub fn skill_slugs(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT slug FROM skills ORDER BY slug")?;
        let slugs = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(slugs)
    }

    pub fn list_skills(&self) -> Result<Vec<SkillView>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare(&format!("SELECT {SKILL_COLUMNS} FROM skills ORDER BY name COLLATE NOCASE, slug"))?;
        let skills = stmt.query_map([], skill_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut views = Vec::with_capacity(skills.len());
        for skill in skills {
            let used_by = bots_using_conn(&conn, &skill.id)?;
            views.push(SkillView { skill, used_by });
        }
        Ok(views)
    }

    pub fn skills_of(&self, bot_id: &str) -> Result<Vec<Skill>> {
        self.check_owner("bots", bot_id)?;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM skills s JOIN bot_skills b ON b.skill_id = s.id WHERE b.bot_id = ?1 ORDER BY s.slug",
            SKILL_COLUMNS.split(", ").map(|c| format!("s.{c}")).collect::<Vec<_>>().join(", ")
        ))?;
        let skills = stmt.query_map(params![bot_id], skill_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(skills)
    }

    pub fn bots_using_skill(&self, id: &str) -> Result<Vec<String>> {
        Ok(bots_using_conn(&self.conn.lock().unwrap(), id)?)
    }

    pub fn update_skill(&self, id: &str, name: &str, description: &str) -> Result<Option<Skill>> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE skills SET name = ?2, description = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, name, description, now()],
        )?;
        Ok(conn
            .query_row(&format!("SELECT {SKILL_COLUMNS} FROM skills WHERE id = ?1"), params![id], skill_from_row)
            .optional()?)
    }

    pub fn bump_bots_with_skill(&self, id: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE bots SET config_version = config_version + 1, updated_at = ?2 \
             WHERE id IN (SELECT bot_id FROM bot_skills WHERE skill_id = ?1)",
            params![id, now()],
        )?;
        Ok(bots_using_conn(&conn, id)?)
    }

    pub fn delete_skill(&self, id: &str) -> Result<Option<Skill>> {
        let conn = self.conn.lock().unwrap();
        let skill = conn
            .query_row(&format!("SELECT {SKILL_COLUMNS} FROM skills WHERE id = ?1"), params![id], skill_from_row)
            .optional()?;
        if skill.is_some() {
            conn.execute("DELETE FROM skills WHERE id = ?1", params![id])?;
        }
        Ok(skill)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn new_bot(name: &str) -> NewBot {
        NewBot {
            display_name: name.into(),
            color: "#3b82f6".into(),
            provider: Provider::Claude,
            model: None,
            instructions: format!("You are {name}."),
            skill_ids: vec![],
        }
    }

    fn apply_first(conn: &Connection, n: usize) {
        for migration in &MIGRATIONS[..n] {
            migration.apply(conn).unwrap();
        }
        conn.pragma_update(None, "user_version", n as i64).unwrap();
    }

    #[test]
    fn onboarding_is_automatic_only_for_installations_without_completed_setup() {
        for (setup, expected) in [(true, "false"), (false, "true")] {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join("previous.sqlite");
            let conn = Connection::open(&path).unwrap();
            // Schema immediately before the onboarding migration, independent of later additions.
            apply_first(&conn, 13);
            if setup {
                set_setting_conn(&conn, "setup_done", "true").unwrap();
            }
            drop(conn);
            let store = Store::open(&path).unwrap();
            assert_eq!(store.get_setting("onboarding_pending").unwrap().as_deref(), Some(expected));
        }
    }

    fn session(s: &Store, bot: &Bot) -> Session {
        s.create_session(&bot.id, None, None).unwrap()
    }

    fn user_msg(to: &Session, body: &str) -> NewMessage {
        NewMessage { recipient_session: Some(to.id.clone()), ..to_bot(&to.bot_id, body) }
    }

    fn to_bot(bot_id: &str, body: &str) -> NewMessage {
        NewMessage {
            sender: crate::model::USER.into(),
            recipient: bot_id.into(),
            sender_session: None,
            recipient_session: None,
            kind: MessageKind::Text,
            body: body.into(),
            attachments: vec![],
            system_code: None,
            system_params: None,
            status: MessageStatus::Queued,
            chain_id: crate::ids::new_id(),
            hop: 0,
            in_reply_to: None,
        }
    }

    fn between(from: &Session, to: &Session, body: &str) -> NewMessage {
        NewMessage { sender: from.bot_id.clone(), sender_session: Some(from.id.clone()), ..user_msg(to, body) }
    }

    fn question_from(w: &Session) -> NewMessage {
        NewMessage {
            sender: w.bot_id.clone(),
            sender_session: Some(w.id.clone()),
            recipient: crate::model::USER.into(),
            recipient_session: None,
            ..to_bot(&w.bot_id, "which one?")
        }
    }

    fn notice_in(asker: &Session) -> NewMessage {
        NewMessage::system(asker, "session_waiting", json!({}))
    }

    fn result_from(w: &Session, asker: &Session) -> NewMessage {
        NewMessage { status: MessageStatus::Delivered, ..between(w, asker, "done") }
    }

    fn asked(s: &Store) -> (Session, Session, Message) {
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let b = s.insert_bot(&new_bot("B"), "b").unwrap();
        let (asker, w) = (session(s, &a), session(s, &b));
        let r =
            s.insert_message(&NewMessage { status: MessageStatus::Delivered, ..between(&asker, &w, "do it") }).unwrap();
        (asker, w, r)
    }

    #[test]
    fn migration_7_starts_every_session_unmarked() {
        let s = Store::open_in_memory().unwrap();
        let version: i64 = s.conn.lock().unwrap().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
        let bot = s.insert_bot(&new_bot("A"), "a").unwrap();
        let sess = session(&s, &bot);
        assert_eq!((sess.waiting_question_id, sess.waiting_since, sess.open_request_id), (None, None, None));
    }

    #[test]
    fn a_question_marks_its_session_opens_the_request_and_marks_the_answer() {
        let s = Store::open_in_memory().unwrap();
        let (asker, w, r) = asked(&s);
        let q = s
            .insert_question("q1", &question_from(&w), &[], Some(&r.id), Some(&notice_in(&asker)), Some(&r.id))
            .unwrap();
        assert_eq!(q.session.waiting_question_id.as_deref(), Some("q1"));
        assert!(q.session.waiting_since.is_some());
        assert_eq!(q.session.open_request_id.as_deref(), Some(r.id.as_str()));
        assert_eq!(q.notice.unwrap().system_code.as_deref(), Some("session_waiting"));
        assert_eq!(s.answered_by(&r.id).unwrap().as_deref(), Some("q1"));
    }

    #[test]
    fn a_question_marks_its_session_with_its_options() {
        let s = Store::open_in_memory().unwrap();
        let (_asker, w, r) = asked(&s);
        let options = ["Approve".to_string(), "Change the plan".to_string()];
        let q = s.insert_question("q1", &question_from(&w), &options, Some(&r.id), None, None).unwrap();
        assert_eq!(q.session.waiting_options, ["Approve", "Change the plan"]);
        assert_eq!(s.get_session(&w.id).unwrap().unwrap().waiting_options, ["Approve", "Change the plan"]);
    }

    #[test]
    fn the_answer_and_archiving_clear_the_options() {
        let s = Store::open_in_memory().unwrap();
        let (_asker, w, r) = asked(&s);
        let options = ["Yes".to_string(), "No".to_string()];
        let options_of = |s: &Store| s.get_session(&w.id).unwrap().unwrap().waiting_options;
        s.insert_question("q1", &question_from(&w), &options, Some(&r.id), None, None).unwrap();
        assert_eq!(options_of(&s), ["Yes", "No"]);
        s.insert_message(&user_msg(&w, "Yes")).unwrap();
        assert!(options_of(&s).is_empty());
        s.insert_question("q2", &question_from(&w), &options, Some(&r.id), None, None).unwrap();
        assert_eq!(options_of(&s), ["Yes", "No"]);
        s.archive_session(&w.id, |_| Ok(())).unwrap().unwrap();
        assert!(options_of(&s).is_empty());
    }

    #[test]
    fn a_newer_question_replaces_the_options() {
        let s = Store::open_in_memory().unwrap();
        let (_asker, w, r) = asked(&s);
        let ab = ["A".to_string(), "B".to_string()];
        s.insert_question("q1", &question_from(&w), &ab, Some(&r.id), None, None).unwrap();
        let q = s.insert_question("q2", &question_from(&w), &[], Some(&r.id), None, None).unwrap();
        assert!(q.session.waiting_options.is_empty());
        assert!(s.get_session(&w.id).unwrap().unwrap().waiting_options.is_empty());
    }

    #[test]
    fn migration_10_leaves_every_session_without_options() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("db.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            apply_first(&conn, 9);
            conn.execute_batch(
                "INSERT INTO bots (id, handle, display_name, color, provider, instructions, created_at, updated_at) \
                   VALUES ('b1', 'a', 'A', '#000', 'claude', '', 't', 't');
                 INSERT INTO sessions (id, bot_id, number, created_at, last_activity_at) \
                   VALUES ('s1', 'b1', 1, 't', 't');",
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert!(s.get_session("s1").unwrap().unwrap().waiting_options.is_empty());
    }

    #[test]
    fn a_question_is_all_or_nothing() {
        let s = Store::open_in_memory().unwrap();
        let (asker, w, r) = asked(&s);
        let bad_notice = NewMessage { sender_session: Some("gone".into()), ..notice_in(&asker) };
        assert!(s.insert_question("q1", &question_from(&w), &[], Some(&r.id), Some(&bad_notice), Some(&r.id)).is_err());
        assert!(s.get_message("q1").unwrap().is_none());
        let w = s.get_session(&w.id).unwrap().unwrap();
        assert_eq!((w.waiting_question_id, w.open_request_id), (None, None));
        assert_eq!(s.answered_by(&r.id).unwrap(), None);
    }

    #[test]
    fn the_result_closes_the_open_request_and_marks_the_users_message() {
        let s = Store::open_in_memory().unwrap();
        let (asker, w, r) = asked(&s);
        s.insert_question("q1", &question_from(&w), &[], Some(&r.id), None, Some(&r.id)).unwrap();
        let users_answer =
            s.insert_message(&NewMessage { status: MessageStatus::Delivered, ..user_msg(&w, "the 2nd") }).unwrap();
        let (msg, w) = s.insert_result("res", &result_from(&w, &asker), Some(&users_answer.id)).unwrap();
        assert_eq!(msg.recipient_session.as_deref(), Some(asker.id.as_str()));
        assert_eq!(w.open_request_id, None);
        assert_eq!(s.answered_by(&users_answer.id).unwrap().as_deref(), Some("res"));
    }

    #[test]
    fn only_the_user_or_the_requester_resolves_a_question_and_only_after_it() {
        let s = Store::open_in_memory().unwrap();
        let (asker, w, r) = asked(&s);
        let c = s.insert_bot(&new_bot("C"), "c").unwrap();
        let third = session(&s, &c);
        let waiting = |s: &Store| s.get_session(&w.id).unwrap().unwrap().waiting_question_id;
        s.insert_message(&user_msg(&w, "early")).unwrap();
        s.insert_question("q1", &question_from(&w), &[], Some(&r.id), None, None).unwrap();
        assert_eq!(waiting(&s).as_deref(), Some("q1"));
        s.insert_message(&between(&third, &w, "hi")).unwrap();
        s.insert_message(&NewMessage::system(&w, "bot_blocked", json!({}))).unwrap();
        assert_eq!(waiting(&s).as_deref(), Some("q1"));
        s.insert_message(&user_msg(&w, "the answer")).unwrap();
        assert_eq!((waiting(&s), s.get_session(&w.id).unwrap().unwrap().waiting_since), (None, None));
        s.insert_question("q2", &question_from(&w), &[], Some(&r.id), None, None).unwrap();
        s.insert_message(&between(&asker, &w, "never mind")).unwrap();
        assert_eq!(waiting(&s), None);
    }

    #[test]
    fn an_answer_from_the_requester_leaves_the_question_and_a_fresh_message_resolves_it() {
        let s = Store::open_in_memory().unwrap();
        let (asker, w, r) = asked(&s);
        let waiting = |s: &Store| s.get_session(&w.id).unwrap().unwrap().waiting_question_id;
        s.insert_question("q1", &question_from(&w), &[], Some(&r.id), None, Some(&r.id)).unwrap();
        let note = s.insert_message(&between(&w, &asker, "halfway")).unwrap();
        s.insert_message(&NewMessage { in_reply_to: Some(note.id), ..between(&asker, &w, "noted") }).unwrap();
        assert_eq!(waiting(&s).as_deref(), Some("q1"), "an answer is no answer to the user's question");
        s.insert_message(&between(&asker, &w, "the user says yes")).unwrap();
        assert_eq!(waiting(&s), None);
    }

    #[test]
    fn archiving_clears_the_question_and_the_open_request() {
        let s = Store::open_in_memory().unwrap();
        let (_asker, w, r) = asked(&s);
        s.insert_question("q1", &question_from(&w), &[], Some(&r.id), None, None).unwrap();
        s.archive_session(&w.id, |_| Ok(())).unwrap().unwrap();
        let w = s.get_session(&w.id).unwrap().unwrap();
        assert!(w.archived_at.is_some());
        assert_eq!((w.waiting_question_id, w.waiting_since, w.open_request_id), (None, None, None));
    }

    #[test]
    fn an_answered_message_is_not_an_unanswered_delivery() {
        let s = Store::open_in_memory().unwrap();
        let bot = s.insert_bot(&new_bot("A"), "a").unwrap();
        let one = session(&s, &bot);
        let m = s.insert_message(&NewMessage { status: MessageStatus::Delivered, ..user_msg(&one, "hi") }).unwrap();
        assert_eq!(s.unanswered_deliveries().unwrap().len(), 1);
        s.conn.lock().unwrap().execute("UPDATE messages SET answered_by = 'x' WHERE id = ?1", params![m.id]).unwrap();
        assert!(s.unanswered_deliveries().unwrap().is_empty());
    }

    #[test]
    fn chain_has_party_finds_senders_and_recipients() {
        let s = Store::open_in_memory().unwrap();
        let (asker, w, r) = asked(&s);
        let c = s.insert_bot(&new_bot("C"), "c").unwrap();
        assert!(s.chain_has_party(&r.chain_id, &asker.bot_id).unwrap());
        assert!(s.chain_has_party(&r.chain_id, &w.bot_id).unwrap());
        assert!(!s.chain_has_party(&r.chain_id, &c.id).unwrap());
    }

    #[test]
    fn a_bot_recorded_as_a_setting_is_one_write() {
        let s = Store::open_in_memory().unwrap();
        let bot = s.insert_bot_recorded_as(&new_bot("Master"), "master", "sonata_master").unwrap();
        assert_eq!(s.get_setting("sonata_master").unwrap(), Some(bot.id));
        assert!(s.insert_bot_recorded_as(&new_bot("Again"), "master", "sonata_other").is_err());
        assert_eq!(s.get_setting("sonata_other").unwrap(), None);
        s.delete_setting("sonata_master").unwrap();
        assert_eq!(s.get_setting("sonata_master").unwrap(), None);
    }

    #[test]
    fn creation_records_round_trip_and_are_replaced() {
        let s = Store::open_in_memory().unwrap();
        let host = s.insert_bot(&new_bot("Host"), "host").unwrap();
        let sess = session(&s, &host);
        let rec = NewCreation { session_id: &sess.id, kind: CreationKind::Bot, key: "seo", fingerprint: "f1" };
        let bot = s.insert_bot_recorded(&new_bot("SEO"), "seo", &rec).unwrap();
        assert_eq!(
            s.creation(&sess.id, CreationKind::Bot, "seo").unwrap(),
            Some(Creation { fingerprint: "f1".into(), result_id: bot.id.clone(), result_name: "seo".into() })
        );
        let skill = s
            .insert_skill_recorded(
                "sonata-seo",
                "sonata-seo",
                "SEO.",
                SkillSource::Imported,
                &NewCreation {
                    kind: CreationKind::Skill,
                    key: "skill-drafts/session-1/sonata-seo",
                    fingerprint: "f2",
                    ..rec
                },
            )
            .unwrap();
        let kept = s.creation(&sess.id, CreationKind::Skill, "skill-drafts/session-1/sonata-seo").unwrap().unwrap();
        assert_eq!((kept.result_id, kept.result_name), (skill.id, "sonata-seo".to_string()));
        let again = s.insert_bot_recorded(&new_bot("SEO"), "seo-2", &NewCreation { fingerprint: "f3", ..rec }).unwrap();
        let replaced = s.creation(&sess.id, CreationKind::Bot, "seo").unwrap().unwrap();
        assert_eq!((replaced.result_id, replaced.result_name), (again.id, "seo-2".to_string()));
    }

    #[test]
    fn bots_round_trip_and_list_in_creation_order() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let b = s.insert_bot(&new_bot("B"), "b").unwrap();
        assert_eq!(s.get_bot(&a.id).unwrap(), Some(a.clone()));
        assert_eq!(s.list_bots().unwrap(), vec![a, b]);
        assert_eq!(s.bot_handles().unwrap(), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(s.get_bot("missing").unwrap(), None);
    }

    #[test]
    fn duplicate_handle_is_rejected() {
        let s = Store::open_in_memory().unwrap();
        s.insert_bot(&new_bot("A"), "a").unwrap();
        assert!(s.insert_bot(&new_bot("A again"), "a").is_err());
    }

    fn cli_ids(s: &Store, bot_id: &str) -> Vec<(i64, Option<String>, Option<i64>)> {
        s.sessions_of(bot_id)
            .unwrap()
            .into_iter()
            .map(|session| (session.number, session.provider_session_id, session.session_config_version))
            .collect()
    }

    fn active_at(s: &Store, session_id: &str, at: &str) {
        s.conn
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET last_activity_at = ?2 WHERE id = ?1", params![session_id, at])
            .unwrap();
    }

    #[test]
    fn a_session_is_saved_with_the_config_version_it_has_seen() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let one = s.create_session(&a.id, None, None).unwrap();
        assert_eq!((one.provider_session_id.as_deref(), one.session_config_version), (None, None));
        s.set_session_cli_id(&one.id, Some("sess-1"), 1, Provider::Claude).unwrap();
        let saved = s.get_session(&one.id).unwrap().unwrap();
        assert_eq!((saved.provider_session_id.as_deref(), saved.session_config_version), (Some("sess-1"), Some(1)));
        s.set_session_cli_id(&one.id, None, 1, Provider::Claude).unwrap();
        let cleared = s.get_session(&one.id).unwrap().unwrap();
        assert_eq!((cleared.provider_session_id, cleared.session_config_version), (None, None));
    }

    #[test]
    fn a_session_id_is_filled_only_when_none_is_saved() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let one = s.create_session(&a.id, None, None).unwrap();
        assert!(s.fill_session_cli_id(&one.id, "first", 1, Provider::Claude).unwrap());
        assert!(!s.fill_session_cli_id(&one.id, "second", 2, Provider::Claude).unwrap());
        let saved = s.get_session(&one.id).unwrap().unwrap();
        assert_eq!((saved.provider_session_id.as_deref(), saved.session_config_version), (Some("first"), Some(1)));
        let two = s.create_session(&a.id, None, None).unwrap();
        assert!(!s.fill_session_cli_id(&two.id, "x", 1, Provider::Grok).unwrap(), "another provider");
        assert_eq!(s.get_session(&two.id).unwrap().unwrap().provider_session_id, None);
    }

    #[test]
    fn a_session_of_another_provider_is_not_saved() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let one = s.create_session(&a.id, None, None).unwrap();
        s.set_session_cli_id(&one.id, Some("sess-1"), 1, Provider::Grok).unwrap();
        let session = s.get_session(&one.id).unwrap().unwrap();
        assert_eq!((session.provider_session_id, session.session_config_version), (None, None));
    }

    #[test]
    fn edits_bump_the_config_version_only_for_what_the_cli_sees() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let one = s.create_session(&a.id, None, None).unwrap();
        let two = s.create_session(&a.id, None, None).unwrap();
        s.set_session_cli_id(&one.id, Some("sess-1"), 1, Provider::Claude).unwrap();
        s.set_session_cli_id(&two.id, Some("sess-2"), 1, Provider::Claude).unwrap();
        let recolored = s.update_bot(&a.id, &NewBot { color: "#000000".into(), ..new_bot("A") }).unwrap().unwrap();
        assert_eq!((recolored.color.as_str(), recolored.config_version), ("#000000", 1));
        let renamed = s.update_bot(&a.id, &NewBot { display_name: "Alpha".into(), ..new_bot("A") }).unwrap().unwrap();
        assert_eq!((renamed.display_name.as_str(), renamed.handle.as_str(), renamed.config_version), ("Alpha", "a", 2));
        assert_eq!(
            cli_ids(&s, &a.id),
            vec![(2, Some("sess-2".into()), Some(1)), (1, Some("sess-1".into()), Some(1))],
            "same CLI, same sessions"
        );
        let switched = s.update_bot(&a.id, &NewBot { provider: Provider::Grok, ..new_bot("Alpha") }).unwrap().unwrap();
        assert_eq!((switched.provider, switched.config_version), (Provider::Grok, 3));
        assert_eq!(cli_ids(&s, &a.id), vec![(2, None, None), (1, None, None)], "the old CLI's sessions are dropped");
        assert_eq!(s.update_bot("missing", &new_bot("X")).unwrap(), None);
    }

    #[test]
    fn sessions_are_numbered_per_bot() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let b = s.insert_bot(&new_bot("B"), "b").unwrap();
        let a1 = s.create_session(&a.id, None, None).unwrap();
        let b1 = s.create_session(&b.id, Some("Review the plan"), Some(&a1.id)).unwrap();
        let a2 = s.create_session(&a.id, None, None).unwrap();
        assert_eq!((a1.number, a2.number, b1.number), (1, 2, 1));
        assert_eq!((b1.title.as_deref(), b1.opened_by.as_deref()), (Some("Review the plan"), Some(a1.id.as_str())));
        assert_eq!((a1.title, a1.opened_by), (None, None));
        assert_eq!(a2.created_at, a2.last_activity_at);
        assert_eq!(s.get_session(&b1.id).unwrap(), Some(b1));
        assert_eq!(s.get_session("missing").unwrap(), None);
        assert!(s.create_session("missing", None, None).is_err(), "a session needs its bot");
    }

    #[test]
    fn sessions_list_the_most_recently_active_first_and_the_newer_on_a_tie() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        assert_eq!(s.latest_session(&a.id).unwrap(), None);
        let [one, two, three] = [(); 3].map(|()| s.create_session(&a.id, None, None).unwrap());
        active_at(&s, &one.id, "2026-10-02T00:00:00.000Z");
        active_at(&s, &two.id, "2026-10-01T00:00:00.000Z");
        active_at(&s, &three.id, "2026-10-02T00:00:00.000Z");
        let numbers = |s: &Store| s.sessions_of(&a.id).unwrap().into_iter().map(|x| x.number).collect::<Vec<_>>();
        assert_eq!(numbers(&s), vec![3, 1, 2]);
        assert_eq!(s.latest_session(&a.id).unwrap().map(|x| x.number), Some(3));

        s.touch_session(&two.id).unwrap();
        assert_eq!(numbers(&s), vec![2, 3, 1], "touched now");
    }

    #[test]
    fn a_message_of_a_bot_that_exists_names_its_session() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let b = s.insert_bot(&new_bot("B"), "b").unwrap();
        let (a1, b1) = (session(&s, &a), session(&s, &b));
        let missing = [
            NewMessage { recipient_session: None, ..user_msg(&a1, "to a") },
            NewMessage { sender_session: None, ..between(&a1, &b1, "from a") },
            NewMessage { sender_session: None, ..NewMessage::system(&a1, "bot_blocked", json!({})) },
        ];
        for m in &missing {
            let err = s.insert_message(m).unwrap_err();
            assert!(matches!(err, crate::CoreError::Internal(_)), "{err}");
        }
        let stored: i64 = s.conn.lock().unwrap().query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0)).unwrap();
        assert_eq!(stored, 0);
        assert_eq!((s.sessions_of(&a.id).unwrap().len(), s.sessions_of(&b.id).unwrap().len()), (1, 1));

        let to_a = s.insert_message(&user_msg(&a1, "hi")).unwrap();
        assert_eq!(to_a.thread_id, s.thread_for(USER, &a1.id).unwrap());
        let notice = s.insert_message(&NewMessage::system(&a1, "bot_blocked", json!({}))).unwrap();
        assert_eq!(
            (notice.sender_session.as_deref(), notice.recipient_session.as_deref(), &notice.thread_id),
            (Some(a1.id.as_str()), None, &to_a.thread_id)
        );
        let to_b = s.insert_message(&between(&a1, &b1, "hi b")).unwrap();
        assert_eq!(to_b.thread_id, s.thread_for(&b1.id, &a1.id).unwrap());
        let late = s.insert_message(&NewMessage { sender: "gone".into(), ..user_msg(&b1, "late") }).unwrap();
        assert_eq!((late.sender_session, late.thread_id), (None, s.thread_for("gone", &b1.id).unwrap()));
    }

    #[test]
    fn a_routed_message_finds_or_opens_its_session() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let b = s.insert_bot(&new_bot("B"), "b").unwrap();
        let to_a = |body: &str| to_bot(&a.id, body);

        let first = s.insert_routed(&new_id(), &to_a("first"), Route::Latest { title: Some("First") }).unwrap();
        let a1 = first.session.clone();
        assert_eq!(
            (a1.number, a1.title.as_deref(), a1.opened_by.as_deref(), first.created),
            (1, Some("First"), None, true)
        );
        assert_eq!(first.message.recipient_session.as_deref(), Some(a1.id.as_str()));
        assert_eq!(a1.last_activity_at, first.message.created_at, "the session as it is after the message");
        let new = s.insert_routed(&new_id(), &to_a("second"), Route::New { title: None }).unwrap();
        let a2 = new.session.clone();
        assert_eq!((a2.number, a2.title.as_deref(), new.created), (2, None, true));
        active_at(&s, &a1.id, "2999-01-01T00:00:00.000Z");
        let latest = s.insert_routed(&new_id(), &to_a("third"), Route::Latest { title: Some("Third") }).unwrap();
        assert_eq!(
            (latest.session.id.as_str(), latest.session.title.as_deref(), latest.created),
            (a1.id.as_str(), Some("First"), false)
        );

        let ask = |from: &Session, body: &str| NewMessage {
            sender: from.bot_id.clone(),
            sender_session: Some(from.id.clone()),
            ..to_bot(&b.id, body)
        };
        let q1 = s.insert_routed(&new_id(), &ask(&a1, "q1"), Route::Shared).unwrap();
        assert_eq!((q1.session.number, q1.session.opened_by.as_deref(), q1.created), (1, Some(a1.id.as_str()), true));
        assert_eq!(q1.session.title, None);
        let q2 = s.insert_routed(&new_id(), &ask(&a2, "q2"), Route::Shared).unwrap();
        assert_eq!((q2.session.number, q2.session.opened_by.as_deref(), q2.created), (2, Some(a2.id.as_str()), true));
        let q3 = s.insert_routed(&new_id(), &ask(&a1, "q3"), Route::Shared).unwrap();
        assert_eq!(
            (q3.session.id.as_str(), q3.created, &q3.message.thread_id),
            (q1.session.id.as_str(), false, &q1.message.thread_id)
        );

        let from_user = s.insert_routed(&new_id(), &to_bot(&b.id, "q0"), Route::Shared);
        assert!(matches!(from_user, Err(crate::CoreError::Internal(_))), "only a session shares threads");

        let a3 = session(&s, &a);
        s.thread_for(&a3.id, &q1.session.id).unwrap();
        assert!(s.insert_routed(&new_id(), &ask(&a3, "q4"), Route::Shared).unwrap().created);

        s.insert_message(&between(&q2.session, &a1, "from b5")).unwrap();
        active_at(&s, &q2.session.id, "2999-01-01T00:00:00.000Z");
        let q5 = s.insert_routed(&new_id(), &ask(&a1, "q5"), Route::Shared).unwrap();
        assert_eq!((q5.session.id.as_str(), q5.created), (q2.session.id.as_str(), false));
        active_at(&s, &q1.session.id, "3000-01-01T00:00:00.000Z");
        let q6 = s.insert_routed(&new_id(), &ask(&a1, "q6"), Route::Shared).unwrap();
        assert_eq!((q6.session.id.as_str(), q6.created), (q1.session.id.as_str(), false));
    }

    #[test]
    fn each_session_has_its_own_queue() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let b = s.insert_bot(&new_bot("B"), "b").unwrap();
        let [a1, a2] = [(); 2].map(|()| s.create_session(&a.id, None, None).unwrap());
        let b1 = s.create_session(&b.id, None, None).unwrap();
        s.insert_message(&user_msg(&a2, "1")).unwrap();
        s.insert_message(&user_msg(&b1, "2")).unwrap();
        s.insert_message(&user_msg(&a1, "3")).unwrap();
        s.insert_message(&user_msg(&a2, "4")).unwrap();
        s.insert_message(&NewMessage { status: MessageStatus::Delivered, ..user_msg(&a1, "5") }).unwrap();
        let bodies = |id: &str| s.queued_for_session(id).unwrap().into_iter().map(|m| m.body).collect::<Vec<_>>();
        assert_eq!(
            (bodies(&a1.id), bodies(&a2.id), bodies(&b1.id)),
            (vec!["3".into()], vec!["1".into(), "4".into()], vec!["2".into()])
        );
        assert_eq!(s.sessions_with_queued().unwrap(), vec![a2.id, b1.id, a1.id]);
    }

    #[test]
    fn deleting_a_bot_deletes_its_sessions_and_unlinks_those_they_opened() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let b = s.insert_bot(&new_bot("B"), "b").unwrap();
        let a1 = s.create_session(&a.id, None, None).unwrap();
        let b1 = s.create_session(&b.id, None, Some(&a1.id)).unwrap();
        let hello = s.insert_message(&user_msg(&a1, "hello")).unwrap();
        s.delete_bot(&a.id).unwrap().unwrap();
        assert_eq!(s.get_session(&a1.id).unwrap(), None);
        assert_eq!(s.get_session(&b1.id).unwrap().unwrap().opened_by, None);
        assert_eq!(s.get_message(&hello.id).unwrap().unwrap().recipient_session, Some(a1.id), "the message stays");
    }

    #[test]
    fn removing_a_session_takes_the_sessions_it_opened_with_their_threads_and_messages() {
        let s = Store::open_in_memory().unwrap();
        let [a, b, c] = ["A", "B", "C"].map(|n| s.insert_bot(&new_bot(n), &n.to_lowercase()).unwrap());
        let a1 = session(&s, &a);
        let b1 = s.create_session(&b.id, None, Some(&a1.id)).unwrap();
        let c1 = s.create_session(&c.id, None, Some(&b1.id)).unwrap();
        let b2 = session(&s, &b);
        let kept = s.insert_message(&user_msg(&b2, "kept")).unwrap();
        let gone = [
            s.insert_message(&user_msg(&a1, "1")).unwrap(),
            s.insert_message(&between(&a1, &b1, "2")).unwrap(),
            s.insert_message(&between(&b1, &c1, "3")).unwrap(),
            s.insert_message(&between(&b2, &c1, "4")).unwrap(),
        ];
        let empty = s.thread_for(USER, &c1.id).unwrap();
        let thread = |i: usize| gone[i].thread_id.clone();
        let exists = |table: &str, id: &str| -> bool {
            let sql = format!("SELECT COUNT(*) FROM {table} WHERE id = ?1");
            s.conn.lock().unwrap().query_row(&sql, params![id], |r| r.get::<_, i64>(0)).unwrap() == 1
        };

        let refused = s.remove_session(&a1.id, |sessions| {
            let ids: Vec<&str> = sessions.iter().map(|x| x.id.as_str()).collect();
            assert_eq!(ids, vec![a1.id.as_str(), b1.id.as_str(), c1.id.as_str()], "each after its opener");
            Err(crate::Refusal::SessionStarting.into())
        });
        assert!(matches!(refused, Err(CoreError::Refused(crate::Refusal::SessionStarting))), "{refused:?}");
        assert!(exists("sessions", &c1.id) && exists("threads", &empty) && exists("messages", &gone[3].id));

        let removed = s.remove_session(&a1.id, |_| Ok(())).unwrap().unwrap();
        let sorted = |mut ids: Vec<String>| {
            ids.sort();
            ids
        };
        let per_session: Vec<(String, Vec<String>)> =
            removed.sessions.iter().map(|r| (r.session.id.clone(), sorted(r.thread_ids.clone()))).collect();
        assert_eq!(
            per_session,
            vec![
                (a1.id.clone(), sorted(vec![thread(0), thread(1)])),
                (b1.id.clone(), sorted(vec![thread(1), thread(2)])),
                (c1.id.clone(), sorted(vec![thread(2), thread(3), empty.clone()])),
            ]
        );
        assert_eq!(sorted(removed.thread_ids.clone()), sorted(vec![thread(0), thread(1), thread(2), thread(3), empty]));
        let messages: Vec<&str> = removed.messages.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(messages, gone.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), "oldest first");
        for id in [&a1.id, &b1.id, &c1.id] {
            assert!(!exists("sessions", id));
        }
        for id in &removed.thread_ids {
            assert!(!exists("threads", id));
        }
        for m in &gone {
            assert!(!exists("messages", &m.id));
        }
        assert!(exists("sessions", &b2.id) && exists("messages", &kept.id), "what is not theirs stays");
        assert_eq!(s.remove_session(&a1.id, |_| Ok(())).unwrap(), None, "gone already");

        let late = s.insert_message(&between(&b2, &b1, "late"));
        assert!(matches!(late, Err(CoreError::Refused(crate::Refusal::SessionNotFound(_)))), "{late:?}");
        let late = s.insert_message(&between(&c1, &b2, "late"));
        assert!(matches!(late, Err(CoreError::Refused(crate::Refusal::SessionNotFound(_)))), "{late:?}");
        let to_a = NewMessage { recipient: a.id.clone(), recipient_session: None, ..between(&c1, &b2, "late") };
        let late = s.insert_routed(&new_id(), &to_a, Route::Shared);
        assert!(matches!(late, Err(CoreError::Refused(crate::Refusal::SessionNotFound(_)))), "{late:?}");
        assert!(s.sessions_of(&a.id).unwrap().is_empty(), "it opened no session");
        assert!(s.bot_threads().unwrap().is_empty());

        let unused = session(&s, &a);
        let removed = s.remove_session(&unused.id, |_| Ok(())).unwrap().unwrap();
        assert!(removed.thread_ids.is_empty() && removed.messages.is_empty());
        assert!(!exists("sessions", &unused.id));
    }

    fn archived(s: &Store, id: &str) -> Option<String> {
        s.get_session(id).unwrap().unwrap().archived_at
    }

    fn archived_on(s: &Store, id: &str, at: &str) {
        s.conn.lock().unwrap().execute("UPDATE sessions SET archived_at = ?2 WHERE id = ?1", params![id, at]).unwrap();
    }

    fn ids(sessions: &[Session]) -> Vec<&str> {
        sessions.iter().map(|x| x.id.as_str()).collect()
    }

    #[test]
    fn a_version_5_database_gains_archived_at_and_keeps_its_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("db.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            apply_first(&conn, 5);
            conn.execute_batch(
                "INSERT INTO bots (id, handle, display_name, color, provider, model, instructions, config_version, \
                   created_at, updated_at) VALUES ('ba', 'a', 'A', '#000', 'claude', NULL, '', 1, 't', 't');
                 INSERT INTO sessions (id, bot_id, number, title, provider_session_id, session_config_version, \
                   opened_by, created_at, last_activity_at) VALUES ('s1', 'ba', 1, 'Plan', 'sess-1', 1, NULL, 't', 't');",
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        let version: i64 = s.conn.lock().unwrap().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
        let one = s.get_session("s1").unwrap().unwrap();
        assert_eq!(
            (one.title.as_deref(), one.provider_session_id.as_deref(), one.archived_at),
            (Some("Plan"), Some("sess-1"), None)
        );
        assert!(!v4_backup_path(&path).exists(), "an added column needs no backup");
    }

    #[test]
    fn archiving_marks_the_session_and_those_it_opened_and_keeps_an_earlier_archive() {
        let s = Store::open_in_memory().unwrap();
        let [a, b, c] = ["A", "B", "C"].map(|n| s.insert_bot(&new_bot(n), &n.to_lowercase()).unwrap());
        let a1 = session(&s, &a);
        let b1 = s.create_session(&b.id, None, Some(&a1.id)).unwrap();
        let c1 = s.create_session(&c.id, None, Some(&b1.id)).unwrap();
        let b2 = session(&s, &b);

        let alone = s.archive_session(&c1.id, |_| Ok(())).unwrap().unwrap();
        assert_eq!(ids(&alone), vec![c1.id.as_str()]);
        assert_eq!(alone[0].archived_at, archived(&s, &c1.id));
        archived_on(&s, &c1.id, "2026-10-01T00:00:00.000Z");

        let archived_now = s
            .archive_session(&a1.id, |tree| {
                assert_eq!(ids(tree), vec![a1.id.as_str(), b1.id.as_str(), c1.id.as_str()], "each after its opener");
                Ok(())
            })
            .unwrap()
            .unwrap();
        assert_eq!(ids(&archived_now), vec![a1.id.as_str(), b1.id.as_str()], "c1 was archived already");
        let stamp = archived(&s, &a1.id);
        assert!(stamp.is_some());
        assert_eq!(archived(&s, &b1.id), stamp, "one mark for the archive");
        assert!(archived_now.iter().all(|x| x.archived_at == stamp), "returned as stored");
        assert_eq!(archived(&s, &c1.id).as_deref(), Some("2026-10-01T00:00:00.000Z"), "its own archive stays");
        assert_eq!(archived(&s, &b2.id), None, "not opened by a1");
        assert_eq!(s.sessions_of(&b.id).unwrap().len(), 2, "archived sessions are still listed");

        assert!(s.archive_session(&a1.id, |_| Ok(())).unwrap().unwrap().is_empty(), "nothing left to archive");
        assert_eq!(s.archive_session("missing", |_| Ok(())).unwrap(), None);
    }

    #[test]
    fn a_refused_archive_changes_nothing() {
        let s = Store::open_in_memory().unwrap();
        let [a, b] = ["A", "B"].map(|n| s.insert_bot(&new_bot(n), &n.to_lowercase()).unwrap());
        let a1 = session(&s, &a);
        let b1 = s.create_session(&b.id, None, Some(&a1.id)).unwrap();
        let refused = s.archive_session(&a1.id, |_| Err(crate::Refusal::SessionStarting.into()));
        assert!(matches!(refused, Err(CoreError::Refused(crate::Refusal::SessionStarting))), "{refused:?}");
        assert_eq!((archived(&s, &a1.id), archived(&s, &b1.id)), (None, None));
    }

    #[test]
    fn restoring_brings_back_only_the_sessions_archived_with_it() {
        let s = Store::open_in_memory().unwrap();
        let [a, b, c] = ["A", "B", "C"].map(|n| s.insert_bot(&new_bot(n), &n.to_lowercase()).unwrap());
        let a1 = session(&s, &a);
        let b1 = s.create_session(&b.id, None, Some(&a1.id)).unwrap();
        let c1 = s.create_session(&c.id, None, Some(&b1.id)).unwrap();
        s.archive_session(&c1.id, |_| Ok(())).unwrap();
        archived_on(&s, &c1.id, "2026-10-01T00:00:00.000Z");
        s.archive_session(&a1.id, |_| Ok(())).unwrap();

        let restored = s.restore_session(&a1.id).unwrap().unwrap();
        assert_eq!(ids(&restored), vec![a1.id.as_str(), b1.id.as_str()]);
        assert!(restored.iter().all(|x| x.archived_at.is_none()), "returned as stored");
        assert_eq!((archived(&s, &a1.id), archived(&s, &b1.id)), (None, None));
        assert!(archived(&s, &c1.id).is_some(), "archived on its own before, it stays archived");

        assert_eq!(ids(&s.restore_session(&c1.id).unwrap().unwrap()), vec![c1.id.as_str()]);
        assert!(s.restore_session(&a1.id).unwrap().unwrap().is_empty(), "not archived");
        assert_eq!(s.restore_session("missing").unwrap(), None);
    }

    #[test]
    fn an_archived_session_takes_no_message_at_either_end() {
        let s = Store::open_in_memory().unwrap();
        let [a, b] = ["A", "B"].map(|n| s.insert_bot(&new_bot(n), &n.to_lowercase()).unwrap());
        let a1 = session(&s, &a);
        let b1 = session(&s, &b);
        let cut = s.insert_message(&user_msg(&b1, "cut off")).unwrap();
        s.set_message_status(&cut.id, MessageStatus::Interrupted).unwrap();
        s.archive_session(&b1.id, |_| Ok(())).unwrap();

        let is_archived = |r: Result<Message>| matches!(r, Err(CoreError::Refused(crate::Refusal::SessionArchived(_))));
        assert!(is_archived(s.insert_message(&user_msg(&b1, "hi"))), "from the user");
        assert!(is_archived(s.insert_message(&between(&a1, &b1, "ask"))), "to it");
        assert!(is_archived(s.insert_message(&between(&b1, &a1, "answer"))), "from it");
        assert!(is_archived(s.insert_message(&NewMessage::system(&b1, "bot_blocked", json!({})))), "a notice");
        let retried = s.retry_interrupted(&cut.id, &user_msg(&b1, "cut off"));
        assert!(matches!(retried, Err(CoreError::Refused(crate::Refusal::SessionArchived(_)))), "{retried:?}");
        assert_eq!(s.get_message(&cut.id).unwrap().unwrap().status, MessageStatus::Interrupted, "rolled back");
        let to_a = NewMessage { recipient: a.id.clone(), recipient_session: None, ..between(&b1, &a1, "late") };
        let routed = s.insert_routed(&new_id(), &to_a, Route::Shared);
        assert!(matches!(routed, Err(CoreError::Refused(crate::Refusal::SessionArchived(_)))), "{routed:?}");
        assert_eq!(s.sessions_of(&a.id).unwrap().len(), 1, "it opened no session");
    }

    #[test]
    fn routing_skips_archived_sessions() {
        let s = Store::open_in_memory().unwrap();
        let [a, b] = ["A", "B"].map(|n| s.insert_bot(&new_bot(n), &n.to_lowercase()).unwrap());
        let a1 = session(&s, &a);
        let b1 = s.create_session(&b.id, None, Some(&a1.id)).unwrap();
        s.insert_message(&between(&a1, &b1, "first")).unwrap();
        s.archive_session(&b1.id, |_| Ok(())).unwrap();
        let ask = NewMessage { recipient: b.id.clone(), recipient_session: None, ..between(&a1, &b1, "again") };
        let routed = s.insert_routed(&new_id(), &ask, Route::Shared).unwrap();
        assert!(routed.created, "a new session instead of the archived one");
        assert_eq!(routed.session.opened_by.as_deref(), Some(a1.id.as_str()));

        let a2 = session(&s, &a);
        s.archive_session(&a2.id, |_| Ok(())).unwrap();
        assert_eq!(s.latest_session(&a.id).unwrap().map(|x| x.id), Some(a1.id.clone()), "a2 is newer but archived");
        let latest = s.insert_routed(&new_id(), &to_bot(&a.id, "hi"), Route::Latest { title: None }).unwrap();
        assert_eq!((latest.session.id.as_str(), latest.created), (a1.id.as_str(), false));
    }

    #[test]
    fn sessions_with_queued_leave_out_archived_sessions() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let [a1, a2] = [(); 2].map(|()| session(&s, &a));
        s.insert_message(&user_msg(&a1, "1")).unwrap();
        s.insert_message(&user_msg(&a2, "2")).unwrap();
        let cut = s.insert_message(&NewMessage { status: MessageStatus::Delivered, ..user_msg(&a2, "3") }).unwrap();
        s.archive_session(&a2.id, |_| Ok(())).unwrap();
        assert_eq!(s.sessions_with_queued().unwrap(), vec![a1.id]);
        let unanswered: Vec<String> = s.unanswered_deliveries().unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(unanswered, vec![cut.id], "a turn the archive cut off is still found after a crash");
    }

    #[test]
    fn a_session_title_can_be_set_and_cleared() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let a1 = session(&s, &a);
        s.set_session_title(&a1.id, Some("Draft the brief")).unwrap();
        assert_eq!(s.get_session(&a1.id).unwrap().unwrap().title.as_deref(), Some("Draft the brief"));
        s.set_session_title(&a1.id, None).unwrap();
        assert_eq!(s.get_session(&a1.id).unwrap().unwrap().title, None);
    }

    #[test]
    fn schema_3_marks_saved_sessions_as_having_seen_the_current_config() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("db.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            apply_first(&conn, 2);
            conn.execute(
                "INSERT INTO bots (id, handle, display_name, color, provider, model, instructions, \
                 provider_session_id, config_version, created_at, updated_at) VALUES \
                 ('b1', 'a', 'A', '#000', 'grok', NULL, '', 'sess-1', 4, 't', 't'), \
                 ('b2', 'b', 'B', '#000', 'grok', NULL, '', NULL, 1, 't', 't')",
                [],
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(cli_ids(&s, "b1"), vec![(1, Some("sess-1".into()), Some(4))], "kept in Session 1");
        assert_eq!(cli_ids(&s, "b2"), vec![(1, None, None)]);
    }

    #[test]
    fn thread_is_shared_by_a_pair_in_either_order() {
        let s = Store::open_in_memory().unwrap();
        let t1 = s.thread_for(crate::model::USER, "bot1").unwrap();
        let t2 = s.thread_for("bot1", crate::model::USER).unwrap();
        assert_eq!(t1, t2);
        assert_ne!(t1, s.thread_for(crate::model::USER, "bot2").unwrap());
    }

    #[test]
    fn a_sessions_last_delivered_message_is_its_newest_delivered_one() {
        let s = Store::open_in_memory().unwrap();
        let bot = s.insert_bot(&new_bot("A"), "a").unwrap();
        let one = session(&s, &bot);
        let two = session(&s, &bot);
        assert_eq!(s.last_delivered_to_session(&one.id).unwrap(), None);
        let first = s.insert_message(&user_msg(&one, "first")).unwrap();
        let second = s.insert_message(&user_msg(&one, "second")).unwrap();
        let elsewhere = s.insert_message(&user_msg(&two, "elsewhere")).unwrap();
        assert_eq!(s.last_delivered_to_session(&one.id).unwrap(), None, "queued is not delivered");
        s.set_message_status(&first.id, MessageStatus::Delivered).unwrap();
        s.set_message_status(&elsewhere.id, MessageStatus::Delivered).unwrap();
        assert_eq!(s.last_delivered_to_session(&one.id).unwrap().map(|m| m.id), Some(first.id.clone()));
        s.set_message_status(&second.id, MessageStatus::Delivered).unwrap();
        assert_eq!(s.last_delivered_to_session(&one.id).unwrap().map(|m| m.id), Some(second.id.clone()));
        s.set_message_status(&second.id, MessageStatus::Interrupted).unwrap();
        assert_eq!(s.last_delivered_to_session(&one.id).unwrap().map(|m| m.id), Some(first.id));
    }

    #[test]
    fn queue_is_fifo_and_delivery_sets_timestamp() {
        let s = Store::open_in_memory().unwrap();
        let bot = s.insert_bot(&new_bot("A"), "a").unwrap();
        let one = session(&s, &bot);
        let m1 = s.insert_message(&user_msg(&one, "first")).unwrap();
        let m2 = s.insert_message(&user_msg(&one, "second")).unwrap();
        let queued: Vec<String> = s.queued_for_session(&one.id).unwrap().into_iter().map(|m| m.body).collect();
        assert_eq!(queued, vec!["first", "second"]);
        assert_eq!(m1.delivered_at, None);

        let d = s.set_message_status(&m1.id, MessageStatus::Delivered).unwrap();
        assert_eq!(d.status, MessageStatus::Delivered);
        assert!(d.delivered_at.is_some());
        assert_eq!(s.queued_for_session(&one.id).unwrap(), vec![m2.clone()]);

        let thread = s.thread_for(crate::model::USER, &one.id).unwrap();
        let all: Vec<String> = s.thread_messages(&thread).unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(all, vec![m1.id, m2.id]);
    }

    #[test]
    fn unanswered_deliveries_skip_answered_queued_and_user_bound_messages() {
        let s = Store::open_in_memory().unwrap();
        let bot = s.insert_bot(&new_bot("A"), "a").unwrap();
        let one = session(&s, &bot);
        let delivered = |body: &str| NewMessage { status: MessageStatus::Delivered, ..user_msg(&one, body) };
        let answered = s.insert_message(&delivered("answered")).unwrap();
        s.insert_message(&NewMessage {
            sender: bot.id.clone(),
            sender_session: Some(one.id.clone()),
            recipient: crate::model::USER.into(),
            recipient_session: None,
            in_reply_to: Some(answered.id.clone()),
            ..delivered("the answer")
        })
        .unwrap();
        let fell_back = s.insert_message(&delivered("fell back")).unwrap();
        s.insert_message(&NewMessage {
            in_reply_to: Some(fell_back.id.clone()),
            ..NewMessage::system(&one, "no_output_captured", json!({}))
        })
        .unwrap();
        let cut = s.insert_message(&delivered("cut off")).unwrap();
        s.insert_message(&user_msg(&one, "still queued")).unwrap();
        s.insert_message(&NewMessage::system(&one, "bot_blocked", json!({}))).unwrap();

        let ids: Vec<String> = s.unanswered_deliveries().unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![cut.id]);
    }

    #[test]
    fn unanswered_deliveries_are_those_of_sessions_that_still_exist() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let [one, two] = [(); 2].map(|()| s.create_session(&a.id, None, None).unwrap());
        let delivered =
            |session: &Session, body: &str| NewMessage { status: MessageStatus::Delivered, ..user_msg(session, body) };
        let cut = s.insert_message(&delivered(&one, "cut off")).unwrap();
        s.insert_message(&delivered(&two, "of a removed session")).unwrap();
        s.conn.lock().unwrap().execute("DELETE FROM sessions WHERE id = ?1", params![two.id]).unwrap();
        let ids: Vec<String> = s.unanswered_deliveries().unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![cut.id]);
    }

    #[test]
    fn a_message_arriving_makes_its_session_the_most_recently_active() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let [one, two] = [(); 2].map(|()| s.create_session(&a.id, None, None).unwrap());
        active_at(&s, &one.id, "2026-10-01T00:00:00.000Z");
        active_at(&s, &two.id, "2026-10-02T00:00:00.000Z");
        let to_one = s.insert_message(&user_msg(&one, "hi")).unwrap();
        let latest = s.latest_session(&a.id).unwrap().unwrap();
        assert_eq!((latest.id.as_str(), latest.last_activity_at), (one.id.as_str(), to_one.created_at));
        s.insert_message(&NewMessage::system(&two, "x", json!({}))).unwrap();
        assert_eq!(s.latest_session(&a.id).unwrap().unwrap().id, one.id, "a message it sends is no arrival");
    }

    #[test]
    fn unanswered_deliveries_skip_turns_that_ended_without_a_reply() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let b = s.insert_bot(&new_bot("B"), "b").unwrap();
        let (a1, b1) = (session(&s, &a), session(&s, &b));
        let delivered = |body: &str| NewMessage { status: MessageStatus::Delivered, ..user_msg(&a1, body) };
        let answer = s
            .insert_message(&NewMessage { status: MessageStatus::Delivered, ..between(&b1, &a1, "the answer") })
            .unwrap();
        let cut = s.insert_message(&delivered("cut off")).unwrap();
        s.mark_turn_ended(&answer.id).unwrap();
        let ids: Vec<String> = s.unanswered_deliveries().unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![cut.id]);
    }

    #[test]
    fn a_version_1_database_gains_the_turn_end_marker_and_keeps_its_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("db.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            apply_first(&conn, 1);
            conn.execute_batch(
                "INSERT INTO bots (id, handle, display_name, color, provider, instructions, created_at, updated_at) \
                   VALUES ('b1', 'a', 'A', '#000', 'claude', '', 't', 't');
                 INSERT INTO threads (id, participant_a, participant_b) VALUES ('t1', 'b1', 'user');
                 INSERT INTO messages (id, thread_id, sender, recipient, kind, body, status, chain_id, hop, \
                   in_reply_to, created_at) VALUES
                   ('m1', 't1', 'user', 'b1', 'text', 'answered', 'delivered', 'c1', 0, NULL, 't'),
                   ('m2', 't1', 'b1', 'user', 'text', 'the answer', 'delivered', 'c1', 1, 'm1', 't'),
                   ('m3', 't1', 'user', 'b1', 'text', 'cut off', 'delivered', 'c2', 0, NULL, 't');",
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        let version: i64 = s.conn.lock().unwrap().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
        assert_eq!(s.get_message("m2").unwrap().unwrap().in_reply_to.as_deref(), Some("m1"));
        let unanswered = |s: &Store| s.unanswered_deliveries().unwrap().into_iter().map(|m| m.id).collect::<Vec<_>>();
        assert_eq!(unanswered(&s), vec!["m3".to_string()], "a Phase 1 row with a reply needs no marker");
        s.mark_turn_ended("m3").unwrap();
        assert!(unanswered(&s).is_empty());
    }

    #[test]
    fn a_version_4_database_gives_each_bot_session_1_with_its_conversation() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("db.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            apply_first(&conn, 4);
            conn.execute_batch(
                "INSERT INTO bots (id, handle, display_name, color, provider, model, instructions, \
                   provider_session_id, config_version, created_at, updated_at, session_config_version) VALUES
                   ('ba', 'a', 'A', '#000', 'claude', NULL, '', 'sess-a', 3,
                     '2026-10-01T00:00:00.000Z', '2026-10-01T01:00:00.000Z', 3),
                   ('bb', 'b', 'B', '#000', 'grok', NULL, '', NULL, 1,
                     '2026-10-02T00:00:00.000Z', '2026-10-02T01:00:00.000Z', NULL),
                   ('bc', 'c', 'C', '#000', 'claude', NULL, '', NULL, 1,
                     '2026-10-03T00:00:00.000Z', '2026-10-03T01:00:00.000Z', NULL);
                 INSERT INTO threads (id, participant_a, participant_b) VALUES
                   ('t-ua', 'ba', 'user'), ('t-ub', 'bb', 'user'), ('t-ab', 'ba', 'bb'), ('t-ug', 'gone', 'user');
                 INSERT INTO messages (id, thread_id, sender, recipient, kind, body, status, chain_id, hop, \
                   in_reply_to, created_at) VALUES
                   ('m7', 't-ug', 'gone', 'user', 'text', 'old', 'delivered', 'c5', 0, NULL, '2026-10-01T00:00:00.000Z'),
                   ('m1', 't-ua', 'user', 'ba', 'text', 'hey', 'delivered', 'c1', 0, NULL, '2026-10-05T00:00:00.000Z'),
                   ('m2', 't-ua', 'ba', 'user', 'text', 'hi', 'delivered', 'c1', 1, 'm1', '2026-10-05T00:01:00.000Z'),
                   ('m4', 't-ub', 'user', 'bb', 'text', 'hey', 'delivered', 'c3', 0, NULL, '2026-10-06T00:00:00.000Z'),
                   ('m5', 't-ab', 'ba', 'bb', 'text', 'ask', 'delivered', 'c4', 1, NULL, '2026-10-07T00:00:00.000Z'),
                   ('m6', 't-ab', 'bb', 'ba', 'text', 'yes', 'delivered', 'c4', 2, 'm5', '2026-10-07T00:01:00.000Z'),
                   ('m3', 't-ua', 'user', 'ba', 'text', 'and', 'queued', 'c2', 0, NULL, '2026-10-08T00:00:00.000Z');",
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        let conn = s.conn.lock().unwrap();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);

        type SessionRow = (String, i64, Option<String>, Option<String>, Option<i64>, Option<String>, String, String);
        let sessions: Vec<(String, SessionRow)> = conn
            .prepare(
                "SELECT id, bot_id, number, title, provider_session_id, session_config_version, opened_by, \
                 created_at, last_activity_at FROM sessions ORDER BY bot_id",
            )
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?),
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let row = |bot: &str, cli: Option<&str>, seen: Option<i64>, created: &str, active: &str| -> SessionRow {
            (bot.into(), 1, None, cli.map(Into::into), seen, None, created.into(), active.into())
        };
        assert_eq!(
            sessions.iter().map(|(_, row)| row.clone()).collect::<Vec<_>>(),
            vec![
                row("ba", Some("sess-a"), Some(3), "2026-10-01T00:00:00.000Z", "2026-10-08T00:00:00.000Z"),
                row("bb", None, None, "2026-10-02T00:00:00.000Z", "2026-10-07T00:01:00.000Z"),
                row("bc", None, None, "2026-10-03T00:00:00.000Z", "2026-10-03T01:00:00.000Z"),
            ]
        );
        let [sa, sb, _sc] = [0, 1, 2].map(|i| sessions[i].0.clone());
        assert!(sessions.iter().all(|(id, _)| id.len() == 26), "ULIDs: {sessions:?}");

        let ends: Vec<(String, Option<String>, Option<String>)> = conn
            .prepare("SELECT id, sender_session, recipient_session FROM messages ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let end = |id: &str, from: Option<&String>, to: Option<&String>| (id.to_string(), from.cloned(), to.cloned());
        assert_eq!(
            ends,
            vec![
                end("m1", None, Some(&sa)),
                end("m2", Some(&sa), None),
                end("m3", None, Some(&sa)),
                end("m4", None, Some(&sb)),
                end("m5", Some(&sa), Some(&sb)),
                end("m6", Some(&sb), Some(&sa)),
                end("m7", None, None),
            ]
        );

        let threads: Vec<(String, String, String)> = conn
            .prepare("SELECT id, participant_a, participant_b FROM threads ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let pair = |id: &str, x: &str, y: &str| {
            let (a, b) = if x <= y { (x, y) } else { (y, x) };
            (id.to_string(), a.to_string(), b.to_string())
        };
        assert_eq!(
            threads,
            vec![
                pair("t-ab", &sa, &sb),
                pair("t-ua", &sa, USER),
                pair("t-ub", &sb, USER),
                ("t-ug".to_string(), "gone".to_string(), USER.to_string()),
            ]
        );
        let thread_of: Vec<(String, String)> = conn
            .prepare("SELECT id, thread_id FROM messages ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let in_thread = |m: &str, t: &str| (m.to_string(), t.to_string());
        assert_eq!(
            thread_of,
            vec![
                in_thread("m1", "t-ua"),
                in_thread("m2", "t-ua"),
                in_thread("m3", "t-ua"),
                in_thread("m4", "t-ub"),
                in_thread("m5", "t-ab"),
                in_thread("m6", "t-ab"),
                in_thread("m7", "t-ug"),
            ],
            "every message kept its thread"
        );

        let bot_columns: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('bots')")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            !bot_columns.iter().any(|c| c == "provider_session_id" || c == "session_config_version"),
            "{bot_columns:?}"
        );
        let broken: i64 = conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r.get(0)).unwrap();
        assert_eq!(broken, 0);
        drop(conn);

        assert_eq!(s.get_bot("ba").unwrap().unwrap().config_version, 3);
        let queued: Vec<String> = s.queued_for_session(&sa).unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(queued, vec!["m3".to_string()]);
        assert_eq!(s.thread_for(USER, &sa).unwrap(), "t-ua", "a new message finds the remapped thread");
    }

    #[test]
    fn a_failed_migration_5_leaves_the_version_4_database_as_it_was() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("db.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            apply_first(&conn, 4);
            conn.execute_batch(
                "INSERT INTO bots (id, handle, display_name, color, provider, instructions, provider_session_id, \
                   created_at, updated_at) VALUES ('ba', 'a', 'A', '#000', 'claude', '', 'sess-a', 't', 't');
                 INSERT INTO threads (id, participant_a, participant_b) VALUES ('t1', 'ba', 'user');
                 INSERT INTO messages (id, thread_id, sender, recipient, kind, body, status, chain_id, hop, created_at)
                   VALUES ('m1', 't1', 'user', 'ba', 'text', 'hello', 'queued', 'c1', 0, 't');
                 -- An index on a column migration 5 drops makes its last step fail.
                 CREATE INDEX bots_cli ON bots(provider_session_id);",
            )
            .unwrap();
        }
        let err = Store::open(&path).err().expect("migration 5 fails");
        assert!(err.to_string().contains("drop column"), "it failed at its last step: {err}");
        let conn = Connection::open(&path).unwrap();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        let tables: i64 =
            conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'sessions'", [], |r| r.get(0)).unwrap();
        let thread: (String, String) = conn
            .query_row("SELECT participant_a, participant_b FROM threads", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        let cli: Option<String> = conn.query_row("SELECT provider_session_id FROM bots", [], |r| r.get(0)).unwrap();
        let message_columns: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name LIKE '%_session'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            (version, tables, thread, cli.as_deref(), message_columns),
            (4, 0, ("ba".to_string(), "user".to_string()), Some("sess-a"), 0)
        );
    }

    fn version_4_file(path: &Path) {
        let conn = Connection::open(path).unwrap();
        apply_first(&conn, 4);
        conn.execute_batch(
            "INSERT INTO bots (id, handle, display_name, color, provider, instructions, provider_session_id, \
               created_at, updated_at) VALUES ('ba', 'a', 'A', '#000', 'claude', '', 'sess-a', 't', 't');",
        )
        .unwrap();
    }

    #[test]
    fn a_version_4_file_is_copied_before_migration_5_and_the_copy_opens_at_version_4() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("db.sqlite");
        version_4_file(&path);
        let s = Store::open(&path).unwrap();
        assert_eq!(s.sessions_of("ba").unwrap().len(), 1, "migrated");
        drop(s);
        let backup = tmp.path().join("db.sqlite.v4.bak");
        assert_eq!(v4_backup_path(&path), backup);
        let conn = Connection::open(&backup).unwrap();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        let cli: Option<String> = conn.query_row("SELECT provider_session_id FROM bots", [], |r| r.get(0)).unwrap();
        assert_eq!((version, cli.as_deref()), (4, Some("sess-a")));
        drop(conn);
        std::fs::remove_file(&backup).unwrap();
        drop(Store::open(&path).unwrap());
        assert!(!backup.exists());
    }

    #[test]
    fn a_backup_already_there_is_kept_and_a_new_database_gets_none() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("db.sqlite");
        version_4_file(&path);
        std::fs::write(v4_backup_path(&path), "an earlier copy").unwrap();
        drop(Store::open(&path).unwrap());
        assert_eq!(std::fs::read_to_string(v4_backup_path(&path)).unwrap(), "an earlier copy");
        let fresh = tmp.path().join("new.sqlite");
        drop(Store::open(&fresh).unwrap());
        assert!(!v4_backup_path(&fresh).exists(), "nothing to keep in a new database");
    }

    #[test]
    fn a_failed_backup_leaves_the_version_4_database_unmigrated() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("db.sqlite");
        version_4_file(&path);
        let unwritable = tmp.path().join("missing").join("db.sqlite.v4.bak");
        let err =
            Store::init(Connection::open(&path).unwrap(), Some(&unwritable)).err().expect("no copy, no migration");
        assert!(err.to_string().contains("could not copy the database"), "{err}");
        let conn = Connection::open(&path).unwrap();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        let sessions: i64 =
            conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'sessions'", [], |r| r.get(0)).unwrap();
        assert_eq!((version, sessions), (4, 0));
    }

    #[test]
    fn migration_5_swaps_a_pair_whose_new_session_sorts_after_a_deleted_bot() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("db.sqlite");
        let (live, deleted) = ("01HA0000000000000000000000", "01HD0000000000000000000000");
        {
            let conn = Connection::open(&path).unwrap();
            apply_first(&conn, 4);
            conn.execute_batch(&format!(
                "INSERT INTO bots (id, handle, display_name, color, provider, instructions, created_at, updated_at) \
                   VALUES ('{live}', 'a', 'A', '#000', 'claude', '', 't', 't');
                 INSERT INTO threads (id, participant_a, participant_b) VALUES ('t1', '{live}', '{deleted}');
                 INSERT INTO messages (id, thread_id, sender, recipient, kind, body, status, chain_id, hop, created_at)
                   VALUES ('m1', 't1', '{deleted}', '{live}', 'text', 'hi', 'delivered', 'c1', 0, 't');"
            ))
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        let sa = s.sessions_of(live).unwrap().remove(0).id;
        assert!(live < deleted && deleted < sa.as_str(), "the test needs {live} < {deleted} < {sa}");
        let pair: (String, String) = s
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT participant_a, participant_b FROM threads WHERE id = 't1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(pair, (deleted.to_string(), sa.clone()));
        assert_eq!(s.thread_for(&sa, deleted).unwrap(), "t1");
    }

    #[test]
    fn only_a_queued_message_is_marked_delivered() {
        let s = Store::open_in_memory().unwrap();
        let bot = s.insert_bot(&new_bot("A"), "a").unwrap();
        let to = session(&s, &bot);
        let m = s.insert_message(&user_msg(&to, "hi")).unwrap();
        let delivered = s.mark_delivered(&m.id).unwrap().expect("it was queued");
        assert_eq!(delivered.status, MessageStatus::Delivered);
        assert!(delivered.delivered_at.is_some());
        assert_eq!(s.mark_delivered(&m.id).unwrap(), None, "already delivered");
        let claimed = s.insert_message(&user_msg(&to, "claimed")).unwrap();
        s.set_message_status(&claimed.id, MessageStatus::Interrupted).unwrap();
        assert_eq!(s.mark_delivered(&claimed.id).unwrap(), None);
        assert_eq!(s.get_message(&claimed.id).unwrap().unwrap().status, MessageStatus::Interrupted);
        assert_eq!(s.mark_delivered("missing").unwrap(), None, "a removal deleted it");
    }

    #[test]
    fn system_params_round_trip() {
        let s = Store::open_in_memory().unwrap();
        let bot = s.insert_bot(&new_bot("A"), "a").unwrap();
        let m = s.insert_message(&NewMessage::system(&session(&s, &bot), "bot_start_failed", json!({"error": "boom"})));
        let m = m.unwrap();
        let got = s.get_message(&m.id).unwrap().unwrap();
        assert_eq!(got.system_params, Some(json!({"error": "boom"})));
        assert_eq!(got.status, MessageStatus::Delivered);
        assert!(got.delivered_at.is_some());
    }

    #[test]
    fn settings_upsert() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.get_setting("language").unwrap(), None);
        s.set_setting("language", "es").unwrap();
        s.set_setting("language", "de").unwrap();
        assert_eq!(s.get_setting("language").unwrap().as_deref(), Some("de"));
    }

    #[test]
    fn attachments_round_trip_and_an_id_can_be_chosen() {
        let s = Store::open_in_memory().unwrap();
        let bot = s.insert_bot(&new_bot("A"), "a").unwrap();
        let one = session(&s, &bot);
        assert!(s.insert_message(&user_msg(&one, "no files")).unwrap().attachments.is_empty());
        let files = vec![Attachment { name: "a b.md".into(), path: "/x/inbox/01JCHOSEN-a b.md".into() }];
        let m = s
            .insert_message_with_id("01JCHOSEN", &NewMessage { attachments: files.clone(), ..user_msg(&one, "files") })
            .unwrap();
        assert_eq!(m.id, "01JCHOSEN");
        assert_eq!(s.get_message("01JCHOSEN").unwrap().unwrap().attachments, files);
        assert_eq!(s.queued_for_session(&one.id).unwrap()[1].attachments, files);
    }

    #[test]
    fn bots_are_found_by_handle() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        assert_eq!(s.get_bot_by_handle("a").unwrap(), Some(a));
        assert_eq!(s.get_bot_by_handle("missing").unwrap(), None);
    }

    #[test]
    fn bot_threads_list_bot_pairs_with_messages_newest_first() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let b = s.insert_bot(&new_bot("B"), "b").unwrap();
        let c = s.insert_bot(&new_bot("C"), "c").unwrap();
        let (a1, b1, c1) = (session(&s, &a), session(&s, &b), session(&s, &c));
        s.insert_message(&user_msg(&a1, "from the user")).unwrap();
        s.insert_message(&between(&a1, &b1, "x")).unwrap();
        s.insert_message(&between(&c1, &b1, "x")).unwrap();
        s.thread_for(&a1.id, &c1.id).unwrap();
        let threads = s.bot_threads().unwrap();
        let pairs: Vec<(String, String)> =
            threads.iter().map(|t| (t.participant_a.clone(), t.participant_b.clone())).collect();
        let sorted =
            |x: &str, y: &str| if x <= y { (x.to_string(), y.to_string()) } else { (y.to_string(), x.to_string()) };
        assert_eq!(pairs, vec![sorted(&b.id, &c.id), sorted(&a.id, &b.id)]);
        assert!(!threads[0].last_message_at.is_empty());
    }

    #[test]
    fn bot_threads_name_the_session_at_each_end_and_none_for_a_raw_end() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let b = s.insert_bot(&new_bot("B"), "b").unwrap();
        let (a1, b1) = (session(&s, &a), session(&s, &b));
        s.insert_message(&between(&a1, &b1, "x")).unwrap();
        let t = s.bot_threads().unwrap().remove(0);
        let ends = [(&t.participant_a, &t.session_a), (&t.participant_b, &t.session_b)];
        for (bot, session) in ends {
            let expected = if *bot == a.id { &a1.id } else { &b1.id };
            assert_eq!(session.as_ref(), Some(expected));
        }
        s.insert_message(&NewMessage { sender: "gone".into(), ..user_msg(&b1, "late") }).unwrap();
        let t = s.bot_threads().unwrap().remove(0);
        let raw = if t.participant_a == "gone" { &t.session_a } else { &t.session_b };
        let known = if t.participant_a == "gone" { &t.session_b } else { &t.session_a };
        assert_eq!((raw.as_ref(), known.as_ref()), (None, Some(&b1.id)));
    }

    #[test]
    fn bot_threads_name_each_end_by_its_bot() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let b = s.insert_bot(&new_bot("B"), "b").unwrap();
        let [a1, a2] = [(); 2].map(|()| s.create_session(&a.id, None, None).unwrap());
        let b1 = session(&s, &b);
        let first = s.insert_message(&between(&a1, &b1, "x")).unwrap();
        let second = s.insert_message(&between(&a2, &b1, "x")).unwrap();
        let late = s.insert_message(&NewMessage { sender: "gone".into(), ..user_msg(&b1, "late") }).unwrap();
        s.insert_message(&user_msg(&a1, "from the user")).unwrap();
        let threads: Vec<(String, String, String)> =
            s.bot_threads().unwrap().into_iter().map(|t| (t.id, t.participant_a, t.participant_b)).collect();
        let (lo, hi) = if a.id <= b.id { (&a.id, &b.id) } else { (&b.id, &a.id) };
        let (lo_gone, hi_gone) =
            if b.id.as_str() <= "gone" { (b.id.as_str(), "gone") } else { ("gone", b.id.as_str()) };
        assert_eq!(
            threads,
            vec![
                (late.thread_id, lo_gone.to_string(), hi_gone.to_string()),
                (second.thread_id, lo.clone(), hi.clone()),
                (first.thread_id, lo.clone(), hi.clone()),
            ]
        );
    }

    #[test]
    fn only_queued_messages_can_be_cancelled() {
        let s = Store::open_in_memory().unwrap();
        let bot = s.insert_bot(&new_bot("A"), "a").unwrap();
        let one = session(&s, &bot);
        let queued = s.insert_message(&user_msg(&one, "q")).unwrap();
        let delivered =
            s.insert_message(&NewMessage { status: MessageStatus::Delivered, ..user_msg(&one, "d") }).unwrap();
        assert_eq!(s.cancel_queued(&queued.id).unwrap().unwrap().status, MessageStatus::Cancelled);
        assert_eq!(s.cancel_queued(&queued.id).unwrap(), None, "already cancelled");
        assert_eq!(s.cancel_queued(&delivered.id).unwrap(), None);
        assert!(s.queued_for_session(&one.id).unwrap().is_empty());
    }

    #[test]
    fn a_database_from_a_newer_app_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("db.sqlite");
        drop(Store::open(&path).unwrap());
        Connection::open(&path).unwrap().pragma_update(None, "user_version", MIGRATIONS.len() as i64 + 1).unwrap();
        match Store::open(&path) {
            Err(crate::CoreError::Refused(crate::Refusal::DatabaseTooNew)) => {}
            Err(e) => panic!("unexpected error {e}"),
            Ok(_) => panic!("a newer database was opened"),
        }
    }

    #[test]
    fn reopening_a_file_keeps_data() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("db.sqlite");
        let id = Store::open(&path).unwrap().insert_bot(&new_bot("A"), "a").unwrap().id;
        assert!(Store::open(&path).unwrap().get_bot(&id).unwrap().is_some());
    }

    #[test]
    fn deleting_a_bot_cancels_what_waits_to_or_from_it_and_keeps_the_rest() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_bot(&new_bot("A"), "a").unwrap();
        let b = s.insert_bot(&new_bot("B"), "b").unwrap();
        let (a1, b1) = (session(&s, &a), session(&s, &b));
        let to_a = s.insert_message(&user_msg(&a1, "for a")).unwrap();
        let from_a = s.insert_message(&between(&a1, &b1, "from a")).unwrap();
        let to_b = s.insert_message(&user_msg(&b1, "for b")).unwrap();
        let done = s.insert_message(&NewMessage { status: MessageStatus::Delivered, ..user_msg(&a1, "done") }).unwrap();
        let cut_to_a =
            s.insert_message(&NewMessage { status: MessageStatus::Interrupted, ..user_msg(&a1, "cut") }).unwrap();
        let cut_from_a = s
            .insert_message(&NewMessage { status: MessageStatus::Interrupted, ..between(&a1, &b1, "cut from a") })
            .unwrap();

        let (bot, cancelled) = s.delete_bot(&a.id).unwrap().unwrap();
        assert_eq!(bot.id, a.id);
        assert_eq!(
            cancelled.iter().map(|m| (m.id.as_str(), m.status)).collect::<Vec<_>>(),
            vec![
                (to_a.id.as_str(), MessageStatus::Cancelled),
                (from_a.id.as_str(), MessageStatus::Cancelled),
                (cut_to_a.id.as_str(), MessageStatus::Cancelled),
                (cut_from_a.id.as_str(), MessageStatus::Cancelled)
            ]
        );
        assert_eq!(s.get_bot(&a.id).unwrap(), None);
        assert_eq!(s.get_message(&to_b.id).unwrap().unwrap().status, MessageStatus::Queued);
        assert_eq!(s.get_message(&done.id).unwrap().unwrap().status, MessageStatus::Delivered);
        assert_eq!(s.thread_messages(&to_a.thread_id).unwrap().len(), 3, "the thread stays");
        assert_eq!(s.delete_bot(&a.id).unwrap(), None);
    }

    #[test]
    fn skills_are_stored_listed_with_their_bots_and_attached_in_slug_order() {
        let s = Store::open_in_memory().unwrap();
        let brief = s.insert_skill("brief", "Brief", "Writes briefs.", SkillSource::Written).unwrap();
        let audit = s.insert_skill("audit", "Audit", "Audits code.", SkillSource::Imported).unwrap();
        assert_eq!(s.skill_slugs().unwrap(), vec!["audit".to_string(), "brief".to_string()]);
        let attached = vec![brief.id.clone(), audit.id.clone(), brief.id.clone()];
        let bot = s.insert_bot(&NewBot { skill_ids: attached, ..new_bot("Researcher") }, "researcher").unwrap();
        assert_eq!(bot.skill_ids, vec![audit.id.clone(), brief.id.clone()], "by slug, each once");
        assert_eq!(s.get_bot(&bot.id).unwrap().unwrap().skill_ids, bot.skill_ids);
        assert_eq!(s.list_bots().unwrap()[0].skill_ids, bot.skill_ids);
        let listed: Vec<(String, Vec<String>)> =
            s.list_skills().unwrap().into_iter().map(|v| (v.skill.slug, v.used_by)).collect();
        let researcher = vec!["researcher".to_string()];
        assert_eq!(listed, vec![("audit".to_string(), researcher.clone()), ("brief".to_string(), researcher)]);
        let of_bot: Vec<String> = s.skills_of(&bot.id).unwrap().into_iter().map(|k| k.slug).collect();
        assert_eq!(of_bot, vec!["audit".to_string(), "brief".to_string()]);
        assert_eq!(s.get_skill(&audit.id).unwrap().unwrap().source, SkillSource::Imported);
        assert_eq!(s.get_skill("missing").unwrap(), None);
    }

    #[test]
    fn skills_are_found_by_slug() {
        let s = Store::open_in_memory().unwrap();
        let audit = s.insert_skill("audit", "Audit", "Audits code.", SkillSource::Written).unwrap();
        assert_eq!(s.get_skill_by_slug("audit").unwrap(), Some(audit));
        assert_eq!(s.get_skill_by_slug("missing").unwrap(), None);
    }

    #[test]
    fn changing_a_bots_skills_bumps_its_config_version_and_the_same_set_does_not() {
        let s = Store::open_in_memory().unwrap();
        let audit = s.insert_skill("audit", "Audit", "Audits code.", SkillSource::Written).unwrap();
        let bot = s.insert_bot(&new_bot("Researcher"), "researcher").unwrap();
        let edit = |ids: Vec<String>| NewBot { skill_ids: ids, ..new_bot("Researcher") };
        let edited = s.update_bot(&bot.id, &edit(vec![audit.id.clone()])).unwrap().unwrap();
        assert_eq!((edited.config_version, edited.skill_ids.clone()), (2, vec![audit.id.clone()]));
        let same = s.update_bot(&bot.id, &edit(vec![audit.id.clone(), audit.id.clone()])).unwrap().unwrap();
        assert_eq!(same.config_version, 2, "the same set, listed twice, is no change");
        let removed = s.update_bot(&bot.id, &edit(vec![])).unwrap().unwrap();
        assert_eq!((removed.config_version, removed.skill_ids.len()), (3, 0));
    }

    #[test]
    fn editing_a_skill_bumps_the_bots_that_have_it_and_deleting_needs_no_bot() {
        let s = Store::open_in_memory().unwrap();
        let audit = s.insert_skill("audit", "Audit", "Audits code.", SkillSource::Written).unwrap();
        let brief = s.insert_skill("brief", "Brief", "Writes briefs.", SkillSource::Written).unwrap();
        let with =
            s.insert_bot(&NewBot { skill_ids: vec![audit.id.clone()], ..new_bot("Researcher") }, "researcher").unwrap();
        let without = s.insert_bot(&new_bot("Writer"), "writer").unwrap();
        let renamed = s.update_skill(&audit.id, "Audits", "Audits anything.").unwrap().unwrap();
        assert_eq!((renamed.name.as_str(), renamed.description.as_str()), ("Audits", "Audits anything."));
        assert_eq!(s.update_skill("missing", "x", "y").unwrap(), None);
        assert_eq!(s.bump_bots_with_skill(&audit.id).unwrap(), vec!["researcher".to_string()]);
        assert_eq!(s.get_bot(&with.id).unwrap().unwrap().config_version, 2);
        assert_eq!(s.get_bot(&without.id).unwrap().unwrap().config_version, 1);
        assert_eq!(s.bots_using_skill(&audit.id).unwrap(), vec!["researcher".to_string()]);
        assert!(s.delete_skill(&audit.id).is_err(), "the foreign key refuses a skill a bot has");
        assert_eq!(s.delete_skill(&brief.id).unwrap().map(|k| k.slug), Some("brief".to_string()));
        assert_eq!(s.delete_skill(&brief.id).unwrap(), None);
        s.delete_bot(&with.id).unwrap();
        assert!(s.bots_using_skill(&audit.id).unwrap().is_empty(), "deleting the bot drops its attachments");
        assert_eq!(s.delete_skill(&audit.id).unwrap().map(|k| k.slug), Some("audit".to_string()));
    }

    fn task_row(bot: &Bot, name: &str) -> TaskRow {
        TaskRow {
            name: name.into(),
            bot_id: bot.id.clone(),
            prompt: "Update the wiki.".into(),
            every_days: 7,
            at: "07:00".into(),
            starts_on: "2027-06-07".into(),
            enabled: true,
            next_run_at: Some("2027-06-07T07:00:00.000Z".into()),
        }
    }

    #[test]
    fn migration_9_marks_every_session_seen() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("db.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            apply_first(&conn, 8);
            conn.execute_batch(
                "INSERT INTO bots (id, handle, display_name, color, provider, instructions, created_at, updated_at) \
                   VALUES ('b1', 'a', 'A', '#000', 'claude', '', 't', 't');
                 INSERT INTO sessions (id, bot_id, number, created_at, last_activity_at) \
                   VALUES ('s1', 'b1', 1, 't', '2026-01-01T00:00:00.000Z');",
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        let session = s.get_session("s1").unwrap().unwrap();
        let seen = session.seen_at.expect("seen");
        assert!(seen >= session.last_activity_at, "old work starts out read");
    }

    #[test]
    fn a_new_session_is_unseen_until_marked() {
        let s = Store::open_in_memory().unwrap();
        let bot = s.insert_bot(&new_bot("A"), "a").unwrap();
        let session = s.create_session(&bot.id, None, None).unwrap();
        assert_eq!(session.seen_at, None);
        let marked = s.set_session_seen(&session.id).unwrap().unwrap();
        assert!(marked.seen_at.is_some());
        assert_eq!(s.set_session_seen("nope").unwrap(), None);
    }

    #[test]
    fn the_card_reads_find_a_sessions_messages_through_its_index() {
        let s = Store::open_in_memory().unwrap();
        let conn = s.conn.lock().unwrap();
        for sql in [latest_where(LAST_SENT), latest_where(LAST_REPLY), LAST_FOR_USER_AT.to_string()] {
            let plan: Vec<String> = conn
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .unwrap()
                .query_map(params!["s1"], |r| r.get(3))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            assert!(plan.iter().any(|step| step.contains("INDEX messages_sender_session")), "{sql}: {plan:?}");
        }
    }

    #[test]
    fn migration_8_adds_tasks_and_the_task_mark() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("db.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            apply_first(&conn, 7);
            conn.execute_batch(
                "INSERT INTO bots (id, handle, display_name, color, provider, instructions, created_at, updated_at) \
                   VALUES ('b1', 'a', 'A', '#000', 'claude', '', 't', 't');
                 INSERT INTO sessions (id, bot_id, number, created_at, last_activity_at) VALUES ('s1', 'b1', 1, 't', 't');
                 INSERT INTO threads (id, participant_a, participant_b) VALUES ('t1', 's1', 'user');
                 INSERT INTO messages (id, thread_id, sender, recipient, kind, body, status, chain_id, hop, \
                   created_at, sender_session, recipient_session) VALUES
                   ('m1', 't1', 'user', 'b1', 'text', 'hi', 'delivered', 'c1', 0, 't', NULL, 's1');",
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        let version: i64 = s.conn.lock().unwrap().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
        let m = s.get_message("m1").unwrap().unwrap();
        assert_eq!((m.task_id, m.task_name, m.task_since), (None, None, None));
        assert!(s.list_tasks().unwrap().is_empty());
    }

    #[test]
    fn a_task_run_is_one_write() {
        let s = Store::open_in_memory().unwrap();
        let bot = s.insert_bot(&new_bot("A"), "a").unwrap();
        let task = s.insert_task(&task_row(&bot, "Wiki update")).unwrap();
        assert_eq!(s.get_task_by_name("wiki UPDATE").unwrap().map(|t| t.id), Some(task.id.clone()));

        let next = "2027-06-14T07:00:00.000Z";
        let slot = Scheduled { read: &task, next_run_at: next };
        let (routed, after) = s
            .start_task_run(&task.id, "Wiki update · 2027-06-07", "Update the wiki.", None, Some(slot))
            .unwrap()
            .unwrap();
        assert!(routed.created);
        assert_eq!(routed.session.title.as_deref(), Some("Wiki update · 2027-06-07"));
        assert_eq!(routed.session.opened_by, None);
        assert_eq!(routed.message.status, MessageStatus::Queued);
        assert_eq!(routed.message.sender, USER);
        assert_eq!(routed.message.recipient_session.as_deref(), Some(routed.session.id.as_str()));
        assert_eq!(routed.message.task_id.as_deref(), Some(task.id.as_str()));
        assert_eq!(routed.message.task_name.as_deref(), Some("Wiki update"));
        assert_eq!(routed.message.task_since, None);
        assert_eq!(after.last_run_session.as_deref(), Some(routed.session.id.as_str()));
        assert!(after.last_run_at.is_some());
        assert_eq!(after.next_run_at.as_deref(), Some(next));
        assert_eq!(s.get_task(&task.id).unwrap(), Some(after));

        s.delete_task(&task.id).unwrap();
        assert!(s.start_task_run(&task.id, "t", "b", None, None).unwrap().is_none());
        assert_eq!(s.sessions_of(&bot.id).unwrap().len(), 1, "no session for a deleted task");
    }

    #[test]
    fn the_scheduler_writes_only_to_the_task_it_read() {
        let s = Store::open_in_memory().unwrap();
        let bot = s.insert_bot(&new_bot("A"), "a").unwrap();
        let next = "2027-06-14T07:00:00.000Z";
        let skipped = "2027-06-07T07:00:00.000Z";

        let read = s.insert_task(&task_row(&bot, "Ran")).unwrap();
        let (_, ran) = s
            .start_task_run(&read.id, "run", "go", None, Some(Scheduled { read: &read, next_run_at: next }))
            .unwrap()
            .unwrap();
        assert_eq!(ran.next_run_at.as_deref(), Some(next));
        let read = s.insert_task(&task_row(&bot, "Skipped")).unwrap();
        let after = s.skip_task_slot(Scheduled { read: &read, next_run_at: next }, skipped).unwrap().unwrap();
        assert_eq!((after.skipped_at.as_deref(), after.next_run_at.as_deref()), (Some(skipped), Some(next)));

        for what in ["a changed next run", "turned off", "off with its next run left", "a changed last run (Run now)"] {
            let read = s.insert_task(&task_row(&bot, what)).unwrap();
            let row = task_row(&bot, what);
            let edited = match what {
                "a changed next run" => TaskRow { next_run_at: Some("2027-06-08T07:00:00.000Z".into()), ..row },
                "turned off" => TaskRow { enabled: false, next_run_at: None, ..row },
                "off with its next run left" => TaskRow { enabled: false, ..row },
                _ => {
                    s.start_task_run(&read.id, "now", "go", None, None).unwrap().unwrap();
                    row
                }
            };
            s.update_task(&read.id, &edited).unwrap().unwrap();
            let before = s.get_task(&read.id).unwrap().unwrap();
            let sessions = s.sessions_of(&bot.id).unwrap().len();
            let slot = Scheduled { read: &read, next_run_at: next };
            assert!(s.start_task_run(&read.id, "run", "go", None, Some(slot)).unwrap().is_none(), "{what}");
            assert!(s.skip_task_slot(slot, skipped).unwrap().is_none(), "{what}");
            assert_eq!(s.get_task(&read.id).unwrap().unwrap(), before, "{what}: the task is as the change left it");
            assert_eq!(s.sessions_of(&bot.id).unwrap().len(), sessions, "{what}: no run");
        }
    }

    #[test]
    fn a_bots_tasks_go_with_it() {
        let s = Store::open_in_memory().unwrap();
        let bot = s.insert_bot(&new_bot("A"), "a").unwrap();
        let gone = s.insert_bot(&new_bot("B"), "b").unwrap();
        let task = s.insert_task(&task_row(&bot, "Wiki update")).unwrap();
        s.insert_task(&task_row(&gone, "Other")).unwrap();
        assert_eq!(s.tasks_of_bot(&gone.id).unwrap().len(), 1);
        s.delete_bot(&gone.id).unwrap();
        assert_eq!(s.list_tasks().unwrap().len(), 1);

        let (routed, _) = s.start_task_run(&task.id, "run", "go", None, None).unwrap().unwrap();
        s.remove_session(&routed.session.id, |_| Ok(())).unwrap();
        assert_eq!(s.get_task(&task.id).unwrap().unwrap().last_run_session, None);
    }

    #[test]
    fn the_last_completed_run_ignores_runs_not_done() {
        let s = Store::open_in_memory().unwrap();
        let bot = s.insert_bot(&new_bot("A"), "a").unwrap();
        let task = s.insert_task(&task_row(&bot, "Wiki update")).unwrap();
        assert_eq!(s.last_completed_run(&task.id).unwrap(), None);
        let (first, _) = s.start_task_run(&task.id, "one", "go", None, None).unwrap().unwrap();
        let (second, _) = s.start_task_run(&task.id, "two", "go", None, None).unwrap().unwrap();
        s.mark_task_done(&first.message.id).unwrap();
        assert_eq!(s.last_completed_run(&task.id).unwrap(), Some(first.session.created_at.clone()));

        let plain = s.insert_message(&user_msg(&second.session, "hi")).unwrap();
        s.mark_task_done(&plain.id).unwrap();
        assert_eq!(s.last_completed_run(&task.id).unwrap(), Some(first.session.created_at));
    }

    #[test]
    fn a_retried_run_keeps_its_task_mark() {
        let s = Store::open_in_memory().unwrap();
        let bot = s.insert_bot(&new_bot("A"), "a").unwrap();
        let task = s.insert_task(&task_row(&bot, "Wiki update")).unwrap();
        let since = "2027-06-01T07:00:00.000Z";
        let (run, _) = s.start_task_run(&task.id, "run", "go", Some(since), None).unwrap().unwrap();
        s.set_message_status(&run.message.id, MessageStatus::Interrupted).unwrap();
        let (_, copy) = s.retry_interrupted(&run.message.id, &user_msg(&run.session, "go")).unwrap().unwrap();
        assert_eq!(copy.task_id.as_deref(), Some(task.id.as_str()));
        assert_eq!(copy.task_name.as_deref(), Some("Wiki update"));
        assert_eq!(copy.task_since.as_deref(), Some(since));
        assert_eq!(s.get_message(&copy.id).unwrap(), Some(copy));
    }

    #[test]
    fn messages_of_session_has_both_ends() {
        let s = Store::open_in_memory().unwrap();
        let [a, b] = ["A", "B"].map(|n| s.insert_bot(&new_bot(n), &n.to_lowercase()).unwrap());
        let (a1, b1) = (session(&s, &a), session(&s, &b));
        let ids: Vec<String> = [
            user_msg(&a1, "task"),
            NewMessage {
                sender: a.id.clone(),
                sender_session: Some(a1.id.clone()),
                recipient: USER.into(),
                recipient_session: None,
                ..to_bot(&a.id, "done")
            },
            between(&a1, &b1, "ask"),
            between(&b1, &a1, "answer"),
        ]
        .iter()
        .map(|m| s.insert_message(m).unwrap().id)
        .collect();
        s.insert_message(&user_msg(&b1, "elsewhere")).unwrap();
        let got: Vec<String> = s.messages_of_session(&a1.id).unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(got, ids);
    }
}
