mod support;

use std::sync::Arc;
use std::time::Duration;

use bridge_proto::BridgeCommand;
use chrono::{DateTime, Local, NaiveDateTime, NaiveTime, SecondsFormat, TimeZone, Utc};
use serde_json::Value;
use sonata_core::engine::{Engine, UiEvent};
use sonata_core::envelope::CONTINUE_LINE;
use sonata_core::limits::{reset_instant, ResetTime};
use sonata_core::model::*;
use sonata_core::runtime::{AgentState, RuntimeError, RuntimeEvent};
use sonata_core::store::Store;
use support::*;

const CLAUDE_LIMIT: &str = include_str!("fixtures/screens/claude-limit.txt");
const GROK_LIMIT: &str = include_str!("fixtures/screens/grok-limit.txt");
const OPENCODE_LIMIT: &str = include_str!("fixtures/screens/opencode-limit.txt");

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 5, 11, 0, 0).unwrap()
}

fn set_now(engine: &Engine, now: DateTime<Utc>) {
    engine.set_clock(Arc::new(move || now));
}

fn claude_reset(now: DateTime<Utc>) -> DateTime<Utc> {
    let reset = ResetTime { date: None, time: NaiveTime::from_hms_opt(14, 10, 0).unwrap() };
    reset_instant(reset, now, &Local).unwrap()
}

fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
}

fn limit_harness() -> Harness {
    let h = harness_after(|_| {});
    set_now(&h.engine, now());
    h
}

async fn ends_on(h: &Harness, pane: &str, screen: &str) {
    h.rt.state().screen = screen.into();
    finish_turn(h, pane).await;
}

fn hold(h: &Harness, provider: Provider) -> Option<Value> {
    let store = Store::open(&h.paths.db_path()).unwrap();
    let value = store.get_setting(&format!("usage_limit.{}", provider.as_str())).unwrap()?;
    Some(serde_json::from_str(&value).unwrap())
}

async fn session_status(h: &Harness, session_id: &str) -> BotStatus {
    h.engine.find_session_view(session_id).await.unwrap().status
}

fn notices(h: &Harness, session_id: &str) -> Vec<Message> {
    h.engine.session_messages(session_id).unwrap().into_iter().filter(|m| m.kind == MessageKind::System).collect()
}

fn notice_codes(h: &Harness, session_id: &str) -> Vec<String> {
    notices(h, session_id).into_iter().filter_map(|m| m.system_code).collect()
}

fn bot_messages(h: &Harness) -> Vec<Message> {
    h.engine.bot_threads().unwrap().into_iter().flat_map(|t| h.engine.thread_messages(&t.id).unwrap()).collect()
}

async fn limited_researcher(h: &Harness) -> (Bot, Message, String) {
    let (bot, hello) = bot_with_delivered(h, "hello").await;
    ends_on(h, "w1:p1", CLAUDE_LIMIT).await;
    let session = latest_session(h, &bot.id);
    (bot, hello, session)
}

#[tokio::test]
async fn a_turn_on_a_limit_posts_no_fallback_and_holds_its_provider() {
    let h = limit_harness();
    let (bot, hello) = bot_with_delivered(&h, "hello").await;
    let session = latest_session(&h, &bot.id);
    h.rt.state().output = "You've hit your 5-hour limit".into();
    let mut ui = h.engine.subscribe_ui();
    ends_on(&h, "w1:p1", CLAUDE_LIMIT).await;

    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert!(!thread.iter().any(|m| m.kind == MessageKind::RawFallback), "{thread:?}");
    assert_eq!(session_status(&h, &session).await, BotStatus::Limited);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Limited);
    assert_eq!(stored(&h, &hello), MessageStatus::Delivered);

    let limits = h.engine.usage_limits().unwrap();
    let [UsageLimitView { provider: Provider::Claude, resets_at: Some(resets_at) }] = &limits[..] else {
        panic!("one claude hold: {limits:?}")
    };
    assert_eq!(instant(resets_at), claude_reset(now()));
    let events = events(&mut ui);
    assert!(events.contains(&UiEvent::UsageLimits { limits: limits.clone() }), "{events:?}");
    assert!(events.iter().any(|e| matches!(
        e,
        UiEvent::SessionStatus { session_id, status: BotStatus::Limited, .. } if *session_id == session
    )));

    let notice = notices(&h, &session).pop().expect("a notice");
    assert_eq!(notice.system_code.as_deref(), Some("usage_limit_wait"));
    let params = notice.system_params.unwrap();
    assert_eq!((params["handle"].as_str(), params["provider"].as_str()), (Some("researcher"), Some("claude")));
    assert_eq!(instant(params["resets_at"].as_str().unwrap()), claude_reset(now()));

    let hold = hold(&h, Provider::Claude).expect("the hold is kept in the settings");
    assert_eq!(instant(hold["until"].as_str().unwrap()), claude_reset(now()));
    assert_eq!(hold["known"], Value::Bool(true));
}

#[tokio::test]
async fn a_waiting_sessions_card_still_names_its_request() {
    let h = limit_harness();
    let (_, _, session) = limited_researcher(&h).await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Limited);
    let [activity] = &h.engine.session_activity(std::slice::from_ref(&session)).await.unwrap()[..] else {
        panic!("one session")
    };
    assert_eq!(activity.working_on.as_ref().map(|l| l.text.as_str()), Some("hello"));
}

#[tokio::test]
async fn a_turn_that_replied_first_still_waits() {
    let h = limit_harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    assert!(h.engine.handle_bridge(reply("w1:p1", "On it.")).await.ok);
    ends_on(&h, "w1:p1", CLAUDE_LIMIT).await;
    assert_eq!(session_status(&h, &latest_session(&h, &bot.id)).await, BotStatus::Limited);
    assert!(hold(&h, Provider::Claude).is_some());
    assert_eq!(h.engine.usage_limits().unwrap().len(), 1);
}

#[tokio::test]
async fn a_turn_on_a_bots_answer_still_waits() {
    let h = limit_harness();
    let (researcher, writer, _) = team(&h).await;
    assert!(h.engine.handle_bridge(reply("w1:p1", "On it.")).await.ok);
    assert!(h.engine.handle_bridge(send("w1:p1", "@writer", "Draft the intro.")).await.ok);
    settle().await;
    finish_turn(&h, "w1:p1").await;
    assert!(h.engine.handle_bridge(reply("w1:p2", "Here is the intro.")).await.ok);
    finish_turn(&h, "w1:p2").await;
    let answer = bot_messages(&h).into_iter().find(|m| m.sender == writer.id).expect("Writer's answer");
    assert!(answer.in_reply_to.is_some());
    assert_eq!(stored(&h, &answer), MessageStatus::Delivered);
    let before = bot_messages(&h).len();

    ends_on(&h, "w1:p1", CLAUDE_LIMIT).await;
    assert_eq!(session_status(&h, &latest_session(&h, &researcher.id)).await, BotStatus::Limited);
    assert!(hold(&h, Provider::Claude).is_some());
    let after = bot_messages(&h);
    assert_eq!(after.len(), before, "no fallback to Writer: {after:?}");
    assert!(!after.iter().any(|m| m.kind == MessageKind::RawFallback));
}

#[tokio::test]
async fn a_held_provider_delivers_nothing_to_its_bots() {
    let h = limit_harness();
    limited_researcher(&h).await;

    let writer = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let held = h.engine.queue_user_message(&writer.id, "Draft the intro.").unwrap();
    h.engine.pump(&writer.id).await;
    assert_eq!(stored(&h, &held), MessageStatus::Queued);
    assert_eq!(h.rt.state().starts.len(), 1, "no CLI starts for a held provider");

    let coder = h.engine.create_bot(NewBot { provider: Provider::Codex, ..claude_bot("Coder") }).unwrap();
    let free = h.engine.queue_user_message(&coder.id, "Fix the build.").unwrap();
    h.engine.pump(&coder.id).await;
    assert_eq!(stored(&h, &free), MessageStatus::Delivered);
    assert_eq!(h.rt.state().prompts.last().unwrap().0, "w1:p2");
    assert_eq!(stored(&h, &held), MessageStatus::Queued);
}

#[tokio::test(start_paused = true)]
async fn a_limited_session_is_not_stopped_when_idle() {
    let h = limit_harness();
    let (_, _, session) = limited_researcher(&h).await;
    let watchdog = h.engine.run_watchdog(Duration::from_secs(15));
    tokio::time::sleep(Duration::from_secs(31 * 60)).await;
    settle().await;
    watchdog.abort();
    assert!(!h.rt.state().stopped.contains(&"w1:p1".to_string()), "{:?}", h.rt.state().stopped);
    assert_eq!(session_status(&h, &session).await, BotStatus::Limited);
    h.rt.state().state = AgentState::Blocked;
    h.engine.watchdog_tick().await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Limited);
    h.rt.state().inspect_failures.push_back(RuntimeError::Gone);
    h.engine.watchdog_tick().await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Offline);
}

fn local_instant(text: &str) -> DateTime<Utc> {
    let naive = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap();
    Local.from_local_datetime(&naive).unwrap().with_timezone(&Utc)
}

fn local(text: &str) -> String {
    local_instant(text).to_rfc3339_opts(SecondsFormat::Millis, true)
}

async fn tick_at(h: &Harness, text: &str) {
    set_now(&h.engine, local_instant(text));
    h.engine.scheduler_tick().await;
    settle().await;
}

fn task(bot: &Bot, name: &str, at: &str) -> NewTask {
    NewTask {
        name: name.into(),
        bot_id: bot.id.clone(),
        prompt: "Update the wiki.".into(),
        every_days: 1,
        at: at.into(),
        starts_on: None,
    }
}

fn task_now(h: &Harness, id: &str) -> Task {
    h.engine.list_tasks().unwrap().into_iter().find(|t| t.id == id).expect("the task exists")
}

#[tokio::test]
async fn a_due_recurring_run_waits_for_the_hold() {
    let h = limit_harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    set_now(&h.engine, local_instant("2026-11-05 08:00"));
    let wiki = h.engine.create_task(task(&bot, "Wiki", "09:00")).unwrap();
    let notes = h.engine.create_task(task(&bot, "Notes", "10:00")).unwrap();

    tick_at(&h, "2026-11-05 09:00").await;
    let [run] = &h.engine.sessions_of(&bot.id).unwrap()[..] else { panic!("one run") };
    ends_on(&h, &pane(&h, run), CLAUDE_LIMIT).await;
    assert_eq!(session_status(&h, &run.id).await, BotStatus::Limited);
    let store = Store::open(&h.paths.db_path()).unwrap();
    assert_eq!(store.last_completed_run(&wiki.id).unwrap(), None, "a limited run has not completed");

    tick_at(&h, "2026-11-05 10:00").await;
    assert_eq!(h.engine.sessions_of(&bot.id).unwrap().len(), 1, "no run starts");
    let held = task_now(&h, &notes.id);
    assert_eq!((held.skipped_at, held.next_run_at.clone()), (None, Some(local("2026-11-05 10:00"))));

    let by_hand = h.engine.run_task_now(&notes.id).unwrap();
    settle().await;
    assert_eq!(h.engine.session_messages(&by_hand.id).unwrap()[0].status, MessageStatus::Queued);
    tick_at(&h, "2026-11-05 10:01").await;
    let held = task_now(&h, &notes.id);
    assert_eq!((held.skipped_at, held.next_run_at), (None, Some(local("2026-11-05 10:00"))));
    assert_eq!(h.engine.sessions_of(&bot.id).unwrap().len(), 2);
}

#[tokio::test]
async fn a_limit_without_a_time_retries_in_30_minutes() {
    let h = limit_harness();
    let bot = h.engine.create_bot(NewBot { provider: Provider::Grok, ..claude_bot("Scout") }).unwrap();
    h.engine.queue_user_message(&bot.id, "hello").unwrap();
    h.engine.pump(&bot.id).await;
    ends_on(&h, "w1:p1", GROK_LIMIT).await;
    let session = latest_session(&h, &bot.id);

    assert_eq!(session_status(&h, &session).await, BotStatus::Limited);
    let hold = hold(&h, Provider::Grok).expect("a grok hold");
    assert_eq!(instant(hold["until"].as_str().unwrap()), now() + chrono::Duration::minutes(30));
    assert_eq!(hold["known"], Value::Bool(false));
    assert_eq!(h.engine.usage_limits().unwrap(), vec![UsageLimitView { provider: Provider::Grok, resets_at: None }]);

    let notice = notices(&h, &session).pop().expect("a notice");
    assert_eq!(notice.system_code.as_deref(), Some("usage_limit_retry"));
    let params = notice.system_params.unwrap();
    assert_eq!(params["handle"], "scout");
    assert_eq!(params["provider"], "grok");
    assert_eq!(params["minutes"], "30");
}

#[tokio::test]
async fn an_opencode_turn_on_its_go_limit_holds_opencode_until_the_reset() {
    let h = limit_harness();
    let scout =
        NewBot { provider: Provider::Opencode, model: Some("opencode-go/kimi-k3".into()), ..claude_bot("Scout") };
    let bot = h.engine.create_bot(scout).unwrap();
    h.engine.queue_user_message(&bot.id, "hello").unwrap();
    h.engine.pump(&bot.id).await;
    ends_on(&h, "w1:p1", OPENCODE_LIMIT).await;

    assert_eq!(session_status(&h, &latest_session(&h, &bot.id)).await, BotStatus::Limited);
    let hold = hold(&h, Provider::Opencode).expect("an opencode hold");
    assert_eq!(hold["known"], Value::Bool(true));
    let reset = ResetTime { date: None, time: NaiveTime::from_hms_opt(15, 5, 0).unwrap() };
    assert_eq!(instant(hold["until"].as_str().unwrap()), reset_instant(reset, now(), &Local).unwrap());
}

fn claude_screen(line: &str) -> String {
    format!("❯ [Message from user · thread you↔user · language: English]\n  hello\n\n⏺ Reading\n  ⎿  {line}\n\n❯\n")
}

#[tokio::test]
async fn a_reset_more_than_8_days_away_retries_in_30_minutes() {
    let h = limit_harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    ends_on(&h, "w1:p1", &claude_screen("You've hit your weekly limit · resets Dec 5 at 8pm (Europe/Madrid)")).await;
    let session = latest_session(&h, &bot.id);
    assert_eq!(session_status(&h, &session).await, BotStatus::Limited);
    let hold = hold(&h, Provider::Claude).expect("a claude hold");
    assert_eq!(instant(hold["until"].as_str().unwrap()), now() + chrono::Duration::minutes(30));
    assert_eq!(hold["known"], Value::Bool(false));
    assert_eq!(h.engine.usage_limits().unwrap(), vec![UsageLimitView { provider: Provider::Claude, resets_at: None }]);
    assert_eq!(notice_codes(&h, &session), vec!["usage_limit_retry".to_string()]);
}

#[tokio::test]
async fn a_weekly_reset_within_8_days_is_kept() {
    let h = limit_harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    ends_on(&h, "w1:p1", &claude_screen("You've hit your weekly limit · resets Nov 11 at 8pm (Europe/Madrid)")).await;
    let session = latest_session(&h, &bot.id);
    let reset = ResetTime { date: Some((None, 11, 11)), time: NaiveTime::from_hms_opt(20, 0, 0).unwrap() };
    let expected = reset_instant(reset, now(), &Local).unwrap();
    assert!(expected - now() > chrono::Duration::days(6), "about 6 days ahead");
    let hold = hold(&h, Provider::Claude).expect("a claude hold");
    assert_eq!(instant(hold["until"].as_str().unwrap()), expected);
    assert_eq!(hold["known"], Value::Bool(true));
    assert_eq!(notice_codes(&h, &session), vec!["usage_limit_wait".to_string()]);
}

#[tokio::test]
async fn the_hold_survives_a_relaunch() {
    let h = limit_harness();
    limited_researcher(&h).await;
    let writer = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let held = h.engine.queue_user_message(&writer.id, "Draft the intro.").unwrap();
    h.engine.pump(&writer.id).await;
    let limits = h.engine.usage_limits().unwrap();
    assert_eq!(limits.len(), 1);

    let again = relaunch(&h);
    set_now(&again.engine, now());
    again.engine.recover().await.unwrap();
    assert_eq!(again.engine.usage_limits().unwrap(), limits);
    assert_eq!(stored(&again, &held), MessageStatus::Queued);
    assert!(again.rt.state().starts.is_empty(), "nothing starts for the held provider");
}

#[tokio::test]
async fn a_turn_the_user_stopped_ends_as_stopped_on_a_limit_screen() {
    let h = limit_harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    let session = latest_session(&h, &bot.id);
    h.engine.stop_turn(&session).await.unwrap();
    h.rt.state().output = "Interrupted".into();
    ends_on(&h, "w1:p1", CLAUDE_LIMIT).await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Idle);
    assert!(h.engine.usage_limits().unwrap().is_empty());
    assert!(!notice_codes(&h, &session).iter().any(|c| c.starts_with("usage_limit")));
    let thread = h.engine.user_thread(&bot.id).unwrap();
    assert!(thread.iter().any(|m| m.kind == MessageKind::RawFallback && m.body == "Interrupted"));
}

const GRACE: Duration = Duration::from_secs(2 * 60);

fn after_reset() -> DateTime<Utc> {
    claude_reset(now()) + chrono::Duration::minutes(1)
}

async fn watchdog_at(h: &Harness, at: DateTime<Utc>) {
    set_now(&h.engine, at);
    h.engine.watchdog_tick().await;
    settle().await;
}

async fn after_the_grace(h: &Harness) {
    tokio::time::advance(GRACE).await;
    h.engine.watchdog_tick().await;
    settle().await;
}

fn prompts_to(h: &Harness, pane: &str) -> Vec<String> {
    h.rt.state().prompts.iter().filter(|(p, _)| p == pane).map(|(_, text)| text.clone()).collect()
}

fn continued(screen: &str) -> String {
    format!("{screen}\n❯ {CONTINUE_LINE}\n\n⏺ Done.\n")
}

async fn reports(h: &Harness, pane: &str, state: AgentState) {
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: pane.into(), state }).await;
    settle().await;
}

async fn held_writer(h: &Harness) -> (Bot, Message) {
    let writer = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let held = h.engine.queue_user_message(&writer.id, "Draft the intro.").unwrap();
    h.engine.pump(&writer.id).await;
    assert_eq!(stored(h, &held), MessageStatus::Queued);
    (writer, held)
}

fn last_fallback(h: &Harness, bot: &Bot) -> Option<Message> {
    h.engine.user_thread(&bot.id).unwrap().into_iter().rfind(|m| m.kind == MessageKind::RawFallback)
}

#[tokio::test(start_paused = true)]
async fn at_the_reset_the_limited_session_gets_the_continue_line() {
    let h = limit_harness();
    let (_, _, session) = limited_researcher(&h).await;
    let (_, held) = held_writer(&h).await;
    let mut ui = h.engine.subscribe_ui();

    watchdog_at(&h, after_reset()).await;
    assert!(hold(&h, Provider::Claude).is_none(), "the hold is lifted");
    assert!(events(&mut ui).contains(&UiEvent::UsageLimits { limits: vec![] }));
    assert_eq!(stored(&h, &held), MessageStatus::Delivered, "the provider's queues are pumped");
    assert_eq!(prompts_to(&h, "w1:p1").len(), 1, "no continue line during the grace");
    assert_eq!(session_status(&h, &session).await, BotStatus::Limited);

    after_the_grace(&h).await;
    assert_eq!(prompts_to(&h, "w1:p1")[1..], [CONTINUE_LINE.to_string()]);
    assert_eq!(session_status(&h, &session).await, BotStatus::Working);
}

#[tokio::test(start_paused = true)]
async fn a_cli_that_continued_by_itself_gets_no_continue_line() {
    let h = limit_harness();
    let (bot, hello, session) = limited_researcher(&h).await;
    watchdog_at(&h, after_reset()).await;
    h.rt.state().state = AgentState::Working;
    after_the_grace(&h).await;
    assert_eq!(prompts_to(&h, "w1:p1").len(), 1, "no continue line");
    assert_eq!(session_status(&h, &session).await, BotStatus::Working);

    h.rt.state().screen = format!("{CLAUDE_LIMIT}\n⏺ Done.\n");
    h.rt.state().output = "Done.".into();
    h.rt.state().state = AgentState::Idle;
    reports(&h, "w1:p1", AgentState::Idle).await;
    assert!(hold(&h, Provider::Claude).is_none(), "no new hold");
    assert_eq!(session_status(&h, &session).await, BotStatus::Idle);
    let answer = last_fallback(&h, &bot).expect("the work's fallback");
    assert_eq!((answer.body.as_str(), answer.in_reply_to.as_deref()), ("Done.", Some(hello.id.as_str())));
}

#[tokio::test(start_paused = true)]
async fn the_continued_work_answers_the_original_sender() {
    let h = limit_harness();
    let (researcher, writer, _) = team(&h).await;
    assert!(h.engine.handle_bridge(send("w1:p1", "@writer", "Draft the intro.")).await.ok);
    settle().await;
    assert!(h.engine.handle_bridge(reply("w1:p1", "Asked @writer.")).await.ok);
    finish_turn(&h, "w1:p1").await;
    let brief = bot_messages(&h).into_iter().find(|m| m.sender == researcher.id).expect("the brief");
    let writer_session = latest_session(&h, &writer.id);
    ends_on(&h, "w1:p2", CLAUDE_LIMIT).await;
    assert_eq!(session_status(&h, &writer_session).await, BotStatus::Limited);

    watchdog_at(&h, after_reset()).await;
    after_the_grace(&h).await;
    assert_eq!(prompts_to(&h, "w1:p2").last().map(String::as_str), Some(CONTINUE_LINE));
    h.rt.state().screen = continued(CLAUDE_LIMIT);
    reports(&h, "w1:p2", AgentState::Working).await;
    assert_eq!(session_status(&h, &writer_session).await, BotStatus::Working);

    let sent = h.engine.handle_bridge(reply("w1:p2", "Here is the intro.")).await;
    assert!(sent.ok, "{:?}", sent.error);
    let answer = bot_messages(&h).into_iter().find(|m| m.body == "Here is the intro.").expect("Writer's answer");
    assert_eq!(answer.in_reply_to.as_deref(), Some(brief.id.as_str()));
    assert_eq!(answer.recipient_session.as_deref(), Some(latest_session(&h, &researcher.id).as_str()));
}

#[tokio::test(start_paused = true)]
async fn a_continued_recurring_run_completes_when_its_work_ends() {
    let h = limit_harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    set_now(&h.engine, local_instant("2026-11-05 08:00"));
    let wiki = h.engine.create_task(task(&bot, "Wiki", "09:00")).unwrap();
    tick_at(&h, "2026-11-05 09:00").await;
    let [run] = &h.engine.sessions_of(&bot.id).unwrap()[..] else { panic!("one run") };
    let run_pane = pane(&h, run);
    ends_on(&h, &run_pane, CLAUDE_LIMIT).await;
    let store = Store::open(&h.paths.db_path()).unwrap();
    assert_eq!(store.last_completed_run(&wiki.id).unwrap(), None);

    watchdog_at(&h, local_instant("2026-11-05 14:11")).await;
    after_the_grace(&h).await;
    assert_eq!(prompts_to(&h, &run_pane).last().map(String::as_str), Some(CONTINUE_LINE));
    assert_eq!(store.last_completed_run(&wiki.id).unwrap(), None, "not before its work ends");
    h.rt.state().screen = continued(CLAUDE_LIMIT);
    finish_turn(&h, &run_pane).await;
    assert_eq!(store.last_completed_run(&wiki.id).unwrap(), Some(run.created_at.clone()));
}

#[tokio::test(start_paused = true)]
async fn a_slot_that_comes_while_its_last_run_waits_to_continue_is_skipped() {
    let h = limit_harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    set_now(&h.engine, local_instant("2026-11-05 08:00"));
    let wiki = h.engine.create_task(task(&bot, "Wiki", "09:00")).unwrap();
    tick_at(&h, "2026-11-05 09:00").await;
    let [run] = &h.engine.sessions_of(&bot.id).unwrap()[..] else { panic!("one run") };
    let weekly = claude_screen("You've hit your weekly limit · resets Nov 7 at 8pm (Europe/Madrid)");
    ends_on(&h, &pane(&h, run), &weekly).await;
    tick_at(&h, "2026-11-06 09:00").await;
    assert_eq!(h.engine.sessions_of(&bot.id).unwrap().len(), 1, "held");

    watchdog_at(&h, local_instant("2026-11-07 20:01")).await;
    tick_at(&h, "2026-11-07 20:01").await;
    assert_eq!(h.engine.sessions_of(&bot.id).unwrap().len(), 1, "no second run beside it");
    assert!(task_now(&h, &wiki.id).skipped_at.is_some(), "the slot is skipped as busy");
}

#[tokio::test(start_paused = true)]
async fn a_stale_limit_above_the_continue_line_does_not_rearm() {
    let h = limit_harness();
    let (bot, hello, session) = limited_researcher(&h).await;
    watchdog_at(&h, after_reset()).await;
    after_the_grace(&h).await;
    h.rt.state().screen = continued(CLAUDE_LIMIT);
    h.rt.state().output = "Done.".into();
    finish_turn(&h, "w1:p1").await;
    assert!(hold(&h, Provider::Claude).is_none(), "no new hold");
    assert!(h.engine.usage_limits().unwrap().is_empty());
    assert_eq!(session_status(&h, &session).await, BotStatus::Idle);
    let answer = last_fallback(&h, &bot).expect("the work's fallback");
    assert_eq!((answer.body.as_str(), answer.in_reply_to.as_deref()), ("Done.", Some(hello.id.as_str())));
    assert_eq!(notice_codes(&h, &session), vec!["usage_limit_wait".to_string()]);
}

#[tokio::test(start_paused = true)]
async fn a_limit_without_a_time_tries_again_and_posts_once() {
    let h = limit_harness();
    let bot = h.engine.create_bot(NewBot { provider: Provider::Grok, ..claude_bot("Scout") }).unwrap();
    h.engine.queue_user_message(&bot.id, "hello").unwrap();
    h.engine.pump(&bot.id).await;
    ends_on(&h, "w1:p1", GROK_LIMIT).await;
    let session = latest_session(&h, &bot.id);

    let retry = now() + chrono::Duration::minutes(31);
    watchdog_at(&h, retry).await;
    assert!(hold(&h, Provider::Grok).is_none(), "the 30 minutes are over");
    after_the_grace(&h).await;
    assert_eq!(prompts_to(&h, "w1:p1").last().map(String::as_str), Some(CONTINUE_LINE));

    h.rt.state().screen = format!(
        "     ❯ {CONTINUE_LINE}\n\n     You’ve hit the rate limit for your plan. Upgrade your account or try again later.\n"
    );
    finish_turn(&h, "w1:p1").await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Limited);
    let hold = hold(&h, Provider::Grok).expect("the wait is re-armed");
    assert_eq!(instant(hold["until"].as_str().unwrap()), retry + chrono::Duration::minutes(30));
    assert_eq!(notice_codes(&h, &session), vec!["usage_limit_retry".to_string()], "one notice per wait");
}

#[tokio::test(start_paused = true)]
async fn a_working_turn_showing_the_limit_is_caught_within_a_minute() {
    let h = limit_harness();
    let (bot, hello) = bot_with_delivered(&h, "hello").await;
    let session = latest_session(&h, &bot.id);
    reports(&h, "w1:p1", AgentState::Working).await;
    h.rt.state().state = AgentState::Working;
    h.rt.state().screen = CLAUDE_LIMIT.into();
    for _ in 0..5 {
        h.engine.watchdog_tick().await;
        settle().await;
        tokio::time::advance(Duration::from_secs(15)).await;
    }
    assert_eq!(session_status(&h, &session).await, BotStatus::Limited);
    let held = hold(&h, Provider::Claude).expect("a claude hold");
    assert_eq!(instant(held["until"].as_str().unwrap()), claude_reset(now()));
    assert_eq!(notice_codes(&h, &session), vec!["usage_limit_wait".to_string()]);
    assert_eq!(stored(&h, &hello), MessageStatus::Delivered);

    watchdog_at(&h, after_reset()).await;
    after_the_grace(&h).await;
    assert!(!prompts_to(&h, "w1:p1").contains(&CONTINUE_LINE.to_string()));
    assert_eq!(session_status(&h, &session).await, BotStatus::Working);

    h.rt.state().output = "Done.".into();
    h.rt.state().state = AgentState::Idle;
    reports(&h, "w1:p1", AgentState::Idle).await;
    assert!(hold(&h, Provider::Claude).is_none(), "no new hold");
    assert_eq!(session_status(&h, &session).await, BotStatus::Idle);
    let answer = last_fallback(&h, &bot).expect("the turn's fallback");
    assert_eq!((answer.body.as_str(), answer.in_reply_to.as_deref()), ("Done.", Some(hello.id.as_str())));
    assert_eq!(notice_codes(&h, &session), vec!["usage_limit_wait".to_string()]);
}

#[tokio::test(start_paused = true)]
async fn a_turn_that_ends_while_its_session_waits_ends_quietly() {
    let h = limit_harness();
    let (bot, hello) = bot_with_delivered(&h, "hello").await;
    let session = latest_session(&h, &bot.id);
    reports(&h, "w1:p1", AgentState::Working).await;
    h.rt.state().state = AgentState::Working;
    h.rt.state().screen = CLAUDE_LIMIT.into();
    h.rt.state().output = "You've hit your 5-hour limit".into();
    h.engine.watchdog_tick().await;
    settle().await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Limited);
    let before = hold(&h, Provider::Claude).expect("a claude hold");

    h.rt.state().state = AgentState::Idle;
    reports(&h, "w1:p1", AgentState::Idle).await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Limited);
    assert!(last_fallback(&h, &bot).is_none(), "no fallback");
    assert_eq!(hold(&h, Provider::Claude), Some(before));
    assert_eq!(notice_codes(&h, &session), vec!["usage_limit_wait".to_string()]);

    watchdog_at(&h, after_reset()).await;
    after_the_grace(&h).await;
    assert_eq!(prompts_to(&h, "w1:p1")[1..], [CONTINUE_LINE.to_string()]);
    h.rt.state().screen = continued(CLAUDE_LIMIT);
    h.rt.state().output = "Done.".into();
    finish_turn(&h, "w1:p1").await;
    let answer = last_fallback(&h, &bot).expect("the work's fallback");
    assert_eq!((answer.body.as_str(), answer.in_reply_to.as_deref()), ("Done.", Some(hello.id.as_str())));
    assert_eq!(session_status(&h, &session).await, BotStatus::Idle);
}

#[tokio::test(start_paused = true)]
async fn pause_defers_the_resume() {
    let h = limit_harness();
    let (_, _, session) = limited_researcher(&h).await;
    let (_, held) = held_writer(&h).await;
    h.engine.set_paused(true).await.unwrap();
    watchdog_at(&h, after_reset()).await;
    assert!(hold(&h, Provider::Claude).is_none(), "the hold lifts while paused");
    after_the_grace(&h).await;
    assert_eq!(prompts_to(&h, "w1:p1").len(), 1, "no continue line while paused");
    assert_eq!(stored(&h, &held), MessageStatus::Queued, "no delivery while paused");
    assert_eq!(session_status(&h, &session).await, BotStatus::Limited);

    h.engine.set_paused(false).await.unwrap();
    h.engine.watchdog_tick().await;
    settle().await;
    assert_eq!(prompts_to(&h, "w1:p1")[1..], [CONTINUE_LINE.to_string()]);
    assert_eq!(stored(&h, &held), MessageStatus::Delivered);
}

#[tokio::test(start_paused = true)]
async fn try_now_lifts_the_hold_at_once() {
    let h = limit_harness();
    let (_, _, session) = limited_researcher(&h).await;
    let (_, held) = held_writer(&h).await;
    let mut ui = h.engine.subscribe_ui();
    h.engine.try_usage_limit_now(Provider::Claude).await.unwrap();
    settle().await;
    assert!(hold(&h, Provider::Claude).is_none());
    assert!(events(&mut ui).contains(&UiEvent::UsageLimits { limits: vec![] }));
    assert_eq!(prompts_to(&h, "w1:p1")[1..], [CONTINUE_LINE.to_string()], "no grace");
    assert_eq!(session_status(&h, &session).await, BotStatus::Working);
    assert_eq!(stored(&h, &held), MessageStatus::Delivered);
}

#[tokio::test(start_paused = true)]
async fn try_now_while_paused_continues_after_the_resume() {
    let h = limit_harness();
    let (_, _, session) = limited_researcher(&h).await;
    h.engine.set_paused(true).await.unwrap();
    h.engine.try_usage_limit_now(Provider::Claude).await.unwrap();
    settle().await;
    assert!(hold(&h, Provider::Claude).is_none());
    assert_eq!(prompts_to(&h, "w1:p1").len(), 1);
    h.engine.set_paused(false).await.unwrap();
    h.engine.watchdog_tick().await;
    settle().await;
    assert_eq!(prompts_to(&h, "w1:p1")[1..], [CONTINUE_LINE.to_string()], "no grace after the resume");
    assert_eq!(session_status(&h, &session).await, BotStatus::Working);
}

#[tokio::test(start_paused = true)]
async fn a_slept_mac_resumes_on_the_first_tick() {
    let h = limit_harness();
    let (_, _, session) = limited_researcher(&h).await;
    watchdog_at(&h, claude_reset(now()) + chrono::Duration::hours(6)).await;
    assert!(hold(&h, Provider::Claude).is_none());
    after_the_grace(&h).await;
    assert_eq!(prompts_to(&h, "w1:p1")[1..], [CONTINUE_LINE.to_string()]);
    assert_eq!(session_status(&h, &session).await, BotStatus::Working);
}

#[tokio::test(start_paused = true)]
async fn a_limited_session_takes_no_delivery_until_its_work_continues() {
    let h = limit_harness();
    let (_, _, session) = limited_researcher(&h).await;
    let next = h.engine.queue_user_message_to_session(&session, "next", &[]).unwrap();
    watchdog_at(&h, after_reset()).await;
    reports(&h, "w1:p1", AgentState::Idle).await;
    assert_eq!(stored(&h, &next), MessageStatus::Queued, "not during the grace");
    assert_eq!(prompts_to(&h, "w1:p1").len(), 1);

    after_the_grace(&h).await;
    assert_eq!(prompts_to(&h, "w1:p1")[1..], [CONTINUE_LINE.to_string()]);
    assert_eq!(stored(&h, &next), MessageStatus::Queued, "the continued work comes first");

    h.rt.state().screen = continued(CLAUDE_LIMIT);
    finish_turn(&h, "w1:p1").await;
    assert_eq!(stored(&h, &next), MessageStatus::Delivered);
}

#[tokio::test(start_paused = true)]
async fn a_limited_session_stays_limited_whatever_its_cli_reports() {
    let h = limit_harness();
    let (_, _, session) = limited_researcher(&h).await;
    for state in [AgentState::Blocked, AgentState::Idle, AgentState::Working] {
        reports(&h, "w1:p1", state).await;
        assert_eq!(session_status(&h, &session).await, BotStatus::Limited, "after {state:?}");
    }
    assert!(!notice_codes(&h, &session).contains(&"bot_blocked".to_string()));
}

#[tokio::test]
async fn a_relaunch_during_the_wait_interrupts_the_limited_message() {
    let h = limit_harness();
    let (_, hello, _) = limited_researcher(&h).await;
    let limits = h.engine.usage_limits().unwrap();
    let again = relaunch(&h);
    set_now(&again.engine, now());
    again.engine.recover().await.unwrap();
    assert_eq!(stored(&again, &hello), MessageStatus::Interrupted, "Retry offers it again");
    assert_eq!(again.engine.usage_limits().unwrap(), limits, "the hold is still there");
}

#[tokio::test(start_paused = true)]
async fn a_cli_that_goes_on_by_itself_after_the_reset_is_followed_at_once() {
    let h = limit_harness();
    let (bot, hello, session) = limited_researcher(&h).await;
    watchdog_at(&h, after_reset()).await;

    h.rt.state().state = AgentState::Working;
    reports(&h, "w1:p1", AgentState::Working).await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Working);
    let sent = h.engine.handle_bridge(reply("w1:p1", "Here it is.")).await;
    assert!(sent.ok, "{:?}", sent.error);
    let thread = h.engine.user_thread(&bot.id).unwrap();
    let answer = thread.iter().find(|m| m.body == "Here it is.").expect("the reply reached the user");
    assert_eq!(answer.in_reply_to.as_deref(), Some(hello.id.as_str()));

    after_the_grace(&h).await;
    assert_eq!(prompts_to(&h, "w1:p1").len(), 1, "no continue line");
    assert_eq!(session_status(&h, &session).await, BotStatus::Working);
}

#[tokio::test(start_paused = true)]
async fn a_cli_that_goes_on_by_itself_once_the_reset_time_came_is_followed_before_the_hold_lifts() {
    let h = limit_harness();
    let (_, _, session) = limited_researcher(&h).await;
    set_now(&h.engine, after_reset());
    h.rt.state().state = AgentState::Working;
    reports(&h, "w1:p1", AgentState::Working).await;
    assert!(hold(&h, Provider::Claude).is_some(), "no tick lifted it yet");
    assert_eq!(session_status(&h, &session).await, BotStatus::Working);
}

#[tokio::test(start_paused = true)]
async fn a_continued_turn_whose_cli_is_gone_is_interrupted() {
    let h = limit_harness();
    let (_, hello, session) = limited_researcher(&h).await;
    watchdog_at(&h, after_reset()).await;
    after_the_grace(&h).await;
    assert_eq!(prompts_to(&h, "w1:p1")[1..], [CONTINUE_LINE.to_string()]);
    reports(&h, "w1:p1", AgentState::Working).await;
    h.rt.state().inspect_failures.push_back(RuntimeError::Gone);
    h.engine.watchdog_tick().await;
    settle().await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Offline);
    assert_eq!(stored(&h, &hello), MessageStatus::Interrupted, "Retry offers it again");
}

#[tokio::test(start_paused = true)]
async fn a_session_whose_cli_is_gone_while_it_waits_interrupts_its_message() {
    let h = limit_harness();
    let (_, hello, session) = limited_researcher(&h).await;
    h.rt.state().inspect_failures.push_back(RuntimeError::Gone);
    h.engine.watchdog_tick().await;
    settle().await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Offline);
    assert_eq!(stored(&h, &hello), MessageStatus::Interrupted);
}

#[tokio::test]
async fn every_other_cut_off_of_a_waiting_session_ends_its_limited_turn() {
    for cut in ["stop", "restart", "exit", "archive", "delete"] {
        let h = limit_harness();
        let (bot, hello, session) = limited_researcher(&h).await;
        match cut {
            "stop" => h.engine.stop_session(&session).await.unwrap(),
            "restart" => h.engine.restart_session(&session).await.unwrap(),
            "exit" => h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: "w1:p1".into() }).await,
            "archive" => drop(h.engine.archive_session(&session).await.unwrap()),
            _ => drop(h.engine.delete_bot(&bot.id).await.unwrap()),
        }
        settle().await;
        let expected = if cut == "delete" { MessageStatus::Cancelled } else { MessageStatus::Interrupted };
        assert_eq!(stored(&h, &hello), expected, "{cut}");
    }
}

#[tokio::test(start_paused = true)]
async fn masters_resumed_turn_on_the_users_message_may_create_a_bot() {
    let h = limit_harness();
    let checkout = tempfile::tempdir().unwrap();
    h.engine.finish_setup(Some(checkout.path())).unwrap();
    let master = h.engine.master_id().unwrap().expect("Master was created");
    let (session, _) = h.engine.start_session_with_message(&master, "Make an SEO bot.", &[]).unwrap();
    h.engine.pump_session(&session.id).await;
    let master_pane = pane(&h, &session);
    ends_on(&h, &master_pane, CLAUDE_LIMIT).await;
    assert_eq!(session_status(&h, &session.id).await, BotStatus::Limited);

    watchdog_at(&h, after_reset()).await;
    after_the_grace(&h).await;
    assert_eq!(prompts_to(&h, &master_pane).last().map(String::as_str), Some(CONTINUE_LINE));
    let create = BridgeCommand::BotCreate {
        name: "SEO".into(),
        color: "#0ea5e9".into(),
        provider: "claude".into(),
        skills: vec![],
        instructions: "Does SEO.".into(),
        recreate: false,
        model: None,
    };
    let created = h.engine.handle_bridge(request(&master_pane, create)).await;
    assert!(created.ok, "still the user's turn: {:?}", created.error);
}

fn claude_echo(header: &str, body: &str, after: &str) -> String {
    let echo: String = body.lines().map(|line| format!("  {line}\n")).collect();
    format!("❯ {header}\n{echo}\n{after}\n\n❯\n")
}

const FROM_USER: &str = "[Message from user · thread you↔user · language: English]";

#[tokio::test]
async fn a_limit_quoted_in_the_users_message_is_not_a_limit() {
    let h = limit_harness();
    let body = "Why did my terminal say this?\nYou've hit your weekly limit · resets Nov 9 at 8pm (Europe/Madrid)\n⎿  You've hit your 5-hour limit · resets 2:10pm (Europe/Madrid)";
    let (bot, _) = bot_with_delivered(&h, body).await;
    let session = latest_session(&h, &bot.id);
    h.rt.state().output = "It is claude's limit notice.".into();
    ends_on(&h, "w1:p1", &claude_echo(FROM_USER, body, "⏺ It is claude's limit notice.")).await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Idle);
    assert!(h.engine.usage_limits().unwrap().is_empty());
    assert_eq!(last_fallback(&h, &bot).map(|m| m.body), Some("It is claude's limit notice.".to_string()));
}

#[tokio::test]
async fn a_limit_quoted_in_a_message_to_grok_is_not_a_limit() {
    let h = limit_harness();
    let bot = h.engine.create_bot(NewBot { provider: Provider::Grok, ..claude_bot("Scout") }).unwrap();
    h.engine.queue_user_message(&bot.id, "You hit your weekly limit. What does that mean?").unwrap();
    h.engine.pump(&bot.id).await;
    let session = latest_session(&h, &bot.id);
    let screen = format!(
        "     ❯ {FROM_USER}   2:33 PM\n       You hit your weekly limit. What does that mean?\n\n     It means your weekly usage is spent.\n"
    );
    ends_on(&h, "w1:p1", &screen).await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Idle);
    assert!(h.engine.usage_limits().unwrap().is_empty());
}

#[tokio::test]
async fn a_limit_quoted_in_a_bots_brief_is_not_a_limit() {
    let h = limit_harness();
    let (_, writer, _) = team(&h).await;
    let brief =
        "If your screen shows\n⏺ Usage limit reached · continuing automatically at 2:10pm · esc to cancel\nwait for it, then go on.";
    assert!(h.engine.handle_bridge(send("w1:p1", "@writer", brief)).await.ok);
    settle().await;
    let writer_session = latest_session(&h, &writer.id);
    let screen = claude_echo("[Message from @researcher · thread you↔@researcher]", brief, "⏺ Noted.");
    ends_on(&h, "w1:p2", &screen).await;
    assert_eq!(session_status(&h, &writer_session).await, BotStatus::Idle);
    assert!(h.engine.usage_limits().unwrap().is_empty());
}

#[tokio::test]
async fn a_real_limit_below_a_quoted_one_is_still_read() {
    let h = limit_harness();
    let body = "See:\n⎿  You've hit your weekly limit · resets Nov 9 at 8pm (Europe/Madrid)";
    let (bot, _) = bot_with_delivered(&h, body).await;
    let session = latest_session(&h, &bot.id);
    let screen =
        claude_echo(FROM_USER, body, "⏺ Reading\n  ⎿  You've hit your 5-hour limit · resets 2:10pm (Europe/Madrid)");
    ends_on(&h, "w1:p1", &screen).await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Limited);
    let hold = hold(&h, Provider::Claude).expect("a claude hold");
    assert_eq!(instant(hold["until"].as_str().unwrap()), claude_reset(now()));
}

fn weekly_reset(now: DateTime<Utc>) -> DateTime<Utc> {
    let reset = ResetTime { date: Some((None, 11, 9)), time: NaiveTime::from_hms_opt(20, 0, 0).unwrap() };
    reset_instant(reset, now, &Local).unwrap()
}

fn weekly_below_the_stale_limit() -> String {
    format!(
        "{CLAUDE_LIMIT}\n⏺ Done.\n\n⏺ Checking the build\n  ⎿  You've hit your weekly limit · resets Nov 9 at 8pm (Europe/Madrid)\n"
    )
}

#[tokio::test(start_paused = true)]
async fn a_new_limit_below_the_one_the_session_waited_on_starts_a_new_wait() {
    let h = limit_harness();
    let (_, _, session) = limited_researcher(&h).await;
    watchdog_at(&h, after_reset()).await;
    h.rt.state().state = AgentState::Working;
    reports(&h, "w1:p1", AgentState::Working).await;
    h.rt.state().screen = format!("{CLAUDE_LIMIT}\n⏺ Done.\n");
    h.rt.state().output = "Done.".into();
    h.rt.state().state = AgentState::Idle;
    reports(&h, "w1:p1", AgentState::Idle).await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Idle);
    assert!(h.engine.usage_limits().unwrap().is_empty());

    h.rt.state().state = AgentState::Working;
    reports(&h, "w1:p1", AgentState::Working).await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Working);
    h.rt.state().screen = weekly_below_the_stale_limit();
    h.rt.state().state = AgentState::Idle;
    reports(&h, "w1:p1", AgentState::Idle).await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Limited);
    let hold = hold(&h, Provider::Claude).expect("a new wait");
    assert_eq!(instant(hold["until"].as_str().unwrap()), weekly_reset(after_reset()));
    assert_eq!(notice_codes(&h, &session), vec!["usage_limit_wait".to_string(), "usage_limit_wait".to_string()]);
}

#[tokio::test(start_paused = true)]
async fn the_working_read_finds_a_new_limit_below_the_one_the_session_waited_on() {
    let h = limit_harness();
    let (_, _, session) = limited_researcher(&h).await;
    watchdog_at(&h, after_reset()).await;
    h.rt.state().state = AgentState::Working;
    reports(&h, "w1:p1", AgentState::Working).await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Working);

    h.rt.state().screen = weekly_below_the_stale_limit();
    tokio::time::advance(Duration::from_secs(60)).await;
    h.engine.watchdog_tick().await;
    settle().await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Limited);
    let hold = hold(&h, Provider::Claude).expect("a new wait");
    assert_eq!(instant(hold["until"].as_str().unwrap()), weekly_reset(after_reset()));
}

#[tokio::test(start_paused = true)]
async fn a_cli_that_goes_on_by_itself_within_a_minute_of_the_printed_reset_is_followed() {
    let h = limit_harness();
    let (bot, hello, session) = limited_researcher(&h).await;
    h.rt.state().state = AgentState::Working;
    set_now(&h.engine, claude_reset(now()) - chrono::Duration::seconds(90));
    reports(&h, "w1:p1", AgentState::Working).await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Limited);

    set_now(&h.engine, claude_reset(now()) - chrono::Duration::seconds(30));
    reports(&h, "w1:p1", AgentState::Working).await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Working);
    let sent = h.engine.handle_bridge(reply("w1:p1", "Here it is.")).await;
    assert!(sent.ok, "{:?}", sent.error);
    let thread = h.engine.user_thread(&bot.id).unwrap();
    let answer = thread.iter().find(|m| m.body == "Here it is.").expect("the reply reached the user");
    assert_eq!(answer.in_reply_to.as_deref(), Some(hello.id.as_str()));
}

#[tokio::test(start_paused = true)]
async fn the_continue_types_nothing_into_a_cli_that_shows_a_dialog() {
    for caught_working in [false, true] {
        let h = limit_harness();
        let (bot, hello) = bot_with_delivered(&h, "hello").await;
        let session = latest_session(&h, &bot.id);
        if caught_working {
            reports(&h, "w1:p1", AgentState::Working).await;
            h.rt.state().state = AgentState::Working;
            h.rt.state().screen = CLAUDE_LIMIT.into();
            h.engine.watchdog_tick().await;
            settle().await;
        } else {
            ends_on(&h, "w1:p1", CLAUDE_LIMIT).await;
        }
        assert_eq!(session_status(&h, &session).await, BotStatus::Limited, "caught working: {caught_working}");

        watchdog_at(&h, after_reset()).await;
        h.rt.state().state = AgentState::Blocked;
        after_the_grace(&h).await;
        assert_eq!(prompts_to(&h, "w1:p1").len(), 1, "no continue line into the dialog ({caught_working})");
        assert_eq!(session_status(&h, &session).await, BotStatus::NeedsAttention, "caught working: {caught_working}");
        assert!(notice_codes(&h, &session).contains(&"bot_blocked".to_string()));

        h.rt.state().state = AgentState::Working;
        reports(&h, "w1:p1", AgentState::Working).await;
        assert_eq!(session_status(&h, &session).await, BotStatus::Working, "caught working: {caught_working}");
        let sent = h.engine.handle_bridge(reply("w1:p1", "Here it is.")).await;
        assert!(sent.ok, "{:?}", sent.error);
        let thread = h.engine.user_thread(&bot.id).unwrap();
        let answer = thread.iter().find(|m| m.body == "Here it is.").expect("the reply reached the user");
        assert_eq!(answer.in_reply_to.as_deref(), Some(hello.id.as_str()));
    }
}

#[tokio::test(start_paused = true)]
async fn a_turn_end_read_during_the_continue_leaves_the_continued_turn_alone() {
    let h = limit_harness();
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    let session = latest_session(&h, &bot.id);
    reports(&h, "w1:p1", AgentState::Working).await;
    h.rt.state().state = AgentState::Working;
    h.rt.state().screen = CLAUDE_LIMIT.into();
    h.engine.watchdog_tick().await;
    settle().await;
    assert_eq!(session_status(&h, &session).await, BotStatus::Limited);
    let next = h.engine.queue_user_message_to_session(&session, "next", &[]).unwrap();
    watchdog_at(&h, after_reset()).await;
    tokio::time::advance(GRACE).await;

    h.rt.state().state = AgentState::Idle;
    h.rt.state().output = "Waiting for the limit.".into();
    h.rt.state().snapshot_delay = Some(Duration::from_secs(1));
    let engine = h.engine.clone();
    let idle = tokio::spawn(async move {
        engine.on_runtime_event(RuntimeEvent::State { pane_id: "w1:p1".into(), state: AgentState::Idle }).await
    });
    settle().await;
    h.engine.watchdog_tick().await;
    settle().await;
    assert_eq!(prompts_to(&h, "w1:p1")[1..], [CONTINUE_LINE.to_string()]);
    tokio::time::advance(Duration::from_secs(1)).await;
    idle.await.unwrap();
    settle().await;

    assert_eq!(session_status(&h, &session).await, BotStatus::Working, "the continued turn goes on");
    assert_eq!(stored(&h, &next), MessageStatus::Queued, "nothing is typed on top of the continue line");
    assert_eq!(prompts_to(&h, "w1:p1").len(), 2);
    assert!(last_fallback(&h, &bot).is_none(), "the continued turn did not end");
}
