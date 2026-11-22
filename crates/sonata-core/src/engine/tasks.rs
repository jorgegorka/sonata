use super::master::on_users_turn;
use super::messaging::{internal, refuse};
use super::*;
use crate::schedule::{
    default_start, format_at, local_date, local_minute, parse_at, parse_date, Schedule, DEFAULT_AT, EVERY_MAX,
    EVERY_MIN, TASK_NAME_MAX,
};
use crate::store::{CreationKind, NewCreation, Scheduled};
use bridge_proto::{BridgeResponse, ErrorCode};
use chrono::{DateTime, Local, SecondsFormat, Utc};
use sha2::{Digest, Sha256};

pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

pub(super) fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

impl Engine {
    pub fn set_clock(&self, clock: Clock) {
        *self.inner.clock.write().unwrap() = clock;
    }

    pub(crate) fn now_utc(&self) -> DateTime<Utc> {
        (self.inner.clock.read().unwrap())()
    }

    pub fn list_tasks(&self) -> Result<Vec<Task>> {
        self.store.list_tasks()
    }

    pub fn tasks_of_bot(&self, bot_id: &str) -> Result<Vec<Task>> {
        self.store.tasks_of_bot(bot_id)
    }

    pub fn create_task(&self, new: NewTask) -> Result<Task> {
        self.store.require_active()?;
        let row = self.checked_task(new, None)?;
        let task = self.store.insert_task(&row)?;
        self.emit(UiEvent::TasksChanged);
        Ok(task)
    }

    pub fn update_task(&self, id: &str, edit: NewTask) -> Result<Task> {
        self.store.require_active()?;
        let current = self.store.get_task(id)?.ok_or_else(|| Refusal::TaskNotFound(id.to_string()))?;
        let row = self.checked_task(edit, Some(&current))?;
        let task = self.store.update_task(id, &row)?.ok_or_else(|| Refusal::TaskNotFound(id.to_string()))?;
        self.emit(UiEvent::TasksChanged);
        Ok(task)
    }

    pub fn set_task_enabled(&self, id: &str, enabled: bool) -> Result<Task> {
        self.store.require_active()?;
        let current = self.store.get_task(id)?.ok_or_else(|| Refusal::TaskNotFound(id.to_string()))?;
        let next_run_at = if enabled {
            Some(stamp(Self::schedule_of(&current)?.first_slot_after(self.now_utc(), &Local)))
        } else {
            None
        };
        let row = TaskRow {
            name: current.name,
            bot_id: current.bot_id,
            prompt: current.prompt,
            every_days: current.every_days,
            at: current.at,
            starts_on: current.starts_on,
            enabled,
            next_run_at,
        };
        let task = self.store.update_task(id, &row)?.ok_or_else(|| Refusal::TaskNotFound(id.to_string()))?;
        self.emit(UiEvent::TasksChanged);
        Ok(task)
    }

    pub fn delete_task(&self, id: &str) -> Result<Task> {
        self.store.require_active()?;
        let task = self.store.delete_task(id)?.ok_or_else(|| Refusal::TaskNotFound(id.to_string()))?;
        self.emit(UiEvent::TasksChanged);
        Ok(task)
    }

    pub fn run_scheduler(&self, every: Duration) -> tokio::task::JoinHandle<()> {
        let engine = self.clone();
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(every);
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                timer.tick().await;
                if engine.closing() {
                    break;
                }
                engine.scheduler_tick().await;
            }
        })
    }

    pub async fn scheduler_tick(&self) {
        let now = self.now_utc();
        let due = match self.store.tasks_due(&stamp(now)) {
            Ok(due) => due,
            Err(e) => {
                tracing::error!(error = %e, "could not read the recurring tasks that are due");
                return;
            }
        };
        let due: Vec<Task> = due.into_iter().filter(|task| !self.task_held(task)).collect();
        if due.is_empty() {
            return;
        }
        for task in due {
            if self.closing() {
                break;
            }
            let engine = match self.for_project(&task.project_id) {
                Ok(engine) => engine,
                Err(_) => continue,
            };
            if let Err(e) = engine.run_due(&task, now).await {
                tracing::error!(bot = %self.handle_of(&task.bot_id), task = %task.name, error = %e, "could not run the recurring task");
            }
        }
        self.emit(UiEvent::TasksChanged);
    }

    pub fn run_task_now(&self, id: &str) -> Result<Session> {
        self.store.require_active()?;
        let task = self.store.get_task(id)?.ok_or_else(|| Refusal::TaskNotFound(id.to_string()))?;
        let session = self.start_run(&task, None)?.ok_or_else(|| Refusal::TaskNotFound(id.to_string()))?;
        self.emit(UiEvent::TasksChanged);
        Ok(session)
    }

    async fn run_due(&self, task: &Task, now: DateTime<Utc>) -> Result<()> {
        let schedule = Self::schedule_of(task)?;
        let next = stamp(schedule.first_slot_after(now, &Local));
        let slot = Scheduled { read: task, next_run_at: &next };
        if self.last_run_busy(task).await? {
            let skipped = match schedule.last_slot_at_or_before(now, &Local) {
                Some(slot) => stamp(slot),
                None => task.next_run_at.clone().unwrap_or_else(|| stamp(now)),
            };
            if self.store.skip_task_slot(slot, &skipped)?.is_some() {
                tracing::info!(bot = %self.handle_of(&task.bot_id), task = %task.name, slot = %skipped, "skipped the recurring task's slot: its last run is still busy");
            }
            return Ok(());
        }
        self.start_run(task, Some(slot))?;
        Ok(())
    }

    fn task_held(&self, task: &Task) -> bool {
        let held = match self.store.get_bot(&task.bot_id) {
            Ok(Some(bot)) => self.held(bot.provider),
            Ok(None) => Ok(false),
            Err(e) => Err(e),
        };
        held.unwrap_or_else(|e| {
            tracing::error!(bot = %self.handle_of(&task.bot_id), task = %task.name, error = %e, "could not read whether the recurring task's provider is held");
            false
        })
    }

    async fn last_run_busy(&self, task: &Task) -> Result<bool> {
        let Some(session_id) = task.last_run_session.as_deref() else { return Ok(false) };
        let Some(session) = self.store.get_session(session_id)? else { return Ok(false) };
        if session.archived_at.is_some() {
            return Ok(false);
        }
        if session.waiting_question_id.is_some() {
            return Ok(true);
        }
        let in_flight = self
            .inner
            .live
            .lock()
            .await
            .get(session_id)
            .is_some_and(|l| l.in_flight.is_some() || l.status == BotStatus::Limited);
        Ok(in_flight || !self.store.queued_for_session(session_id)?.is_empty())
    }

    fn start_run(&self, task: &Task, slot: Option<Scheduled<'_>>) -> Result<Option<Session>> {
        let title = format!("{} · {}", task.name, local_date(self.now_utc(), &Local).format("%Y-%m-%d"));
        let since = self.store.last_completed_run(&task.id)?;
        let Some((routed, task)) = self.store.start_task_run(&task.id, &title, &task.prompt, since.as_deref(), slot)?
        else {
            return Ok(None);
        };
        self.emit_routed(&routed);
        let bot = self.handle_of(&task.bot_id);
        tracing::info!(bot = %bot, session = routed.session.number, task = %task.name, "started a run of the recurring task");
        tokio::spawn(self.pump_session(&routed.session.id));
        Ok(Some(routed.session))
    }

    pub(super) fn schedule_of(task: &Task) -> Result<Schedule> {
        let at = parse_at(&task.at).ok_or_else(|| Refusal::TaskTimeInvalid(task.at.clone()))?;
        let starts_on = parse_date(&task.starts_on).ok_or_else(|| Refusal::TaskStartInvalid(task.starts_on.clone()))?;
        Ok(Schedule { every_days: task.every_days.max(EVERY_MIN), at, starts_on })
    }

    pub(super) fn checked_task(&self, new: NewTask, editing: Option<&Task>) -> Result<TaskRow> {
        let name = new.name.trim().to_string();
        if name.is_empty() {
            return Err(Refusal::TaskNameRequired.into());
        }
        if name.chars().count() > TASK_NAME_MAX {
            return Err(Refusal::TaskNameTooLong { max: TASK_NAME_MAX }.into());
        }
        if new.prompt.trim().is_empty() {
            return Err(Refusal::TaskPromptRequired.into());
        }
        if !(EVERY_MIN..=EVERY_MAX).contains(&new.every_days) {
            return Err(Refusal::TaskEveryRange { min: EVERY_MIN, max: EVERY_MAX }.into());
        }
        let at = parse_at(new.at.trim()).ok_or_else(|| Refusal::TaskTimeInvalid(new.at.clone()))?;
        let given_start = match new.starts_on.as_deref() {
            Some(text) => Some(parse_date(text.trim()).ok_or_else(|| Refusal::TaskStartInvalid(text.to_string()))?),
            None => None,
        };
        if self.store.get_bot(&new.bot_id)?.is_none() {
            return Err(Refusal::BotNotFound(new.bot_id).into());
        }
        if let Some(other) = self.store.get_task_by_name(&name)? {
            if editing.map(|t| t.id.as_str()) != Some(other.id.as_str()) {
                return Err(Refusal::TaskNameTaken(name).into());
            }
        }

        let now = self.now_utc();
        let enabled = editing.is_none_or(|t| t.enabled);
        let kept_start = editing.and_then(|t| match t.next_run_at.as_deref() {
            Some(next) => {
                let next = DateTime::parse_from_rfc3339(next).ok()?.with_timezone(&Utc);
                Some(local_date(next, &Local))
            }
            None => parse_date(&t.starts_on),
        });
        let starts_on = given_start.or(kept_start).unwrap_or_else(|| default_start(at, now, &Local));
        let schedule = Schedule { every_days: new.every_days, at, starts_on };
        let next_run_at = enabled.then(|| stamp(schedule.first_slot_at_or_after(now, &Local)));
        Ok(TaskRow {
            name,
            bot_id: new.bot_id,
            prompt: new.prompt,
            every_days: new.every_days,
            at: format_at(at),
            starts_on: starts_on.format("%Y-%m-%d").to_string(),
            enabled,
            next_run_at,
        })
    }
}

fn task_fingerprint(bot: &str, every: u32, at: &str, from: &str, prompt: &str) -> String {
    let mut hash = Sha256::new();
    for field in [bot, &every.to_string(), at, from, prompt] {
        hash.update(field.as_bytes());
        hash.update([0]);
    }
    format!("{:x}", hash.finalize())
}

const CHANGE_ONLY_ON_USERS_TURN: &str =
    "Create, change and delete recurring tasks only while answering the user's message.";

pub(super) fn bad_request(e: CoreError) -> BridgeResponse {
    match e {
        CoreError::Refused(why) => refuse(ErrorCode::BadRequest, why.to_string()),
        other => internal(other),
    }
}

impl Engine {
    #[allow(clippy::result_large_err)]
    fn named_task(&self, name: &str) -> std::result::Result<Task, BridgeResponse> {
        match self.store.get_task_by_name(name.trim()) {
            Ok(Some(task)) => Ok(task),
            Ok(None) => Err(refuse(
                ErrorCode::BadRequest,
                format!("No recurring task named {}; run `sbot tasks`.", name.trim()),
            )),
            Err(e) => Err(internal(e)),
        }
    }

    #[allow(clippy::result_large_err)]
    pub(super) fn bot_named(&self, handle: &str) -> std::result::Result<Bot, BridgeResponse> {
        let handle = handle.trim().trim_start_matches('@').to_lowercase();
        match self.store.get_bot_by_handle(&handle) {
            Ok(Some(bot)) => Ok(bot),
            Ok(None) => Err(refuse(
                ErrorCode::UnknownRecipient,
                format!("No bot named @{handle}. Run `sbot bots` to see your teammates."),
            )),
            Err(e) => Err(internal(e)),
        }
    }

    fn task_block(&self, task: &Task) -> Result<String> {
        let store = &self.store;
        let minute = |stamp: &str| {
            DateTime::parse_from_rfc3339(stamp)
                .map(|t| local_minute(t.with_timezone(&Utc), &Local))
                .unwrap_or_else(|_| stamp.to_string())
        };
        let days = if task.every_days == 1 { "day" } else { "days" };
        let mut text = format!(
            "{}: @{}, every {} {days} at {} from {}, {}\n",
            task.name,
            self.handle_of(&task.bot_id),
            task.every_days,
            task.at,
            task.starts_on,
            if task.enabled { "on" } else { "off" },
        );
        text += &format!("  next run: {}\n", task.next_run_at.as_deref().map_or("none".to_string(), minute));
        let mut ran = Vec::new();
        if let Some(at) = &task.last_run_at {
            ran.push(match task.last_run_session.as_deref().map(|id| store.get_session(id)).transpose()?.flatten() {
                Some(session) => {
                    format!("last run: {} (@{}/{})", minute(at), self.handle_of(&session.bot_id), session.number)
                }
                None => "last run: (removed)".to_string(),
            });
        }
        if let Some(done) = store.last_completed_run(&task.id)? {
            ran.push(format!("last completed run: {}", minute(&done)));
        }
        if !ran.is_empty() {
            text += &format!("  {}\n", ran.join(" · "));
        }
        let instant = |stamp: &str| DateTime::parse_from_rfc3339(stamp).ok();
        if let Some(skipped) = &task.skipped_at {
            if task.last_run_at.as_deref().is_none_or(|last| instant(skipped) > instant(last)) {
                text += &format!("  skipped: {}, the last run was still busy\n", minute(skipped));
            }
        }
        text += "  prompt:";
        for line in task.prompt.trim_end().lines() {
            text += &format!("\n    {line}");
        }
        Ok(text)
    }

    pub(super) async fn bridge_tasks(&self, pane_id: &str) -> BridgeResponse {
        if let Err(refusal) = self.master_caller(pane_id, "tasks").await {
            return refusal;
        }
        let tasks = match self.list_tasks() {
            Ok(tasks) => tasks,
            Err(e) => return internal(e),
        };
        if tasks.is_empty() {
            return BridgeResponse::output("No recurring tasks yet.");
        }
        match tasks.iter().map(|t| self.task_block(t)).collect::<Result<Vec<_>>>() {
            Ok(blocks) => BridgeResponse::output(blocks.join("\n")),
            Err(e) => internal(e),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn bridge_task_create(
        &self,
        pane_id: &str,
        name: &str,
        bot: &str,
        every: u32,
        at: Option<&str>,
        from: Option<&str>,
        prompt: &str,
        recreate: bool,
    ) -> BridgeResponse {
        let (caller, flight) = match self.master_caller(pane_id, "task create").await {
            Ok(found) => found,
            Err(refusal) => return refusal,
        };
        if !on_users_turn(flight.as_ref()) {
            return refuse(ErrorCode::NotAllowed, CHANGE_ONLY_ON_USERS_TURN);
        }
        let store = &self.store;
        let (master, session) = match (store.get_bot(&caller.bot_id), store.get_session(&caller.session_id)) {
            (Ok(Some(master)), Ok(Some(session))) => (master, session),
            (Err(e), _) | (_, Err(e)) => return internal(e),
            _ => return refuse(ErrorCode::NotAllowed, "This conversation is gone."),
        };
        let target = match self.bot_named(bot) {
            Ok(target) => target,
            Err(refusal) => return refusal,
        };
        let key = name.trim().to_lowercase();
        let fingerprint = task_fingerprint(&target.handle, every, at.unwrap_or(""), from.unwrap_or(""), prompt);
        let recorded = match store.creation(&session.id, CreationKind::Task, &key) {
            Ok(recorded) => recorded,
            Err(e) => return internal(e),
        };
        if let Some(recorded) = recorded {
            match store.get_task(&recorded.result_id) {
                Ok(Some(made)) if recorded.fingerprint == fingerprint => {
                    return match self.task_block(&made) {
                        Ok(block) => BridgeResponse::output(format!("Already created.\n{block}")),
                        Err(e) => internal(e),
                    }
                }
                Ok(Some(made)) => {
                    return refuse(
                        ErrorCode::CreatedDifferently,
                        format!(
                            "This conversation already created the recurring task {} with other settings. \
                             Change it with `sbot task edit`.",
                            made.name
                        ),
                    )
                }
                Ok(None) if !recreate => {
                    return refuse(
                        ErrorCode::CreatedThenDeleted,
                        format!(
                            "The recurring task {} this conversation created was deleted in the app. Ask the user; \
                             only if they want it again, repeat the command with --recreate.",
                            recorded.result_name
                        ),
                    )
                }
                Ok(None) => {}
                Err(e) => return internal(e),
            }
        }
        let new = NewTask {
            name: name.to_string(),
            bot_id: target.id.clone(),
            prompt: prompt.to_string(),
            every_days: every,
            at: at.unwrap_or(DEFAULT_AT).to_string(),
            starts_on: from.map(str::to_string),
        };
        let row = match self.checked_task(new, None) {
            Ok(row) => row,
            Err(e) => return bad_request(e),
        };
        let record =
            NewCreation { session_id: &session.id, kind: CreationKind::Task, key: &key, fingerprint: &fingerprint };
        match store.insert_task_recorded(&row, &record) {
            Ok(task) => {
                tracing::info!(bot = %master.handle, session = session.number, task = %task.name, "Master created a recurring task");
                self.notice(&session.id, "master_created_task", json!({"name": task.name, "handle": target.handle}));
                self.emit(UiEvent::TasksChanged);
                match self.task_block(&task) {
                    Ok(block) => BridgeResponse::output(block),
                    Err(e) => internal(e),
                }
            }
            Err(e) => refuse(ErrorCode::Internal, format!("Could not create the recurring task: {e}")),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn bridge_task_edit(
        &self,
        pane_id: &str,
        task: &str,
        name: Option<String>,
        bot: Option<String>,
        every: Option<u32>,
        at: Option<String>,
        from: Option<String>,
        enabled: Option<bool>,
        prompt: Option<String>,
    ) -> BridgeResponse {
        let (caller, flight) = match self.master_caller(pane_id, "task edit").await {
            Ok(found) => found,
            Err(refusal) => return refusal,
        };
        if !on_users_turn(flight.as_ref()) {
            return refuse(ErrorCode::NotAllowed, CHANGE_ONLY_ON_USERS_TURN);
        }
        let current = match self.named_task(task) {
            Ok(current) => current,
            Err(refusal) => return refusal,
        };
        let given = [name.is_some(), bot.is_some(), every.is_some(), at.is_some(), from.is_some(), enabled.is_some()];
        if prompt.is_none() && !given.contains(&true) {
            return refuse(
                ErrorCode::BadRequest,
                "Nothing to change: give --name, --bot, --every, --at, --from, --on, --off or --prompt.",
            );
        }
        let bot_id = match bot.as_deref() {
            Some(handle) => match self.bot_named(handle) {
                Ok(target) => target.id,
                Err(refusal) => return refusal,
            },
            None => current.bot_id.clone(),
        };
        let edit = NewTask {
            name: name.unwrap_or_else(|| current.name.clone()),
            bot_id,
            prompt: prompt.unwrap_or_else(|| current.prompt.clone()),
            every_days: every.unwrap_or(current.every_days),
            at: at.unwrap_or_else(|| current.at.clone()),
            starts_on: from,
        };
        let mut changed = match self.update_task(&current.id, edit) {
            Ok(changed) => changed,
            Err(e) => return bad_request(e),
        };
        if enabled.is_some_and(|on| on != changed.enabled) {
            changed = match self.set_task_enabled(&changed.id, !changed.enabled) {
                Ok(changed) => changed,
                Err(e) => return bad_request(e),
            };
        }
        let handle = self.handle_of(&caller.bot_id);
        let number = self.store.get_session(&caller.session_id).ok().flatten().map_or(0, |s| s.number);
        tracing::info!(bot = %handle, session = number, task = %changed.name, "Master changed a recurring task");
        self.notice(&caller.session_id, "master_changed_task", json!({"name": changed.name}));
        match self.task_block(&changed) {
            Ok(block) => BridgeResponse::output(block),
            Err(e) => internal(e),
        }
    }

    pub(super) async fn bridge_task_delete(&self, pane_id: &str, task: &str) -> BridgeResponse {
        let (caller, flight) = match self.master_caller(pane_id, "task delete").await {
            Ok(found) => found,
            Err(refusal) => return refusal,
        };
        if !on_users_turn(flight.as_ref()) {
            return refuse(ErrorCode::NotAllowed, CHANGE_ONLY_ON_USERS_TURN);
        }
        let current = match self.named_task(task) {
            Ok(current) => current,
            Err(refusal) => return refusal,
        };
        match self.delete_task(&current.id) {
            Ok(gone) => {
                let handle = self.handle_of(&caller.bot_id);
                let number = self.store.get_session(&caller.session_id).ok().flatten().map_or(0, |s| s.number);
                tracing::info!(bot = %handle, session = number, task = %gone.name, "Master deleted a recurring task");
                self.notice(&caller.session_id, "master_deleted_task", json!({"name": gone.name}));
                BridgeResponse::output(format!(
                    "Deleted the recurring task {}. Its past runs' sessions stay.",
                    gone.name
                ))
            }
            Err(e) => bad_request(e),
        }
    }
}
