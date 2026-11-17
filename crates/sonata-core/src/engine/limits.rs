use super::tasks::stamp;
use super::*;
use crate::envelope::CONTINUE_LINE;
use crate::limits::{reset_instant, Skip, UsageLimit};
use chrono::{DateTime, Utc};

const RETRY_MINUTES: i64 = 30;

const CONTINUE_GRACE: Duration = Duration::from_secs(2 * 60);

const EARLY_RESUME_SECONDS: i64 = 60;

const WORKING_READ_EVERY: Duration = Duration::from_secs(60);

const MAX_RESET_DAYS: i64 = 8;

const PROVIDERS: [Provider; 5] =
    [Provider::Claude, Provider::Codex, Provider::Grok, Provider::Opencode, Provider::Cursor];

fn resumed(mut turn: InFlight, working: bool, now: tokio::time::Instant) -> InFlight {
    turn.after_limit = true;
    turn.seen_working = working;
    turn.delivered_at = Some(now);
    turn.enter_pressed_at = None;
    turn.stopped = false;
    turn
}

fn hold_key(provider: Provider) -> String {
    format!("usage_limit.{}", provider.as_str())
}

#[derive(Debug, Clone, Copy)]
struct Hold {
    until: DateTime<Utc>,
    known: bool,
}

#[derive(serde::Deserialize)]
struct StoredHold {
    until: String,
    known: bool,
}

impl Hold {
    fn parse(value: &str) -> Option<Hold> {
        let stored: StoredHold = serde_json::from_str(value).ok()?;
        let until = DateTime::parse_from_rfc3339(&stored.until).ok()?.with_timezone(&Utc);
        Some(Hold { until, known: stored.known })
    }

    fn to_json(self) -> String {
        json!({"until": stamp(self.until), "known": self.known}).to_string()
    }
}

impl Engine {
    pub fn usage_limits(&self) -> Result<Vec<UsageLimitView>> {
        let mut limits = Vec::new();
        for provider in PROVIDERS {
            if let Some(hold) = self.hold(provider)? {
                limits.push(UsageLimitView { provider, resets_at: hold.known.then(|| stamp(hold.until)) });
            }
        }
        Ok(limits)
    }

    pub(super) fn held(&self, provider: Provider) -> Result<bool> {
        Ok(self.hold(provider)?.is_some())
    }

    fn hold(&self, provider: Provider) -> Result<Option<Hold>> {
        let Some(value) = self.inner.store.get_setting(&hold_key(provider))? else { return Ok(None) };
        let hold = Hold::parse(&value);
        if hold.is_none() {
            tracing::warn!(provider = provider.as_str(), value = %value, "the usage-limit hold cannot be read; the provider's bots are not held");
        }
        Ok(hold)
    }

    pub(super) async fn turn_end_screen(&self, session_id: &str, message_id: &str, rt: &RuntimeBot) -> Option<String> {
        match self.inner.runtime.snapshot(rt, super::controls::SNAPSHOT_LINES).await {
            Ok(screen) => Some(screen),
            Err(e) => {
                let who = self.who(session_id);
                tracing::warn!(bot = %who.bot, session = who.session, message_id, error = %e, "could not read the session's screen at the end of its turn");
                None
            }
        }
    }

    pub(super) async fn end_turn_on_limit(
        &self,
        session_id: &str,
        matches: impl Fn(&Live) -> bool,
        hit: Option<(Provider, UsageLimit)>,
    ) {
        let (rt, message_id) = {
            let mut live = self.inner.live.lock().await;
            let Some(l) = live.get_mut(session_id) else { return };
            if !matches(l) {
                return;
            }
            let flight = l.in_flight.take().expect("checked above");
            let message_id = flight.message_id.clone();
            l.limited_turn = Some(flight);
            l.status = BotStatus::Limited;
            l.continue_at = None;
            (l.rt.clone(), message_id)
        };
        self.emit_status(session_id).await;
        match hit {
            Some((provider, limit)) => self.hit_limit(session_id, provider, limit).await,
            None => {
                let who = self.who(session_id);
                tracing::info!(bot = %who.bot, session = who.session, message_id = %message_id, "the turn stopped while the session waits for its usage limit; it goes on after the reset");
            }
        }
        if let Some(rt) = &rt {
            self.save_session_id(session_id, &message_id, rt).await;
        }
    }

    pub(super) async fn hit_limit(&self, session_id: &str, provider: Provider, limit: UsageLimit) {
        let now = self.now_utc();
        let who = self.who(session_id);
        let latest = now + chrono::Duration::days(MAX_RESET_DAYS);
        let reset = limit
            .reset
            .and_then(|reset| reset_instant(reset, now, &chrono::Local))
            .filter(|at| *at > now && *at <= latest);
        let new = match reset {
            Some(until) => Hold { until, known: true },
            None => Hold { until: now + chrono::Duration::minutes(RETRY_MINUTES), known: false },
        };
        let current = self.hold(provider).unwrap_or_else(|e| {
            tracing::error!(bot = %who.bot, session = who.session, provider = provider.as_str(), error = %e, "could not read the provider's usage-limit hold");
            None
        });
        let hold = match current {
            Some(current) if current.until > new.until => current,
            _ => {
                if let Err(e) = self.inner.store.set_setting(&hold_key(provider), &new.to_json()) {
                    tracing::error!(bot = %who.bot, session = who.session, provider = provider.as_str(), error = %e, "could not record the provider's usage-limit hold");
                }
                new
            }
        };
        tracing::info!(bot = %who.bot, session = who.session, provider = provider.as_str(), until = %stamp(hold.until), known = hold.known, line = %limit.line, "the turn hit the provider's usage limit; its bots wait");
        let first = {
            let mut live = self.inner.live.lock().await;
            live.get_mut(session_id).is_some_and(|l| {
                l.waited_limit_lines = limit.lines.clone();
                !std::mem::replace(&mut l.usage_limit_noticed, true)
            })
        };
        if first {
            let (code, params) = if hold.known {
                let params = json!({"handle": who.bot, "provider": provider.as_str(), "resets_at": stamp(hold.until)});
                ("usage_limit_wait", params)
            } else {
                let params =
                    json!({"handle": who.bot, "provider": provider.as_str(), "minutes": RETRY_MINUTES.to_string()});
                ("usage_limit_retry", params)
            };
            self.notice(session_id, code, params);
        }
        match self.usage_limits() {
            Ok(limits) => self.emit(UiEvent::UsageLimits { limits }),
            Err(e) => {
                tracing::error!(bot = %who.bot, session = who.session, error = %e, "could not read the usage-limit holds for the UI")
            }
        }
    }

    pub(super) async fn check_usage_limits(&self) {
        for provider in self.lift_due_holds() {
            self.pump_provider(provider);
        }
        self.start_graces().await;
        match self.paused() {
            Ok(false) => self.continue_lifted().await,
            Ok(true) => {}
            Err(e) => tracing::error!(error = %e, "could not read whether the bots are paused"),
        }
        self.read_working_screens().await;
    }

    pub async fn try_usage_limit_now(&self, provider: Provider) -> Result<()> {
        self.inner.store.delete_setting(&hold_key(provider))?;
        tracing::info!(provider = provider.as_str(), "Try now: the provider's usage-limit hold is lifted");
        self.emit(UiEvent::UsageLimits { limits: self.usage_limits()? });
        let now = tokio::time::Instant::now();
        let limited: Vec<String> = {
            let mut live = self.inner.live.lock().await;
            live.iter_mut()
                .filter(|(_, l)| l.status == BotStatus::Limited && l.started.is_some_and(|s| s.provider == provider))
                .map(|(id, l)| {
                    l.continue_at = Some(now);
                    id.clone()
                })
                .collect()
        };
        if self.paused()? {
            return Ok(());
        }
        for session_id in limited {
            self.continue_limited(&session_id).await;
        }
        self.pump_provider(provider);
        Ok(())
    }

    fn lift_due_holds(&self) -> Vec<Provider> {
        let now = self.now_utc();
        let mut lifted = Vec::new();
        for provider in PROVIDERS {
            match self.hold(provider) {
                Ok(Some(hold)) if hold.until <= now => {
                    if let Err(e) = self.inner.store.delete_setting(&hold_key(provider)) {
                        tracing::error!(provider = provider.as_str(), error = %e, "could not lift the provider's usage-limit hold");
                        continue;
                    }
                    tracing::info!(provider = provider.as_str(), until = %stamp(hold.until), "the provider's usage limit reset; its bots go on");
                    lifted.push(provider);
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::error!(provider = provider.as_str(), error = %e, "could not read the provider's usage-limit hold")
                }
            }
        }
        if !lifted.is_empty() {
            match self.usage_limits() {
                Ok(limits) => self.emit(UiEvent::UsageLimits { limits }),
                Err(e) => tracing::error!(error = %e, "could not read the usage-limit holds for the UI"),
            }
        }
        lifted
    }

    async fn start_graces(&self) {
        let waiting: Vec<(String, Provider)> = {
            let live = self.inner.live.lock().await;
            live.iter()
                .filter(|(_, l)| l.waits_for_limit())
                .filter_map(|(id, l)| Some((id.clone(), l.started?.provider)))
                .collect()
        };
        let mut lifted = Vec::new();
        for (session_id, provider) in waiting {
            match self.held(provider) {
                Ok(false) => lifted.push(session_id),
                Ok(true) => {}
                Err(e) => {
                    let who = self.who(&session_id);
                    tracing::error!(bot = %who.bot, session = who.session, provider = provider.as_str(), error = %e, "could not read whether the session's provider is still held");
                }
            }
        }
        if lifted.is_empty() {
            return;
        }
        let at = tokio::time::Instant::now() + CONTINUE_GRACE;
        let mut live = self.inner.live.lock().await;
        for session_id in &lifted {
            if let Some(l) = live.get_mut(session_id).filter(|l| l.waits_for_limit()) {
                l.continue_at = Some(at);
            }
        }
    }

    async fn continue_lifted(&self) {
        let now = tokio::time::Instant::now();
        let due: Vec<String> = {
            let live = self.inner.live.lock().await;
            live.iter()
                .filter(|(_, l)| l.status == BotStatus::Limited && l.continue_at.is_some_and(|at| at <= now))
                .map(|(id, _)| id.clone())
                .collect()
        };
        for session_id in due {
            self.continue_limited(&session_id).await;
        }
    }

    async fn continue_limited(&self, session_id: &str) {
        let (rt, provider) = {
            let live = self.inner.live.lock().await;
            let Some(l) = live.get(session_id).filter(|l| l.status == BotStatus::Limited && l.continue_at.is_some())
            else {
                return;
            };
            let (Some(rt), Some(started)) = (l.rt.clone(), l.started) else { return };
            (rt, started.provider)
        };
        let who = self.who(session_id);
        match self.held(provider) {
            Ok(false) => {}
            Ok(true) => {
                if let Some(l) = self.inner.live.lock().await.get_mut(session_id) {
                    if l.status == BotStatus::Limited {
                        l.continue_at = None;
                    }
                }
                return;
            }
            Err(e) => {
                tracing::error!(bot = %who.bot, session = who.session, provider = provider.as_str(), error = %e, "could not read whether the session's provider is still held");
                return;
            }
        }
        let state = match self.inner.runtime.inspect(&rt).await {
            Ok(inspection) => inspection.state,
            Err(e) => {
                tracing::warn!(bot = %who.bot, session = who.session, error = %e, "could not inspect the session's CLI to continue its work after the usage limit");
                return;
            }
        };
        enum Go {
            Follow(String),
            Prompt(String),
            Blocked(String),
            Idle,
        }
        let working = state == AgentState::Working;
        let blocked = state == AgentState::Blocked;
        let now = tokio::time::Instant::now();
        let go = {
            let mut live = self.inner.live.lock().await;
            let Some(l) = live.get_mut(session_id) else { return };
            let same_cli = l.rt.as_ref().is_some_and(|r| r.pane_id == rt.pane_id);
            if !same_cli || l.status != BotStatus::Limited || l.continue_at.take().is_none() {
                return;
            }
            let go = if let Some(f) = l.in_flight.as_mut() {
                f.after_limit = true;
                if working || f.stopped {
                    Go::Follow(f.message_id.clone())
                } else if blocked {
                    Go::Blocked(f.message_id.clone())
                } else {
                    f.seen_working = false;
                    f.delivered_at = Some(now);
                    f.enter_pressed_at = None;
                    Go::Prompt(f.message_id.clone())
                }
            } else if let Some(turn) = l.limited_turn.take() {
                let id = turn.message_id.clone();
                l.in_flight = Some(resumed(turn, working || blocked, now));
                if working {
                    Go::Follow(id)
                } else if blocked {
                    Go::Blocked(id)
                } else {
                    Go::Prompt(id)
                }
            } else {
                Go::Idle
            };
            match &go {
                Go::Follow(_) => l.status = BotStatus::Working,
                Go::Prompt(_) => {
                    l.status = BotStatus::Working;
                    l.waited_limit_lines.clear();
                    l.prompt = Some(CONTINUE_LINE.to_string());
                }
                Go::Blocked(_) => l.status = BotStatus::NeedsAttention,
                Go::Idle => {
                    l.status = BotStatus::Idle;
                    l.idle_since = Some(now);
                }
            }
            go
        };
        self.emit_status(session_id).await;
        match go {
            Go::Follow(message_id) => {
                tracing::info!(bot = %who.bot, session = who.session, message_id = %message_id, "the usage limit reset and the session's turn goes on without a continue line");
            }
            Go::Prompt(message_id) => {
                tracing::info!(bot = %who.bot, session = who.session, message_id = %message_id, "the usage limit reset; telling the session's CLI to continue");
                match self.prompt_with_retry(session_id, &message_id, &rt, CONTINUE_LINE).await {
                    Ok(()) => {
                        if let Some(grace) = self.inner.cfg.turn_start_grace {
                            tokio::spawn(self.check_stalled_turn_after(grace, session_id, &message_id));
                        }
                    }
                    Err(e) => {
                        tracing::warn!(bot = %who.bot, session = who.session, message_id = %message_id, error = %e, "could not type the continue line")
                    }
                }
            }
            Go::Blocked(message_id) => {
                tracing::info!(bot = %who.bot, session = who.session, message_id = %message_id, "the usage limit reset, but the session's CLI shows a dialog; nothing is typed until the user answers it");
                self.notice(session_id, "bot_blocked", json!({}));
            }
            Go::Idle => {
                tracing::warn!(bot = %who.bot, session = who.session, "the usage limit reset, but the session has no turn to go on");
                tokio::spawn(self.pump_session(session_id));
            }
        }
    }

    pub(super) async fn resume_on_its_own(&self, session_id: &str, rt: &RuntimeBot) {
        let provider = self.inner.live.lock().await.get(session_id).and_then(|l| l.started).map(|s| s.provider);
        let Some(provider) = provider else { return };
        let who = self.who(session_id);
        match self.hold(provider) {
            Ok(Some(hold)) if hold.until - chrono::Duration::seconds(EARLY_RESUME_SECONDS) > self.now_utc() => return,
            Ok(_) => {}
            Err(e) => {
                tracing::error!(bot = %who.bot, session = who.session, provider = provider.as_str(), error = %e, "could not read the provider's usage-limit hold");
                return;
            }
        }
        match self.inner.runtime.inspect(rt).await {
            Ok(inspection) if inspection.state == AgentState::Working => {}
            Ok(_) => return,
            Err(e) => {
                tracing::warn!(bot = %who.bot, session = who.session, error = %e, "could not confirm that the session's CLI went on by itself after the usage limit");
                return;
            }
        }
        let message_id = {
            let mut live = self.inner.live.lock().await;
            let Some(l) = live.get_mut(session_id) else { return };
            let same_cli = l.rt.as_ref().is_some_and(|r| r.pane_id == rt.pane_id);
            if !same_cli || l.status != BotStatus::Limited || l.in_flight.is_some() {
                return;
            }
            let Some(turn) = l.limited_turn.take() else { return };
            let id = turn.message_id.clone();
            l.in_flight = Some(resumed(turn, true, tokio::time::Instant::now()));
            l.status = BotStatus::Working;
            l.continue_at = None;
            id
        };
        self.emit_status(session_id).await;
        tracing::info!(bot = %who.bot, session = who.session, message_id = %message_id, "the usage limit reset and the session's CLI went on by itself; its turn goes on");
    }

    async fn read_working_screens(&self) {
        {
            let now = tokio::time::Instant::now();
            let mut last = self.inner.last_limit_read.lock().unwrap();
            if last.is_some_and(|at| now.duration_since(at) < WORKING_READ_EVERY) {
                return;
            }
            *last = Some(now);
        }
        let working: Vec<_> = {
            let live = self.inner.live.lock().await;
            live.iter()
                .filter(|(_, l)| l.status == BotStatus::Working)
                .filter_map(|(id, l)| {
                    let f =
                        l.in_flight.as_ref().filter(|f| f.seen_working && f.delivered_at.is_some() && !f.stopped)?;
                    Some((
                        id.clone(),
                        l.rt.clone()?,
                        l.started?.provider,
                        f.message_id.clone(),
                        l.prompt.clone(),
                        l.waited_limit_lines.clone(),
                    ))
                })
                .collect()
        };
        for (session_id, rt, provider, message_id, prompt, waited) in working {
            let Some(adapter) = adapter_for(provider) else { continue };
            let screen = match self.inner.runtime.snapshot(&rt, super::controls::SNAPSHOT_LINES).await {
                Ok(screen) => screen,
                Err(e) => {
                    let who = self.who(&session_id);
                    tracing::warn!(bot = %who.bot, session = who.session, message_id = %message_id, error = %e, "could not read the working session's screen for a usage limit");
                    continue;
                }
            };
            let skip = Skip { prompt: prompt.as_deref(), waited: &waited };
            let Some(limit) = adapter.usage_limit(&screen, &skip) else { continue };
            let caught = {
                let mut live = self.inner.live.lock().await;
                match live.get_mut(&session_id) {
                    Some(l)
                        if l.status == BotStatus::Working
                            && l.in_flight.as_ref().is_some_and(|f| f.message_id == message_id && !f.stopped) =>
                    {
                        l.status = BotStatus::Limited;
                        l.continue_at = None;
                        true
                    }
                    _ => false,
                }
            };
            if caught {
                let who = self.who(&session_id);
                tracing::info!(bot = %who.bot, session = who.session, message_id = %message_id, "the working turn shows the usage limit; it waits with the turn in flight");
                self.emit_status(&session_id).await;
                self.hit_limit(&session_id, provider, limit).await;
            }
        }
    }

    fn pump_provider(&self, provider: Provider) {
        let sessions = match self.inner.store.sessions_with_queued() {
            Ok(sessions) => sessions,
            Err(e) => {
                tracing::error!(provider = provider.as_str(), error = %e, "could not read the sessions with queued messages");
                return;
            }
        };
        for session_id in sessions {
            if self.session_with_bot(&session_id).is_some_and(|(bot, _)| bot.provider == provider) {
                tokio::spawn(self.pump_session(&session_id));
            }
        }
    }
}
