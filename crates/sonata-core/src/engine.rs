use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{broadcast, Mutex};

use crate::attachments;
use crate::context::{write_bot_files, SkillCopy};
use crate::envelope::{envelope, language_name, raw_output_body, silent_turn_note, Origin};
use crate::handle::{derive_handle, is_valid_handle};
use crate::ids::new_id;
use crate::limits::Skip;
use crate::model::*;
use crate::paths::AppPaths;
use crate::provider::{adapter_for, ProviderAdapter};
use crate::runtime::{AgentState, BotRuntime, LaunchSpec, RuntimeBot, RuntimeError, RuntimeEvent};
use crate::schedule::local_minute;
use crate::store::{Route, Routed, Store};
use crate::workflow::prompts::open_step_line;
use crate::{CoreError, Refusal, Result};

mod archiving;
mod asking;
mod board;
mod changing;
mod config_transfer;
mod controls;
mod defaults;
mod deleting;
mod drafts;
mod editing;
mod follow_up;
mod limits;
mod master;
mod messaging;
mod projects;
mod reviewing;
mod settings;
mod skills;
mod tasks;
mod watchdog;
mod workflows;
pub use changing::Trash;
pub use config_transfer::{
    BackupPlace, DependencyChoice, ExportSummary, ImportMode, ImportOutcome, ImportPreview, ImportReport, ItemKind,
    ItemStatus, PlannedItem, WorkflowFileEntry, WorkflowImportAction, WorkflowImportItem, WorkflowImportOutcome,
    WorkflowImportPreview, WorkflowImportRequest, WrittenItem,
};
pub use controls::DialogAnswer;
pub use messaging::{DEFAULT_HOP_LIMIT, MASTER_HOP_FACTOR};
pub use settings::MAX_HOP_LIMIT;
pub use settings::{LANGUAGES, SYSTEM_LANGUAGE};
pub use tasks::Clock;

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub paths: AppPaths,
    pub sbot_dir: PathBuf,
    pub fallback_lines: u32,
    pub turn_start_grace: Option<Duration>,
}

pub const STALL_RETRY_PAUSE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiEvent {
    MessageAdded {
        message: Message,
    },
    MessageUpdated {
        message: Message,
    },
    BotStatus {
        bot_id: String,
        status: BotStatus,
    },
    SessionStatus {
        session_id: String,
        bot_id: String,
        status: BotStatus,
    },
    SessionAdded {
        session: SessionView,
    },
    SessionUpdated {
        session: SessionView,
    },
    SessionRemoved {
        session_id: String,
        bot_id: String,
        thread_ids: Vec<String>,
    },
    Paused {
        paused: bool,
    },
    SkillsChanged,
    ProjectsChanged,
    TasksChanged,
    WorkflowsChanged,
    WorkflowRunChanged {
        run_id: String,
        name: String,
        status: RunStatus,
        pause: Option<PauseReason>,
        step: Option<String>,
        session_ids: Vec<String>,
    },
    BotAdded {
        bot: BotView,
    },
    BotUpdated {
        bot: BotView,
    },
    UsageLimits {
        limits: Vec<UsageLimitView>,
    },
}

#[derive(Debug, Clone)]
struct InFlight {
    message_id: String,
    sender: String,
    sender_session: Option<String>,
    chain_id: String,
    hop: i64,
    in_reply_to: Option<String>,
    replied: bool,
    seen_working: bool,
    limit_noticed: bool,
    delivered_at: Option<tokio::time::Instant>,
    enter_pressed_at: Option<tokio::time::Instant>,
    stopped: bool,
    follow_up: bool,
    after_limit: bool,
    scheduled: bool,
    from_run: bool,
}

struct Stop {
    session_id: String,
    rt: Option<RuntimeBot>,
    flight: Option<InFlight>,
    cancelled: Vec<Message>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Started {
    provider: Provider,
    config_version: i64,
}

#[derive(Debug, Clone)]
struct Live {
    bot_id: String,
    rt: Option<RuntimeBot>,
    status: BotStatus,
    in_flight: Option<InFlight>,
    remind: bool,
    started: Option<Started>,
    idle_since: Option<tokio::time::Instant>,
    usage_limit_noticed: bool,
    continue_at: Option<tokio::time::Instant>,
    waited_limit_lines: Vec<String>,
    prompt: Option<String>,
    limited_turn: Option<InFlight>,
}

impl Live {
    fn new(bot_id: &str) -> Self {
        Self {
            bot_id: bot_id.to_string(),
            rt: None,
            status: BotStatus::Offline,
            in_flight: None,
            remind: false,
            started: None,
            idle_since: None,
            usage_limit_noticed: false,
            continue_at: None,
            waited_limit_lines: Vec::new(),
            prompt: None,
            limited_turn: None,
        }
    }

    fn waits_for_limit(&self) -> bool {
        self.status == BotStatus::Limited && self.continue_at.is_none()
    }
}

struct Inner {
    runtime: Arc<dyn BotRuntime>,
    cfg: EngineConfig,
    bots_root: std::sync::RwLock<PathBuf>,
    live: Mutex<HashMap<String, Live>>,
    start_locks: std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>,
    written: std::sync::Mutex<HashMap<String, (PathBuf, i64, String)>>,
    shown: std::sync::Mutex<HashMap<String, BotStatus>>,
    shown_sessions: std::sync::Mutex<HashMap<String, (String, BotStatus)>>,
    ui: broadcast::Sender<UiEvent>,
    closing: AtomicBool,
    system_language: std::sync::RwLock<String>,
    clock: std::sync::RwLock<Clock>,
    trash: std::sync::RwLock<Option<Trash>>,
    last_limit_read: std::sync::Mutex<Option<tokio::time::Instant>>,
    importing: std::sync::Mutex<()>,
}

#[derive(Clone)]
pub struct Engine {
    pub(crate) store: Store,
    inner: Arc<Inner>,
}

fn answers_a_bot(flight: &InFlight) -> bool {
    flight.sender != USER && flight.in_reply_to.is_some()
}

fn folder_inside(dir: &Path, root: &Path) -> bool {
    match (std::fs::canonicalize(dir), std::fs::canonicalize(root)) {
        (Ok(dir), Ok(root)) => dir != root && dir.starts_with(&root),
        _ => false,
    }
}

fn session_for_pane(live: &HashMap<String, Live>, pane_id: &str) -> Option<String> {
    live.iter().find(|(_, l)| l.rt.as_ref().is_some_and(|rt| rt.pane_id == pane_id)).map(|(id, _)| id.clone())
}

fn aggregate(statuses: impl IntoIterator<Item = BotStatus>) -> BotStatus {
    let weight = |status: &BotStatus| match status {
        BotStatus::Offline => 0,
        BotStatus::Idle => 1,
        BotStatus::Starting => 2,
        BotStatus::Limited => 3,
        BotStatus::Working => 4,
        BotStatus::NeedsAttention => 5,
    };
    statuses.into_iter().max_by_key(weight).unwrap_or(BotStatus::Offline)
}

fn bot_status(live: &HashMap<String, Live>, bot_id: &str) -> BotStatus {
    aggregate(live.values().filter(|l| l.bot_id == bot_id).map(|l| l.status))
}

fn view_of(live: &HashMap<String, Live>, session: Session) -> SessionView {
    let status = live.get(&session.id).map_or(BotStatus::Offline, |l| l.status);
    SessionView { session, status }
}

const TITLE_MAX: usize = 60;

fn session_title(body: &str) -> Option<String> {
    first_line(body, TITLE_MAX)
}

fn first_line(body: &str, max: usize) -> Option<String> {
    let line = body.lines().map(str::trim).find(|line| !line.is_empty())?;
    if line.chars().count() <= max {
        return Some(line.to_string());
    }
    Some(line.chars().take(max - 1).chain(['…']).collect())
}

struct Who {
    bot: String,
    session: i64,
}

fn removed_with_its_conversation(e: &CoreError) -> bool {
    matches!(e, CoreError::Refused(Refusal::MessageNotFound(_)))
}

fn error_params(e: &CoreError) -> Value {
    match e {
        CoreError::Refused(refusal) => {
            let mut params = refusal.params();
            params["code"] = Value::String(refusal.code().to_string());
            if params.get("error").is_none() {
                params["error"] = Value::String(e.to_string());
            }
            params
        }
        other => json!({ "error": other.to_string() }),
    }
}

impl Engine {
    pub fn new(store: Store, runtime: Arc<dyn BotRuntime>, cfg: EngineConfig) -> Self {
        let (ui, _) = broadcast::channel(1024);
        Self {
            store,
            inner: Arc::new(Inner {
                runtime,
                bots_root: std::sync::RwLock::new(cfg.paths.bots_root.clone()),
                cfg,
                live: Mutex::new(HashMap::new()),
                start_locks: std::sync::Mutex::new(HashMap::new()),
                written: std::sync::Mutex::new(HashMap::new()),
                shown: std::sync::Mutex::new(HashMap::new()),
                shown_sessions: std::sync::Mutex::new(HashMap::new()),
                ui,
                closing: AtomicBool::new(false),
                system_language: std::sync::RwLock::new("en".into()),
                clock: std::sync::RwLock::new(Arc::new(chrono::Utc::now)),
                trash: std::sync::RwLock::new(None),
                last_limit_read: std::sync::Mutex::new(None),
                importing: std::sync::Mutex::new(()),
            }),
        }
    }

    pub fn subscribe_ui(&self) -> broadcast::Receiver<UiEvent> {
        self.inner.ui.subscribe()
    }

    pub fn run_event_loop(&self) -> tokio::task::JoinHandle<()> {
        let engine = self.clone();
        let mut rx = self.inner.runtime.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => engine.on_runtime_event(event).await,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    }

    pub async fn recover(&self) -> Result<()> {
        let in_flight: HashMap<String, String> = {
            let live = self.inner.live.lock().await;
            live.iter().filter_map(|(id, l)| Some((id.clone(), l.in_flight.as_ref()?.message_id.clone()))).collect()
        };
        for m in self.store.unanswered_deliveries()? {
            if m.recipient_session.as_ref().and_then(|s| in_flight.get(s)) != Some(&m.id) {
                let message = self.store.set_message_status(&m.id, MessageStatus::Interrupted)?;
                self.report_interrupted(message);
            }
        }
        self.pause_runs_missing_a_session(None);
        let pumps: Vec<_> = self
            .store
            .sessions_with_queued()?
            .iter()
            .map(|session_id| tokio::spawn(self.pump_session(session_id)))
            .collect();
        for pump in pumps {
            let _ = pump.await;
        }
        Ok(())
    }

    pub fn create_bot(&self, new: NewBot) -> Result<Bot> {
        self.create_bot_with(new, None, None, |new, handle| self.store.insert_bot(new, handle))
    }

    fn taken_handles(&self) -> Result<Vec<String>> {
        let mut taken = self.store.scoped(self.store.project_key()).bot_handles()?;
        if let Ok(entries) = std::fs::read_dir(self.project_workspace()) {
            taken.extend(entries.filter_map(|e| Some(e.ok()?.file_name().into_string().ok()?.to_lowercase())));
        }
        Ok(taken)
    }

    fn create_bot_with(
        &self,
        new: NewBot,
        wanted: Option<&str>,
        reuse_empty: Option<&str>,
        insert: impl FnOnce(&NewBot, &str) -> Result<Bot>,
    ) -> Result<Bot> {
        self.store.require_active()?;
        let new = editing::checked_bot(new)?;
        let new = NewBot { skill_ids: self.checked_skill_ids(&new.skill_ids)?, ..new };
        let mut taken = self.taken_handles()?;
        if let Some(reuse) = reuse_empty {
            let dir = self.bot_dir(reuse);
            let real_dir = std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir());
            let empty = real_dir && std::fs::read_dir(&dir).is_ok_and(|mut entries| entries.next().is_none());
            if empty && self.store.get_bot_by_handle(reuse)?.is_none() {
                taken.retain(|t| t != reuse);
            }
        }
        let handle = match wanted {
            Some(wanted) if is_valid_handle(wanted) && !taken.iter().any(|t| t == wanted) => wanted.to_string(),
            Some(wanted) => derive_handle(wanted, &taken),
            None => derive_handle(&new.display_name, &taken),
        };
        let dir = self.bot_dir(&handle);
        let existed = dir.exists();
        std::fs::create_dir_all(&dir)?;
        match insert(&new, &handle) {
            Ok(bot) => Ok(bot),
            Err(e) => {
                if !existed {
                    let _ = std::fs::remove_dir(&dir);
                }
                Err(e)
            }
        }
    }

    pub async fn list_bots(&self) -> Result<Vec<BotView>> {
        let master = self.master_id()?;
        let (mut bots, others): (Vec<Bot>, Vec<Bot>) =
            self.store.list_bots()?.into_iter().partition(|bot| master.as_deref() == Some(bot.id.as_str()));
        bots.extend(others);
        let live = self.inner.live.lock().await;
        Ok(bots
            .into_iter()
            .map(|bot| {
                let status = bot_status(&live, &bot.id);
                BotView { bot, status }
            })
            .collect())
    }

    pub async fn list_sessions(&self, bot_id: &str) -> Result<Vec<SessionView>> {
        let sessions = self.store.sessions_of(bot_id)?;
        let live = self.inner.live.lock().await;
        Ok(sessions.into_iter().map(|session| view_of(&live, session)).collect())
    }

    pub async fn all_sessions(&self) -> Result<Vec<SessionView>> {
        let mut all = Vec::new();
        for bot in self.store.list_bots()? {
            all.extend(self.store.sessions_of(&bot.id)?);
        }
        let live = self.inner.live.lock().await;
        Ok(all.into_iter().map(|session| view_of(&live, session)).collect())
    }

    pub fn user_thread(&self, bot_id: &str) -> Result<Vec<Message>> {
        let Some(session) = self.store.latest_session(bot_id)? else { return Ok(Vec::new()) };
        let thread = self.store.thread_for(USER, &session.id)?;
        self.store.thread_messages(&thread)
    }

    pub fn session_messages(&self, session_id: &str) -> Result<Vec<Message>> {
        self.store.get_session(session_id)?.ok_or_else(|| Refusal::SessionNotFound(session_id.into()))?;
        let thread = self.store.thread_for(USER, session_id)?;
        self.store.thread_messages(&thread)
    }

    pub async fn find_session_view(&self, session_id: &str) -> Result<SessionView> {
        let session = self.store.get_session(session_id)?.ok_or_else(|| Refusal::SessionNotFound(session_id.into()))?;
        let live = self.inner.live.lock().await;
        Ok(view_of(&live, session))
    }

    pub fn session_with_bot(&self, session_id: &str) -> Option<(Bot, Session)> {
        let session = self.store.get_session(session_id).ok().flatten()?;
        let bot = self.store.get_bot(&session.bot_id).ok().flatten()?;
        Some((bot, session))
    }

    pub fn sessions_of(&self, bot_id: &str) -> Result<Vec<Session>> {
        self.store.sessions_of(bot_id)
    }

    pub fn start_session_with_message(&self, bot_id: &str, body: &str, files: &[String]) -> Result<(Session, Message)> {
        let (id, new) = self.user_message(bot_id, None, body, files)?;
        let title = session_title(body);
        let routed = self.store.insert_routed(&id, &new, Route::New { title: title.as_deref() })?;
        self.emit_routed(&routed);
        Ok((routed.session, routed.message))
    }

    pub fn start_workflow_request(&self, body: &str) -> Result<(Session, Message)> {
        let master = self.master_bot()?.ok_or(Refusal::MasterMissing)?;
        let (id, new) = self.user_message(&master.id, None, body, &[])?;
        let title = session_title(body);
        let routed = self.store.insert_workflow_request(&id, &new, title.as_deref())?;
        self.emit_routed(&routed);
        Ok((routed.session, routed.message))
    }

    pub fn queue_user_message(&self, bot_id: &str, body: &str) -> Result<Message> {
        self.queue_user_message_with_files(bot_id, body, &[])
    }

    pub fn queue_user_message_with_files(&self, bot_id: &str, body: &str, files: &[String]) -> Result<Message> {
        let (id, new) = self.user_message(bot_id, None, body, files)?;
        let title = session_title(body);
        let routed = self.store.insert_routed(&id, &new, Route::Latest { title: title.as_deref() })?;
        self.emit_routed(&routed);
        Ok(routed.message)
    }

    pub fn queue_user_message_to_session(&self, session_id: &str, body: &str, files: &[String]) -> Result<Message> {
        let session = self.store.get_session(session_id)?.ok_or_else(|| Refusal::SessionNotFound(session_id.into()))?;
        if session.archived_at.is_some() {
            return Err(Refusal::SessionArchived(session.id).into());
        }
        let (id, new) = self.user_message(&session.bot_id, Some(session.id), body, files)?;
        let message = self.store.insert_message_with_id(&id, &new)?;
        self.emit_message(message.clone());
        Ok(message)
    }

    fn user_message(
        &self,
        bot_id: &str,
        session: Option<String>,
        body: &str,
        files: &[String],
    ) -> Result<(String, NewMessage)> {
        if body.trim().is_empty() && files.is_empty() {
            return Err(Refusal::MessageEmpty.into());
        }
        let bot = self.store.get_bot(bot_id)?.ok_or_else(|| Refusal::BotNotFound(bot_id.into()))?;
        let checked = attachments::check(files, Path::new("/")).map_err(Refusal::Attachment)?;
        let id = new_id();
        let stored = if checked.is_empty() {
            Vec::new()
        } else {
            let dir = self.for_project(&bot.project_id)?.inbox_bot_dir(&bot.handle)?;
            attachments::copy_to_inbox(&checked, &dir, &id).map_err(Refusal::Attachment)?
        };
        let new = NewMessage {
            sender: USER.into(),
            recipient: bot_id.into(),
            sender_session: None,
            recipient_session: session,
            kind: MessageKind::Text,
            body: body.to_string(),
            system_code: None,
            system_params: None,
            attachments: stored,
            status: MessageStatus::Queued,
            chain_id: new_id(),
            hop: 0,
            in_reply_to: None,
        };
        Ok((id, new))
    }

    fn inbox_bot_dir(&self, handle: &str) -> Result<PathBuf> {
        match self.checked_bot_dir(handle) {
            Ok(dir) => Ok(dir),
            Err(settings::FolderError::Outside(dir)) => {
                Err(Refusal::BotFolderOutside(dir.to_string_lossy().into_owned()).into())
            }
            Err(settings::FolderError::Io(e)) => Err(e.into()),
        }
    }

    pub fn attachment_path(&self, message_id: &str, index: usize) -> Result<PathBuf> {
        let message = self.store.get_message(message_id)?.ok_or_else(|| Refusal::MessageNotFound(message_id.into()))?;
        let file = message
            .attachments
            .get(index)
            .ok_or_else(|| Refusal::AttachmentNotFound { message: message_id.into(), index })?;
        Ok(PathBuf::from(&file.path))
    }

    pub fn pump(&self, bot_id: &str) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let engine = self.clone();
        let bot_id = bot_id.to_string();
        Box::pin(async move {
            let sessions = match engine.queued_sessions_of(&bot_id) {
                Ok(sessions) => sessions,
                Err(e) => {
                    tracing::error!(bot = %engine.handle_of(&bot_id), error = %e, "could not read the bot's queued sessions");
                    return;
                }
            };
            let pumps: Vec<_> =
                sessions.iter().map(|session_id| tokio::spawn(engine.pump_session(session_id))).collect();
            for pump in pumps {
                let _ = pump.await;
            }
        })
    }

    fn queued_sessions_of(&self, bot_id: &str) -> Result<Vec<String>> {
        let mine: HashSet<String> = self.store.sessions_of(bot_id)?.into_iter().map(|s| s.id).collect();
        Ok(self.store.sessions_with_queued()?.into_iter().filter(|id| mine.contains(id)).collect())
    }

    pub fn pump_session(&self, session_id: &str) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let engine = self.clone();
        let session_id = session_id.to_string();
        Box::pin(async move {
            let engine = match engine.for_session(&session_id) {
                Ok(engine) => engine,
                Err(_) => return,
            };
            if let Err(e) = engine.try_pump(&session_id).await {
                engine.notice(&session_id, "engine_error", error_params(&e));
            }
        })
    }

    async fn try_pump(&self, session_id: &str) -> Result<()> {
        if self.closing() || self.paused()? || self.current_project()?.archived_at.is_some() {
            return Ok(());
        }
        let Some(session) = self.store.get_session(session_id)? else { return Ok(()) };
        if session.archived_at.is_some() {
            return Ok(());
        }
        let Some(bot) = self.store.get_bot(&session.bot_id)? else { return Ok(()) };
        if self.held(bot.provider)? {
            return Ok(());
        }
        let (needs_start, stale) = {
            let mut live = self.inner.live.lock().await;
            if self.current_project()?.archived_at.is_some() {
                return Ok(());
            }
            if self.store.queued_for_session(session_id)?.is_empty() {
                return Ok(());
            }
            let l = live.entry(session_id.to_string()).or_insert_with(|| Live::new(&bot.id));
            if l.in_flight.is_some() || l.status == BotStatus::Starting || l.status == BotStatus::Limited {
                return Ok(());
            }
            let edited =
                l.started.is_some_and(|s| s.provider != bot.provider || s.config_version != bot.config_version);
            let stale = if edited { l.rt.take() } else { None };
            if l.rt.is_none() {
                l.status = BotStatus::Starting;
            }
            (l.rt.is_none(), stale)
        };
        if let Some(rt) = stale {
            tracing::info!(bot = %bot.handle, session = session.number, "restarting the session's CLI so it picks up the bot's new settings");
            if let Err(e) = self.inner.runtime.stop_bot(&rt).await {
                tracing::warn!(bot = %bot.handle, session = session.number, error = %e, "could not close the session's terminal for a restart");
            }
        }
        if needs_start {
            self.emit_status(session_id).await;
            match self.start(session_id).await {
                Ok(true) => {}
                Ok(false) => return Ok(()),
                Err(e) => {
                    self.set_status(session_id, BotStatus::NeedsAttention).await;
                    return Err(e);
                }
            }
        }
        self.deliver_next(session_id).await
    }

    fn start_lock(&self, bot_id: &str) -> Arc<Mutex<()>> {
        self.inner.start_locks.lock().unwrap().entry(bot_id.to_string()).or_default().clone()
    }

    async fn start(&self, session_id: &str) -> Result<bool> {
        let Some(bot_id) = self.store.get_session(session_id)?.map(|s| s.bot_id) else {
            return Ok(self.went_away_before_start(session_id).await);
        };
        let lock = self.start_lock(&bot_id);
        let _one_at_a_time = lock.lock().await;
        let Some(session) = self.store.get_session(session_id)?.filter(|s| s.archived_at.is_none()) else {
            return Ok(self.went_away_before_start(session_id).await);
        };
        let bot = self.store.get_bot(&bot_id)?.ok_or_else(|| Refusal::BotNotFound(bot_id.clone()))?;
        let Some(adapter) = adapter_for(bot.provider) else {
            self.set_status(session_id, BotStatus::NeedsAttention).await;
            self.notice(session_id, "provider_unsupported", json!({"provider": bot.provider.as_str()}));
            return Ok(false);
        };
        let dir = match self.checked_bot_dir(&bot.handle) {
            Ok(dir) => dir,
            Err(settings::FolderError::Io(e)) => {
                self.set_status(session_id, BotStatus::NeedsAttention).await;
                self.notice(session_id, "bot_start_failed", json!({"error": e.to_string()}));
                return Ok(false);
            }
            Err(settings::FolderError::Outside(dir)) => {
                tracing::warn!(bot = %bot.handle, session = session.number, folder = %dir.display(), "the bot's folder leads outside the bots folder");
                self.set_status(session_id, BotStatus::NeedsAttention).await;
                self.notice(session_id, "bot_folder_outside", json!({"path": dir.to_string_lossy()}));
                return Ok(false);
            }
        };
        let skills = self.skill_copies(&bot, &session)?;
        let language = self.language()?;
        let wrote = (dir.clone(), bot.config_version, language.clone());
        let current =
            self.inner.written.lock().unwrap().get(&bot_id) == Some(&wrote) && dir.join("AGENTS.md").is_file();
        if !current {
            if let Err(e) = write_bot_files(&dir, &bot, language_name(&language), adapter, &skills) {
                self.inner.written.lock().unwrap().remove(&bot_id);
                self.set_status(session_id, BotStatus::NeedsAttention).await;
                self.notice(session_id, "bot_start_failed", json!({"error": e.to_string()}));
                return Ok(false);
            }
            self.inner.written.lock().unwrap().insert(bot_id.clone(), wrote);
        }
        let mut spec = LaunchSpec {
            handle: bot.handle.clone(),
            agent: format!("s{}", session.id.to_lowercase()),
            label: format!("{}-{}", bot.handle, session.number),
            kind: adapter.herdr_kind().to_string(),
            cwd: dir,
            args: Vec::new(),
            env: adapter
                .launch_env(&bot)
                .into_iter()
                .chain([(
                    "SBOT_SOCKET".to_string(),
                    self.inner.cfg.paths.bridge_socket().to_string_lossy().into_owned(),
                )])
                .collect(),
            path_prefix: self.inner.cfg.sbot_dir.clone(),
            trust: adapter.trust_prompt(),
            sign_in: adapter.sign_in_text(),
        };
        let saved = session.provider_session_id.as_deref();
        let resume = saved.filter(|_| {
            adapter.rereads_context_on_resume() || session.session_config_version == Some(bot.config_version)
        });
        let (result, resumed) = match resume {
            Some(saved) => {
                spec.args = adapter.launch_args(&bot, Some(saved));
                let resumed = self.inner.runtime.start_bot(&spec).await;
                if matches!(resumed, Err(RuntimeError::StartFailed(_))) {
                    let fresh = self.start_fresh(&session.id, &bot, adapter, &mut spec).await?;
                    if fresh.is_ok() {
                        self.notice(session_id, "session_reset", json!({}));
                    }
                    (fresh, None)
                } else {
                    (resumed, Some(saved))
                }
            }
            None => (self.start_fresh(&session.id, &bot, adapter, &mut spec).await?, None),
        };
        match result {
            Ok(rt) => {
                if let Some(id) = rt.session_id.as_deref().or(resumed) {
                    self.store.set_session_cli_id(&session.id, Some(id), bot.config_version, bot.provider)?;
                }
                if saved.is_some() && resume.is_none() {
                    self.notice(session_id, "config_new_session", json!({}));
                }
                let gone = {
                    let mut live = self.inner.live.lock().await;
                    let went_away = match self.store.get_session(session_id) {
                        Ok(session) => session.is_none_or(|s| s.archived_at.is_some()),
                        Err(_) => false,
                    };
                    if went_away {
                        self.drop_live(&mut live, session_id);
                        Some(rt)
                    } else {
                        let l = live.entry(session_id.to_string()).or_insert_with(|| Live::new(&bot.id));
                        l.rt = Some(rt);
                        l.status = BotStatus::Idle;
                        l.idle_since = Some(tokio::time::Instant::now());
                        l.started = Some(Started { provider: bot.provider, config_version: bot.config_version });
                        l.usage_limit_noticed = false;
                        None
                    }
                };
                if let Some(rt) = gone {
                    tracing::info!(bot = %bot.handle, session = session.number, "the session went away while its CLI started; closing it");
                    if let Err(e) = self.inner.runtime.stop_bot(&rt).await {
                        tracing::warn!(bot = %bot.handle, session = session.number, error = %e, "could not close the terminal of a session that went away");
                    }
                    return Ok(false);
                }
                self.emit_status(session_id).await;
                Ok(true)
            }
            Err(e) => {
                self.set_status(session_id, BotStatus::NeedsAttention).await;
                self.notice(session_id, "bot_start_failed", json!({"error": e.to_string()}));
                Ok(false)
            }
        }
    }

    async fn went_away_before_start(&self, session_id: &str) -> bool {
        let bot_id = {
            let mut live = self.inner.live.lock().await;
            let bot_id = live.get(session_id).map(|l| l.bot_id.clone());
            self.drop_live(&mut live, session_id);
            bot_id
        };
        let bot = bot_id.map_or_else(String::new, |id| self.handle_of(&id));
        tracing::info!(bot = %bot, session_id, "the session went away before its CLI started");
        false
    }

    async fn start_fresh(
        &self,
        session_id: &str,
        bot: &Bot,
        adapter: &dyn ProviderAdapter,
        spec: &mut LaunchSpec,
    ) -> Result<std::result::Result<RuntimeBot, RuntimeError>> {
        let id = adapter.cli_session_id(&uuid::Uuid::new_v4().to_string());
        spec.args = adapter.launch_args(bot, None);
        let chosen = adapter.new_session_args(&id).map(|extra| {
            spec.args.extend(extra);
            id
        });
        let result = self.inner.runtime.start_bot(spec).await;
        if result.is_ok() {
            self.store.set_session_cli_id(session_id, chosen.as_deref(), bot.config_version, bot.provider)?;
        }
        Ok(result)
    }

    async fn deliver_next(&self, session_id: &str) -> Result<()> {
        let language = self.language()?;
        let (rt, msg, remind, text) = {
            let mut live = self.inner.live.lock().await;
            let Some(l) = live.get_mut(session_id) else { return Ok(()) };
            let Some(rt) = l.rt.clone() else { return Ok(()) };
            if self.paused()? {
                return Ok(());
            }
            if l.in_flight.is_some() {
                return Ok(());
            }
            let Some(msg) = self.next_to_deliver(session_id)? else {
                return Ok(());
            };
            let remind = l.remind;
            let text = self.envelope_for(session_id, &msg, &language, remind)?;
            l.in_flight = Some(InFlight {
                message_id: msg.id.clone(),
                sender: msg.sender.clone(),
                sender_session: msg.sender_session.clone(),
                chain_id: msg.chain_id.clone(),
                hop: msg.hop,
                in_reply_to: msg.in_reply_to.clone(),
                replied: false,
                seen_working: false,
                limit_noticed: false,
                delivered_at: None,
                enter_pressed_at: None,
                stopped: false,
                follow_up: false,
                after_limit: false,
                scheduled: msg.task_id.is_some(),
                from_run: msg.workflow_mark.is_some(),
            });
            l.status = BotStatus::Working;
            l.remind = false;
            l.waited_limit_lines.clear();
            l.prompt = Some(text.clone());
            (rt, msg, remind, text)
        };
        self.emit_status(session_id).await;
        self.send_claimed(session_id, rt, msg, remind, text).await
    }

    fn next_to_deliver(&self, session_id: &str) -> Result<Option<Message>> {
        loop {
            let Some(msg) = self.store.queued_for_session(session_id)?.into_iter().next() else {
                return Ok(None);
            };
            let stale = match &msg.workflow_mark {
                Some(mark) if mark.role == MarkRole::Reminder => self.store.visit_is_over(mark)?,
                _ => false,
            };
            if !stale {
                return Ok(Some(msg));
            }
            if let Some(cancelled) = self.store.cancel_queued(&msg.id)? {
                let who = self.who(session_id);
                tracing::info!(bot = %who.bot, session = who.session, message_id = %msg.id, "a workflow step's reminder came up after the step no longer waited for its result; cancelled, not delivered");
                self.emit(UiEvent::MessageUpdated { message: cancelled });
            }
        }
    }

    async fn send_claimed(
        &self,
        session_id: &str,
        rt: RuntimeBot,
        msg: Message,
        remind: bool,
        text: String,
    ) -> Result<()> {
        let failure: Option<(&'static str, String, Option<RuntimeError>)> = match self.store.mark_delivered(&msg.id) {
            Err(e) => Some(("engine_error", e.to_string(), None)),
            Ok(None) => {
                self.claim_lost(session_id, &msg.id, remind).await;
                return Ok(());
            }
            Ok(Some(delivered)) => {
                self.emit(UiEvent::MessageUpdated { message: delivered });
                if let Err(e) = self.store.touch_session(session_id) {
                    let who = self.who(session_id);
                    tracing::warn!(bot = %who.bot, session = who.session, error = %e, "could not record the session's activity");
                } else {
                    self.emit_session_updated(session_id);
                }
                self.prompt_with_retry(session_id, &msg.id, &rt, &text)
                    .await
                    .err()
                    .map(|e| ("prompt_failed", e.to_string(), Some(e)))
            }
        };
        let Some((code, error, prompt_error)) = failure else {
            {
                let mut live = self.inner.live.lock().await;
                if let Some(f) = live.get_mut(session_id).and_then(|l| l.in_flight.as_mut()) {
                    if f.message_id == msg.id {
                        f.delivered_at = Some(tokio::time::Instant::now());
                    }
                }
            }
            if let Some(grace) = self.inner.cfg.turn_start_grace {
                tokio::spawn(self.check_stalled_turn_after(grace, session_id, &msg.id));
            }
            return Ok(());
        };
        let who = self.who(session_id);
        tracing::warn!(bot = %who.bot, session = who.session, message_id = %msg.id, code, error = %error, "delivery failed");

        let gone = match prompt_error {
            Some(RuntimeError::Blocked) | None => false,
            Some(_) => matches!(self.inner.runtime.inspect(&rt).await, Err(RuntimeError::Gone)),
        };
        let status = if gone { BotStatus::Offline } else { BotStatus::NeedsAttention };

        let requeued = {
            let mut live = self.inner.live.lock().await;
            let still_ours =
                live.get(session_id).and_then(|l| l.in_flight.as_ref()).is_some_and(|f| f.message_id == msg.id);
            if !still_ours {
                return Ok(());
            }
            let l = live.get_mut(session_id).expect("checked above");
            l.in_flight = None;
            l.remind = remind;
            l.status = status;
            if gone {
                l.rt = None;
            }
            self.store.set_message_status(&msg.id, MessageStatus::Queued)
        };
        if let Ok(message) = requeued {
            self.emit(UiEvent::MessageUpdated { message });
        }
        if gone {
            let _ = self.inner.runtime.stop_bot(&rt).await;
        }
        self.emit_status(session_id).await;
        self.notice(session_id, code, json!({"error": error}));
        Ok(())
    }

    async fn claim_lost(&self, session_id: &str, message_id: &str, remind: bool) {
        let who = self.who(session_id);
        tracing::info!(bot = %who.bot, session = who.session, message_id, "the message was no longer queued when its delivery came; not sent");
        let released = {
            let mut live = self.inner.live.lock().await;
            match live.get_mut(session_id) {
                Some(l) if l.in_flight.as_ref().is_some_and(|f| f.message_id == message_id) => {
                    l.in_flight = None;
                    l.remind = remind;
                    l.status = BotStatus::Idle;
                    true
                }
                _ => false,
            }
        };
        if released {
            self.emit_status(session_id).await;
            tokio::spawn(self.pump_session(session_id));
        }
    }

    async fn prompt_with_retry(
        &self,
        session_id: &str,
        message_id: &str,
        rt: &RuntimeBot,
        text: &str,
    ) -> std::result::Result<(), RuntimeError> {
        match self.inner.runtime.prompt(rt, text).await {
            Err(RuntimeError::Stalled) => {
                tokio::time::sleep(STALL_RETRY_PAUSE).await;
                let landed = match self.inner.live.lock().await.get(session_id).and_then(|l| l.in_flight.as_ref()) {
                    Some(f) if f.message_id == message_id => f.seen_working || f.replied,
                    _ => return Err(RuntimeError::Stalled),
                };
                if landed {
                    return Ok(());
                }
                let state = match self.inner.runtime.inspect(rt).await {
                    Ok(inspection) => inspection.state,
                    Err(RuntimeError::Gone) => return Err(RuntimeError::Gone),
                    Err(e) => {
                        let who = self.who(session_id);
                        tracing::warn!(bot = %who.bot, session = who.session, message_id, error = %e, "could not inspect the session's CLI after a stalled prompt");
                        AgentState::Unknown
                    }
                };
                let who = self.who(session_id);
                tracing::info!(bot = %who.bot, session = who.session, message_id, ?state, "prompt stalled");
                match state {
                    AgentState::Idle => self.inner.runtime.prompt(rt, text).await,
                    AgentState::Working => {
                        let mut live = self.inner.live.lock().await;
                        if let Some(f) = live.get_mut(session_id).and_then(|l| l.in_flight.as_mut()) {
                            if f.message_id == message_id {
                                f.seen_working = true;
                            }
                        }
                        Ok(())
                    }
                    AgentState::Blocked | AgentState::Unknown => Ok(()),
                }
            }
            other => other,
        }
    }

    fn envelope_for(&self, session_id: &str, msg: &Message, language: &str, remind: bool) -> Result<String> {
        let session = self.store.get_session(session_id)?;
        let bot = match &session {
            Some(session) => self.store.get_bot(&session.bot_id)?,
            None => None,
        };
        let owed_to = match session.and_then(|s| s.open_request_id).filter(|_| msg.sender == USER) {
            Some(request) => self.requester_handle(&request)?,
            None => None,
        };
        let dir = match bot {
            Some(bot) => self.bot_directory(&bot),
            None => PathBuf::new(),
        };
        let files: Vec<String> = msg
            .attachments
            .iter()
            .map(|a| {
                Path::new(&a.path)
                    .strip_prefix(&dir)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| a.path.clone())
            })
            .collect();
        let sender_handle;
        let last_run;
        let started_by;
        let step_mark = msg.workflow_mark.as_ref().filter(|m| matches!(m.role, MarkRole::Request | MarkRole::Reminder));
        let (from, body) = if let Some(mark) = step_mark {
            started_by = self.run_starter(&mark.run_id)?;
            let from = Origin::Workflow {
                language: language_name(language),
                workflow: &mark.workflow,
                step: &mark.step,
                visit: mark.visit,
                started_by: started_by.as_deref(),
            };
            (from, msg.body.clone())
        } else if let Some(mark) = msg.workflow_mark.as_ref().filter(|m| m.role == MarkRole::Finished) {
            (Origin::WorkflowDone { language: language_name(language), workflow: &mark.workflow }, msg.body.clone())
        } else if let Some(task_id) = msg.task_id.as_deref() {
            let every_days = self.store.get_task(task_id)?.map(|t| t.every_days);
            last_run = msg.task_since.as_deref().map(|since| match chrono::DateTime::parse_from_rfc3339(since) {
                Ok(t) => local_minute(t.with_timezone(&chrono::Utc), &chrono::Local),
                Err(_) => since.to_string(),
            });
            let from = Origin::Task {
                language: language_name(language),
                name: msg.task_name.as_deref().unwrap_or_default(),
                every_days,
                last_run: last_run.as_deref(),
            };
            (from, msg.body.clone())
        } else if msg.workflow_request {
            (Origin::WorkflowRequest { language: language_name(language) }, msg.body.clone())
        } else if msg.sender == USER {
            (Origin::User { language: language_name(language) }, msg.body.clone())
        } else {
            sender_handle = self.store.get_bot(&msg.sender)?.map(|b| b.handle).unwrap_or_else(|| "unknown".into());
            let body = if msg.kind == MessageKind::RawFallback {
                raw_output_body(&sender_handle, &msg.body)
            } else {
                msg.body.clone()
            };
            (Origin::Bot { handle: &sender_handle }, body)
        };
        let mut text = envelope(from, &body, &files, remind, owed_to.as_deref());
        if step_mark.is_none() {
            if let Some((run, visit)) = self.store.open_visit_of_session(session_id)? {
                if let Some(step) = crate::workflow::step(&run.def, &visit.step_id) {
                    let outcomes = crate::workflow::outcomes(step);
                    text.push_str("\n\n");
                    text.push_str(&open_step_line(&run.name, step.name.trim(), &outcomes));
                }
            }
        }
        Ok(text)
    }

    pub fn bot_threads(&self) -> Result<Vec<ThreadSummary>> {
        self.store.bot_threads()
    }

    pub fn thread_messages(&self, thread_id: &str) -> Result<Vec<Message>> {
        self.store.thread_messages(thread_id)
    }

    pub async fn on_runtime_event(&self, event: RuntimeEvent) {
        let pane = match &event {
            RuntimeEvent::State { pane_id, .. } | RuntimeEvent::Exited { pane_id } => pane_id,
        };
        let session = {
            let live = self.inner.live.lock().await;
            session_for_pane(&live, pane)
        };
        let Some(engine) = session.and_then(|id| self.for_session(&id).ok()) else { return };
        match event {
            RuntimeEvent::State { pane_id, state } => engine.on_state(&pane_id, state).await,
            RuntimeEvent::Exited { pane_id } => engine.on_exited(&pane_id).await,
        }
    }

    async fn on_state(&self, pane_id: &str, state: AgentState) {
        enum Next {
            Nothing,
            Status(String),
            Blocked(String),
            EndTurn(String, String),
            Pump(String, bool),
            FollowUp(String, RuntimeBot),
            Resume(String, RuntimeBot),
        }
        let next = {
            let mut live = self.inner.live.lock().await;
            let Some(session_id) = session_for_pane(&live, pane_id) else { return };
            let l = live.get_mut(&session_id).expect("session found by pane");
            match state {
                AgentState::Working => match l.in_flight.as_mut() {
                    Some(f) => {
                        f.seen_working = true;
                        if l.status != BotStatus::Working && l.status != BotStatus::Limited {
                            l.status = BotStatus::Working;
                            Next::Status(session_id)
                        } else {
                            Next::Nothing
                        }
                    }
                    None if l.status == BotStatus::Idle => {
                        Next::FollowUp(session_id, l.rt.clone().expect("found by its pane"))
                    }
                    None if l.limited_turn.is_some() => {
                        Next::Resume(session_id, l.rt.clone().expect("found by its pane"))
                    }
                    None => Next::Nothing,
                },
                AgentState::Idle => match &l.in_flight {
                    Some(f) if f.seen_working => Next::EndTurn(session_id, f.message_id.clone()),
                    Some(_) => Next::Nothing,
                    None => {
                        let changed = l.status == BotStatus::NeedsAttention;
                        if changed {
                            l.status = BotStatus::Idle;
                            l.idle_since = Some(tokio::time::Instant::now());
                        }
                        Next::Pump(session_id, changed)
                    }
                },
                AgentState::Blocked => {
                    if matches!(l.status, BotStatus::NeedsAttention | BotStatus::Limited) {
                        Next::Nothing
                    } else {
                        l.status = BotStatus::NeedsAttention;
                        Next::Blocked(session_id)
                    }
                }
                AgentState::Unknown => Next::Nothing,
            }
        };
        match next {
            Next::Nothing => {}
            Next::Status(session_id) => self.emit_status(&session_id).await,
            Next::Blocked(session_id) => {
                self.emit_status(&session_id).await;
                self.notice(&session_id, "bot_blocked", json!({}));
            }
            Next::EndTurn(session_id, message_id) => self.end_turn(&session_id, &message_id, false).await,
            Next::FollowUp(session_id, rt) => self.open_follow_up(&session_id, &rt).await,
            Next::Resume(session_id, rt) => self.resume_on_its_own(&session_id, &rt).await,
            Next::Pump(session_id, changed) => {
                if changed {
                    self.emit_status(&session_id).await;
                }
                tokio::spawn(self.pump_session(&session_id));
            }
        }
    }

    pub async fn check_stalled_turn(&self, session_id: &str, message_id: &str) {
        let grace = self.inner.cfg.turn_start_grace.unwrap_or_default();
        let (rt, may_be_unsent) = {
            let live = self.inner.live.lock().await;
            let Some(l) = live.get(session_id) else { return };
            match (&l.rt, &l.in_flight) {
                (Some(rt), Some(f)) if !f.seen_working && f.message_id == message_id => {
                    if f.enter_pressed_at.is_some_and(|at| at.elapsed() < grace) {
                        return;
                    }
                    (rt.clone(), (!f.replied || f.after_limit) && f.enter_pressed_at.is_none())
                }
                _ => return,
            }
        };
        let inspection = match self.inner.runtime.inspect(&rt).await {
            Ok(inspection) => inspection,
            Err(RuntimeError::Gone) => return self.vanished_mid_turn(session_id, message_id, &rt).await,
            Err(e) => {
                let who = self.who(session_id);
                tracing::warn!(bot = %who.bot, session = who.session, message_id, error = %e, "turn watchdog could not inspect the session's CLI");
                return;
            }
        };
        match inspection.state {
            AgentState::Idle if may_be_unsent => self.press_enter(session_id, message_id, &rt).await,
            AgentState::Idle => self.end_turn(session_id, message_id, true).await,
            AgentState::Working => {
                let mut live = self.inner.live.lock().await;
                if let Some(f) = live.get_mut(session_id).and_then(|l| l.in_flight.as_mut()) {
                    if f.message_id == message_id {
                        f.seen_working = true;
                    }
                }
            }
            AgentState::Blocked => self.on_state(&rt.pane_id, AgentState::Blocked).await,
            AgentState::Unknown => {}
        }
    }

    async fn press_enter(&self, session_id: &str, message_id: &str, rt: &RuntimeBot) {
        {
            let mut live = self.inner.live.lock().await;
            match live.get_mut(session_id).and_then(|l| l.in_flight.as_mut()) {
                Some(f) if f.message_id == message_id && !f.seen_working && f.enter_pressed_at.is_none() => {
                    f.enter_pressed_at = Some(tokio::time::Instant::now())
                }
                _ => return,
            }
        }
        let who = self.who(session_id);
        if let Err(e) = self.inner.runtime.send_keys(rt, &["enter"]).await {
            tracing::warn!(bot = %who.bot, session = who.session, message_id, error = %e, "could not press Enter for a prompt the CLI did not start on");
            return self.end_turn(session_id, message_id, true).await;
        }
        tracing::info!(bot = %who.bot, session = who.session, message_id, "the CLI stayed idle after the prompt; pressed Enter in case the prompt sat unsent in its input");
        if let Some(grace) = self.inner.cfg.turn_start_grace {
            tokio::spawn(self.check_stalled_turn_after(grace, session_id, message_id));
        }
    }

    fn check_stalled_turn_after(
        &self,
        grace: Duration,
        session_id: &str,
        message_id: &str,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let engine = self.clone();
        let (session_id, message_id) = (session_id.to_string(), message_id.to_string());
        Box::pin(async move {
            tokio::time::sleep(grace).await;
            engine.check_stalled_turn(&session_id, &message_id).await;
        })
    }

    async fn vanished_mid_turn(&self, session_id: &str, message_id: &str, rt: &RuntimeBot) {
        let interrupted = {
            let mut live = self.inner.live.lock().await;
            let Some(l) = live.get_mut(session_id) else { return };
            let same_turn = l.in_flight.as_ref().is_some_and(|f| f.message_id == message_id)
                && l.rt.as_ref().is_some_and(|r| r.pane_id == rt.pane_id);
            if !same_turn {
                return;
            }
            let flight = l.in_flight.take();
            l.rt = None;
            l.status = BotStatus::Offline;
            flight
                .filter(|f| !f.follow_up)
                .map(|_| self.store.set_message_status(message_id, MessageStatus::Interrupted))
        };
        let who = self.who(session_id);
        match interrupted {
            Some(Ok(message)) => self.report_interrupted(message),
            Some(Err(e)) if removed_with_its_conversation(&e) => {
                tracing::info!(bot = %who.bot, session = who.session, message_id, "the message was removed with its conversation")
            }
            Some(Err(e)) => {
                tracing::error!(bot = %who.bot, session = who.session, message_id, error = %e, "could not mark the message interrupted")
            }
            None => {}
        }
        self.emit_status(session_id).await;
        tracing::info!(bot = %who.bot, session = who.session, message_id, "the session's CLI is gone mid-turn");
        let _ = self.inner.runtime.stop_bot(rt).await;
        tokio::spawn(self.pump_session(session_id));
    }

    async fn end_turn(&self, session_id: &str, message_id: &str, only_if_unseen: bool) {
        let matches = |l: &Live| {
            l.in_flight.as_ref().is_some_and(|f| f.message_id == message_id && !(only_if_unseen && f.seen_working))
        };
        let (rt, needs_answer, follow_up, stopped, provider, still_limited, prompt, waited, delivered_at) = {
            let live = self.inner.live.lock().await;
            let Some(l) = live.get(session_id) else { return };
            if !matches(l) {
                return;
            }
            let flight = l.in_flight.as_ref();
            (
                l.rt.clone(),
                flight.is_some_and(|f| !f.replied && !answers_a_bot(f)),
                flight.is_some_and(|f| f.follow_up),
                flight.is_some_and(|f| f.stopped),
                l.started.map(|s| s.provider),
                l.waits_for_limit(),
                l.prompt.clone(),
                l.waited_limit_lines.clone(),
                flight.and_then(|f| f.delivered_at),
            )
        };
        let same_turn = |l: &Live| matches(l) && l.in_flight.as_ref().is_some_and(|f| f.delivered_at == delivered_at);
        let (fallback_text, waiting, screen_busy) = {
            if still_limited && !stopped {
                let still = |l: &Live| same_turn(l) && l.waits_for_limit();
                return self.end_turn_on_limit(session_id, still, None).await;
            }
            let adapter = provider.and_then(adapter_for);
            let screen = match (&rt, adapter) {
                (Some(rt), Some(_)) => self.turn_end_screen(session_id, message_id, rt).await,
                _ => None,
            };
            let limit = match (adapter, &screen) {
                (Some(adapter), Some(screen)) if !stopped => {
                    adapter.usage_limit(screen, &Skip { prompt: prompt.as_deref(), waited: &waited })
                }
                _ => None,
            };
            if let (Some(limit), Some(provider)) = (limit, provider) {
                return self.end_turn_on_limit(session_id, same_turn, Some((provider, limit))).await;
            }
            let screen_busy = adapter.zip(screen.as_deref()).is_some_and(|(a, screen)| a.background_work(screen));
            let background_work = follow_up && screen_busy;
            let (fallback_text, waiting) = match rt {
                Some(rt) if needs_answer => {
                    if background_work {
                        let who = self.who(session_id);
                        tracing::info!(bot = %who.bot, session = who.session, message_id, "follow-up work ended with background work still running; nothing posted yet");
                        (None, true)
                    } else {
                        let text = match self.inner.runtime.read_output(&rt, self.inner.cfg.fallback_lines).await {
                            Ok(text) => text,
                            Err(e) => {
                                let who = self.who(session_id);
                                tracing::warn!(bot = %who.bot, session = who.session, message_id, error = %e, "could not read the terminal for the fallback");
                                String::new()
                            }
                        };
                        (Some(text), false)
                    }
                }
                _ => (None, false),
            };
            (fallback_text, waiting, screen_busy)
        };
        let (rt, flight) = {
            let mut live = self.inner.live.lock().await;
            let Some(l) = live.get_mut(session_id) else { return };
            if !same_turn(l) {
                return;
            }
            let flight = l.in_flight.take().expect("checked above");
            l.status = BotStatus::Idle;
            l.idle_since = Some(tokio::time::Instant::now());
            l.usage_limit_noticed = false;
            l.continue_at = None;
            if !flight.replied && !answers_a_bot(&flight) && !waiting {
                l.remind = true;
            }
            (l.rt.clone(), flight)
        };
        let who = self.who(session_id);
        if let Err(e) = self.store.mark_turn_ended(&flight.message_id) {
            tracing::error!(bot = %who.bot, session = who.session, message_id = %flight.message_id, error = %e, "could not record the end of the turn");
        }
        if flight.scheduled && !flight.stopped && !flight.follow_up {
            if let Err(e) = self.store.mark_task_done(&flight.message_id) {
                tracing::error!(bot = %who.bot, session = who.session, message_id = %flight.message_id, error = %e, "could not record that the recurring task's run completed");
            }
        }
        self.emit_status(session_id).await;
        if !flight.replied && answers_a_bot(&flight) {
            tracing::info!(bot = %who.bot, session = who.session, message_id = %flight.message_id, "turn on another bot's answer ended without a message to it; an answer needs no answer, so no fallback");
        }
        if let Some(rt) = &rt {
            if let (Some(text), false) = (fallback_text, flight.replied) {
                self.post_fallback(session_id, &text, &flight);
            }
            self.save_session_id(session_id, &flight.message_id, rt).await;
        }
        self.after_step_turn(session_id, &flight, screen_busy).await;
        tokio::spawn(self.pump_session(session_id));
    }

    fn post_fallback(&self, session_id: &str, text: &str, flight: &InFlight) {
        let session = match self.store.get_session(session_id) {
            Ok(Some(session)) if session.archived_at.is_some() => {
                tracing::info!(bot = %self.handle_of(&session.bot_id), session = session.number, message_id = %flight.message_id, "the session was archived; no fallback");
                return;
            }
            Ok(Some(session)) => session,
            Ok(None) => return,
            Err(e) => {
                tracing::error!(session_id, message_id = %flight.message_id, error = %e, "could not read the session for the fallback");
                return;
            }
        };
        let who = Who { bot: self.handle_of(&session.bot_id), session: session.number };
        if flight.stopped && flight.sender != USER {
            tracing::info!(bot = %who.bot, session = who.session, message_id = %flight.message_id, "turn stopped by the user; the asking bot gets no fallback");
            return;
        }
        if flight.sender != USER {
            match self.store.get_bot(&flight.sender) {
                Ok(Some(_)) => {}
                Ok(None) => {
                    tracing::info!(bot = %who.bot, session = who.session, message_id = %flight.message_id, "the asking bot was deleted; no fallback");
                    return;
                }
                Err(e) => {
                    tracing::error!(bot = %who.bot, session = who.session, message_id = %flight.message_id, error = %e, "could not read the asking bot")
                }
            }
        }
        if let Some(asker) = flight.sender_session.as_deref() {
            match self.store.get_session(asker) {
                Ok(Some(asker)) if asker.archived_at.is_some() => {
                    tracing::info!(bot = %who.bot, session = who.session, message_id = %flight.message_id, "the asking session was archived; no fallback");
                    return;
                }
                Ok(Some(_)) => {}
                Ok(None) => {
                    tracing::info!(bot = %who.bot, session = who.session, message_id = %flight.message_id, "the asking session was removed; no fallback");
                    return;
                }
                Err(e) => {
                    tracing::error!(bot = %who.bot, session = who.session, message_id = %flight.message_id, error = %e, "could not read the asking session")
                }
            }
        }
        let to_user = flight.sender == USER;
        let hop = flight.hop + 1;
        if to_user && text.trim().is_empty() {
            let notice = NewMessage {
                chain_id: flight.chain_id.clone(),
                hop,
                in_reply_to: Some(flight.message_id.clone()),
                ..NewMessage::system(&session, "no_output_captured", json!({}))
            };
            match self.store.insert_message(&notice) {
                Ok(message) => self.emit(UiEvent::MessageAdded { message }),
                Err(e) => {
                    tracing::error!(bot = %who.bot, session = who.session, message_id = %flight.message_id, error = %e, "could not store the fallback notice")
                }
            }
            return;
        }
        let body = if text.trim().is_empty() {
            self.notice(session_id, "no_output_captured", json!({}));
            silent_turn_note(&who.bot)
        } else {
            text.trim_end().to_string()
        };
        let mut status = if to_user { MessageStatus::Delivered } else { MessageStatus::Queued };
        if !to_user {
            match self.limit_for(&flight.chain_id, &session.bot_id, &flight.sender) {
                Ok(Some(limit)) if hop > limit => {
                    if !flight.limit_noticed {
                        self.notice(session_id, "chain_limit_reached", json!({"limit": limit.to_string()}));
                    }
                    status = MessageStatus::Failed;
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::error!(bot = %who.bot, session = who.session, error = %e, "could not read the hop limit")
                }
            }
        }
        let new = NewMessage {
            sender: session.bot_id.clone(),
            recipient: flight.sender.clone(),
            sender_session: Some(session.id.clone()),
            recipient_session: flight.sender_session.clone(),
            kind: MessageKind::RawFallback,
            body,
            system_code: None,
            system_params: None,
            attachments: vec![],
            status,
            chain_id: flight.chain_id.clone(),
            hop,
            in_reply_to: Some(flight.message_id.clone()),
        };
        match self.store.insert_message(&new) {
            Ok(message) => self.emit_message(message),
            Err(e) => {
                tracing::error!(bot = %who.bot, session = who.session, message_id = %flight.message_id, error = %e, "could not store the fallback answer");
                return;
            }
        }
        if let (MessageStatus::Queued, Some(asker)) = (status, &flight.sender_session) {
            tokio::spawn(self.pump_session(asker));
        }
    }

    async fn save_session_id(&self, session_id: &str, message_id: &str, rt: &RuntimeBot) {
        let Some(started) = self.inner.live.lock().await.get(session_id).and_then(|l| l.started) else { return };
        let inspection = match self.inner.runtime.inspect(rt).await {
            Ok(inspection) => inspection,
            Err(e) => {
                let who = self.who(session_id);
                tracing::warn!(bot = %who.bot, session = who.session, message_id, error = %e, "could not read the session id at the end of the turn");
                return;
            }
        };
        let Some(id) = inspection.session_id else { return };
        let Some((session, bot)) = self.cli_id_target(session_id, started, Some(message_id)).await else { return };
        if session.provider_session_id.as_deref() != Some(id.as_str()) {
            if let Err(e) =
                self.store.set_session_cli_id(&session.id, Some(&id), started.config_version, started.provider)
            {
                tracing::error!(bot = %bot.handle, session = session.number, message_id, error = %e, "could not save the session id");
            }
        }
    }

    async fn cli_id_target(
        &self,
        session_id: &str,
        started: Started,
        message_id: Option<&str>,
    ) -> Option<(Session, Bot)> {
        let session = match self.store.get_session(session_id) {
            Ok(Some(session)) => session,
            Ok(None) => return None,
            Err(e) => {
                let who = self.who(session_id);
                tracing::error!(bot = %who.bot, session = who.session, session_id, message_id, error = %e, "could not read the session to save its CLI session id");
                return None;
            }
        };
        let bot = match self.store.get_bot(&session.bot_id) {
            Ok(Some(bot)) => bot,
            Ok(None) => return None,
            Err(e) => {
                tracing::error!(bot = %self.handle_of(&session.bot_id), session = session.number, message_id, error = %e, "could not read the bot to save its CLI session id");
                return None;
            }
        };
        (bot.provider == started.provider).then_some((session, bot))
    }

    async fn fill_session_id_from_sbot(&self, pane_id: &str, cli_id: &str) {
        let cli_id = cli_id.trim();
        if cli_id.is_empty() {
            return;
        }
        let (session_id, started) = {
            let live = self.inner.live.lock().await;
            let Some(session_id) = session_for_pane(&live, pane_id) else { return };
            let Some(started) = live.get(&session_id).and_then(|l| l.started) else { return };
            (session_id, started)
        };
        if !adapter_for(started.provider).is_some_and(|a| a.session_id_from_sbot()) {
            return;
        }
        let Some((session, bot)) = self.cli_id_target(&session_id, started, None).await else { return };
        if session.provider_session_id.is_some() {
            return;
        }
        match self.store.fill_session_cli_id(&session.id, cli_id, started.config_version, started.provider) {
            Ok(true) => {
                tracing::info!(bot = %bot.handle, session = session.number, "saved the session id sbot reported")
            }
            Ok(false) => {}
            Err(e) => {
                tracing::error!(bot = %bot.handle, session = session.number, error = %e, "could not save the session id sbot reported")
            }
        }
    }

    async fn on_exited(&self, pane_id: &str) {
        let (session_id, flight) = {
            let mut live = self.inner.live.lock().await;
            let Some(session_id) = session_for_pane(&live, pane_id) else { return };
            let l = live.get_mut(&session_id).expect("session found by pane");
            l.rt = None;
            l.status = BotStatus::Offline;
            (session_id, l.in_flight.take().or_else(|| l.limited_turn.take()))
        };
        self.emit_status(&session_id).await;
        let who = self.who(&session_id);
        tracing::info!(bot = %who.bot, session = who.session, "the session's terminal closed");
        if let Some(f) = flight.filter(|f| !f.follow_up && !self.closing()) {
            match self.store.set_message_status(&f.message_id, MessageStatus::Interrupted) {
                Ok(message) => self.report_interrupted(message),
                Err(e) if removed_with_its_conversation(&e) => {
                    tracing::info!(bot = %who.bot, session = who.session, message_id = %f.message_id, "the message was removed with its conversation")
                }
                Err(e) => {
                    tracing::error!(bot = %who.bot, session = who.session, message_id = %f.message_id, error = %e, "could not mark the message interrupted")
                }
            }
        }
        tokio::spawn(self.pump_session(&session_id));
    }

    pub async fn shutdown(&self) {
        self.inner.closing.store(true, Ordering::SeqCst);
        self.inner.runtime.shutdown().await;
    }

    fn closing(&self) -> bool {
        self.inner.closing.load(Ordering::SeqCst)
    }

    fn handle_of(&self, bot_id: &str) -> String {
        self.store.get_bot(bot_id).ok().flatten().map(|b| b.handle).unwrap_or_else(|| bot_id.to_string())
    }

    fn who(&self, session_id: &str) -> Who {
        match self.store.get_session(session_id) {
            Ok(Some(s)) => Who { bot: self.handle_of(&s.bot_id), session: s.number },
            _ => Who { bot: session_id.to_string(), session: 0 },
        }
    }

    fn skill_copies(&self, bot: &Bot, session: &Session) -> Result<Vec<SkillCopy>> {
        let library = self.inner.cfg.paths.skills_dir();
        let mut copies = Vec::new();
        for s in self.store.skills_of(&bot.id)? {
            let source_dir = library.join(&s.slug);
            if !source_dir.is_dir() {
                tracing::warn!(
                    bot = %bot.handle,
                    session = session.number,
                    skill = %s.slug,
                    folder = %source_dir.display(),
                    "the skill's library folder is missing; the bot starts without it"
                );
                let params = json!({"name": s.name, "path": source_dir.to_string_lossy()});
                self.notice(&session.id, "skill_folder_missing", params);
                continue;
            }
            copies.push(SkillCopy { source_dir, slug: s.slug, name: s.name, description: s.description });
        }
        Ok(copies)
    }

    fn emit(&self, event: UiEvent) {
        let _ = self.inner.ui.send(event);
    }

    fn emit_message(&self, message: Message) {
        self.emit_activity(&message, true);
        self.emit(UiEvent::MessageAdded { message });
    }

    fn emit_activity(&self, message: &Message, recipient: bool) {
        if message.status == MessageStatus::Failed {
            return;
        }
        if message.kind != MessageKind::System {
            if let Some(session) = &message.sender_session {
                self.emit_session_updated(session);
            }
        }
        if let (Some(session), true) = (&message.recipient_session, recipient) {
            self.emit_session_updated(session);
        }
    }

    fn emit_routed(&self, routed: &Routed) {
        if routed.created {
            self.emit_activity(&routed.message, false);
            self.emit(UiEvent::SessionAdded { session: self.session_view(routed.session.clone()) });
            self.emit(UiEvent::MessageAdded { message: routed.message.clone() });
        } else {
            self.emit_message(routed.message.clone());
        }
    }

    fn emit_session_updated(&self, session_id: &str) {
        match self.store.get_session(session_id) {
            Ok(Some(session)) => self.emit(UiEvent::SessionUpdated { session: self.session_view(session) }),
            Ok(None) => {}
            Err(e) => tracing::error!(session_id, error = %e, "could not read a session to tell the UI"),
        }
    }

    fn session_view(&self, session: Session) -> SessionView {
        let status = self.inner.shown_sessions.lock().unwrap().get(&session.id).map_or(BotStatus::Offline, |s| s.1);
        SessionView { session, status }
    }

    async fn emit_status(&self, session_id: &str) {
        let live = self.inner.live.lock().await;
        if let Some(bot_id) = live.get(session_id).map(|l| l.bot_id.clone()) {
            self.publish_status(&live, &bot_id);
        }
    }

    fn publish_status(&self, live: &HashMap<String, Live>, bot_id: &str) {
        self.publish_session_statuses(live, bot_id);
        let status = bot_status(live, bot_id);
        let mut shown = self.inner.shown.lock().unwrap();
        if shown.get(bot_id).copied().unwrap_or(BotStatus::Offline) != status {
            shown.insert(bot_id.to_string(), status);
            self.emit(UiEvent::BotStatus { bot_id: bot_id.to_string(), status });
        }
    }

    fn publish_session_statuses(&self, live: &HashMap<String, Live>, bot_id: &str) {
        let mut shown = self.inner.shown_sessions.lock().unwrap();
        let gone: Vec<String> = shown
            .iter()
            .filter(|(id, (b, _))| b == bot_id && !live.contains_key(*id))
            .map(|(id, _)| id.clone())
            .collect();
        for id in gone {
            if let Some((_, BotStatus::Offline)) | None = shown.remove(&id) {
                continue;
            }
            self.emit(UiEvent::SessionStatus {
                session_id: id,
                bot_id: bot_id.to_string(),
                status: BotStatus::Offline,
            });
        }
        for (id, l) in live.iter().filter(|(_, l)| l.bot_id == bot_id) {
            let before = shown.get(id).map_or(BotStatus::Offline, |s| s.1);
            shown.insert(id.clone(), (bot_id.to_string(), l.status));
            if before != l.status {
                self.emit(UiEvent::SessionStatus {
                    session_id: id.clone(),
                    bot_id: bot_id.to_string(),
                    status: l.status,
                });
            }
        }
    }

    async fn set_status(&self, session_id: &str, status: BotStatus) {
        let mut live = self.inner.live.lock().await;
        let Some(l) = live.get_mut(session_id) else { return };
        l.status = status;
        let bot_id = l.bot_id.clone();
        self.publish_status(&live, &bot_id);
    }

    fn report_interrupted(&self, message: Message) {
        if let Some(sender_session) = message.sender_session.as_deref().filter(|_| message.sender != USER) {
            let to = self.handle_of(&message.recipient);
            self.notice(sender_session, "bot_message_interrupted", json!({"to": to}));
        }
        self.emit(UiEvent::MessageUpdated { message });
    }

    fn notice(&self, session_id: &str, code: &str, params: Value) {
        if self.closing() {
            return;
        }
        let session = match self.store.get_session(session_id) {
            Ok(Some(session)) if session.archived_at.is_some() => {
                let bot = self.handle_of(&session.bot_id);
                tracing::info!(bot = %bot, session = session.number, code, params = %params, "no notice for an archived session");
                return;
            }
            Ok(Some(session)) => session,
            Ok(None) => {
                tracing::info!(session_id, code, params = %params, "no notice for a session that is gone");
                return;
            }
            Err(e) => {
                tracing::error!(session_id, code, params = %params, error = %e, "could not read the session for a notice");
                return;
            }
        };
        let bot = self.handle_of(&session.bot_id);
        tracing::warn!(bot = %bot, session = session.number, code, params = %params, "notice for the user");
        match self.store.insert_message(&NewMessage::system(&session, code, params)) {
            Ok(message) => self.emit(UiEvent::MessageAdded { message }),
            Err(e) => {
                tracing::error!(bot = %bot, session = session.number, code, error = %e, "could not store a notice")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_engine_error_notice_carries_the_refusal_code() {
        let p = error_params(&CoreError::Refused(Refusal::BotStarting));
        assert_eq!(p["code"], "botStarting");
        assert_eq!(p["error"], "the bot is starting; try again in a moment");
        let p = error_params(&CoreError::Refused(Refusal::StopFailed("x".into())));
        assert_eq!(p["code"], "stopFailed");
        assert_eq!(p["error"], "x");
        let p = error_params(&CoreError::Io(std::io::Error::other("disk")));
        assert_eq!(p, json!({ "error": "file error: disk" }));
    }

    #[test]
    fn a_session_is_titled_by_the_first_line_of_its_first_message() {
        assert_eq!(session_title("\n \n  Review the login flow  \nDetails.").as_deref(), Some("Review the login flow"));
        let sixty = "a".repeat(60);
        assert_eq!(session_title(&sixty), Some(sixty.clone()), "60 characters fit");
        assert_eq!(session_title(&format!("{sixty}b")), Some(format!("{}…", "a".repeat(59))));
        assert_eq!(session_title(&"ü".repeat(61)), Some(format!("{}…", "ü".repeat(59))), "characters, not bytes");
        assert_eq!(session_title(""), None);
        assert_eq!(session_title(" \n\t\n"), None);
    }

    #[test]
    fn a_bot_shows_the_status_of_its_session_that_matters_most() {
        use BotStatus::*;
        assert_eq!(aggregate([]), Offline, "no session runs");
        assert_eq!(aggregate([Offline, Idle]), Idle);
        assert_eq!(aggregate([Idle, Starting, Offline]), Starting);
        assert_eq!(aggregate([Starting, Working, Idle]), Working);
        assert_eq!(aggregate([Working, NeedsAttention, Starting]), NeedsAttention);
        assert_eq!(aggregate([NeedsAttention, Offline]), NeedsAttention);
        assert_eq!(aggregate([Idle, Limited, Starting]), Limited);
        assert_eq!(aggregate([Limited, Working]), Working);
    }
}
