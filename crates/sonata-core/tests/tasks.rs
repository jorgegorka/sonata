mod support;

use std::time::Duration;

use bridge_proto::{BridgeCommand, BridgeResponse, ErrorCode};
use chrono::{DateTime, Local, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use sonata_core::engine::{Engine, UiEvent};
use sonata_core::envelope::SCHEDULED_NOTE;
use sonata_core::model::*;
use sonata_core::refusal::Refusal;
use sonata_core::schedule::local_minute;
use sonata_core::store::Store;
use sonata_core::CoreError;
use support::*;

fn local_instant(text: &str) -> DateTime<Utc> {
    let naive = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap();
    Local.from_local_datetime(&naive).unwrap().with_timezone(&Utc)
}

fn local(text: &str) -> String {
    local_instant(text).to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn set_now(engine: &Engine, now: DateTime<Utc>) {
    engine.set_clock(std::sync::Arc::new(move || now));
}

fn at(engine: &Engine, text: &str) {
    set_now(engine, local_instant(text));
}

fn minute(stored: &str) -> String {
    local_minute(DateTime::parse_from_rfc3339(stored).unwrap().with_timezone(&Utc), &Local)
}

fn new_task(bot: &Bot, name: &str) -> NewTask {
    NewTask {
        name: name.into(),
        bot_id: bot.id.clone(),
        prompt: "Update the wiki.".into(),
        every_days: 7,
        at: "09:00".into(),
        starts_on: None,
    }
}

fn refusal(e: CoreError) -> Refusal {
    match e {
        CoreError::Refused(r) => r,
        other => panic!("expected a refusal, got {other}"),
    }
}

#[test]
fn a_new_task_is_due_at_its_first_slot() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    at(&h.engine, "2027-06-07 08:00");
    let task = h.engine.create_task(new_task(&bot, "Wiki")).unwrap();
    assert_eq!(task.starts_on, "2027-06-07");
    assert!(task.enabled);
    assert_eq!(task.next_run_at, Some(local("2027-06-07 09:00")));

    at(&h.engine, "2027-06-07 10:00");
    let task = h.engine.create_task(new_task(&bot, "Later")).unwrap();
    assert_eq!(task.starts_on, "2027-06-08");

    let mut future = new_task(&bot, "Future");
    future.starts_on = Some("2027-06-14".into());
    let task = h.engine.create_task(future).unwrap();
    assert_eq!(task.next_run_at, Some(local("2027-06-14 09:00")));

    let mut past = new_task(&bot, "Past");
    past.starts_on = Some("2027-05-31".into());
    let task = h.engine.create_task(past).unwrap();
    assert_eq!(task.next_run_at, Some(local("2027-06-14 09:00")));
}

#[test]
fn task_input_is_checked() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    at(&h.engine, "2027-06-07 08:00");
    let code = |edit: &dyn Fn(&mut NewTask)| {
        let mut new = new_task(&bot, "Wiki");
        edit(&mut new);
        refusal(h.engine.create_task(new).unwrap_err()).code()
    };
    assert_eq!(code(&|t| t.name = "  ".into()), "taskNameRequired");
    assert_eq!(code(&|t| t.name = "x".repeat(41)), "taskNameTooLong");
    assert_eq!(code(&|t| t.prompt = " \n".into()), "taskPromptRequired");
    assert_eq!(code(&|t| t.every_days = 0), "taskEveryRange");
    assert_eq!(code(&|t| t.every_days = 366), "taskEveryRange");
    assert_eq!(code(&|t| t.at = "25:00".into()), "taskTimeInvalid");
    assert_eq!(code(&|t| t.starts_on = Some("2027-13-01".into())), "taskStartInvalid");
    assert_eq!(code(&|t| t.bot_id = "01JNOBOT".into()), "botNotFound");

    let task = h.engine.create_task(new_task(&bot, "  Wiki  ")).unwrap();
    assert_eq!(task.name, "Wiki");
    let mut forty = new_task(&bot, &"y".repeat(40));
    forty.at = "9:30".into();
    assert_eq!(h.engine.create_task(forty).unwrap().at, "09:30");
}

#[test]
fn task_names_ignore_case() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    at(&h.engine, "2027-06-07 08:00");
    let first = h.engine.create_task(new_task(&bot, "Wiki update")).unwrap();
    let e = h.engine.create_task(new_task(&bot, "wiki update")).unwrap_err();
    assert_eq!(refusal(e), Refusal::TaskNameTaken("wiki update".into()));

    let other = h.engine.create_task(new_task(&bot, "Other")).unwrap();
    let e = h.engine.update_task(&other.id, new_task(&bot, "WIKI UPDATE")).unwrap_err();
    assert_eq!(refusal(e), Refusal::TaskNameTaken("WIKI UPDATE".into()));
    h.engine.update_task(&first.id, new_task(&bot, "WIKI UPDATE")).unwrap();

    assert_eq!(h.engine.list_tasks().unwrap().len(), 2);
}

#[test]
fn an_edit_keeps_the_weekday_unless_a_start_is_given() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    at(&h.engine, "2027-06-07 08:00");
    let task = h.engine.create_task(new_task(&bot, "Wiki")).unwrap();
    assert_eq!(task.next_run_at, Some(local("2027-06-07 09:00")));

    at(&h.engine, "2027-06-09 12:00");
    let mut edit = new_task(&bot, "Wiki");
    edit.at = "10:00".into();
    let edited = h.engine.update_task(&task.id, edit.clone()).unwrap();
    assert_eq!(edited.next_run_at, Some(local("2027-06-14 10:00")));

    edit.starts_on = Some("2027-06-10".into());
    let edited = h.engine.update_task(&task.id, edit).unwrap();
    assert_eq!(edited.next_run_at, Some(local("2027-06-10 10:00")));
}

#[test]
fn turning_a_task_off_and_on_makes_up_nothing() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    at(&h.engine, "2027-06-07 08:00");
    let task = h.engine.create_task(new_task(&bot, "Wiki")).unwrap();

    let off = h.engine.set_task_enabled(&task.id, false).unwrap();
    assert!(!off.enabled);
    assert_eq!(off.next_run_at, None);

    at(&h.engine, "2027-06-28 12:00");
    let on = h.engine.set_task_enabled(&task.id, true).unwrap();
    assert!(on.enabled);
    assert_eq!(on.next_run_at, Some(local("2027-07-05 09:00")));

    h.engine.set_task_enabled(&task.id, false).unwrap();
    let edited = h.engine.update_task(&task.id, new_task(&bot, "Wiki")).unwrap();
    assert!(!edited.enabled);
    assert_eq!(edited.next_run_at, None);
}

#[test]
fn every_change_tells_the_ui() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    at(&h.engine, "2027-06-07 08:00");
    let mut ui = h.engine.subscribe_ui();
    let mut changed = || {
        let mut n = 0;
        while let Ok(event) = ui.try_recv() {
            if matches!(event, UiEvent::TasksChanged) {
                n += 1;
            }
        }
        n
    };
    let task = h.engine.create_task(new_task(&bot, "Wiki")).unwrap();
    assert_eq!(changed(), 1);
    h.engine.update_task(&task.id, new_task(&bot, "Wiki 2")).unwrap();
    assert_eq!(changed(), 1);
    h.engine.set_task_enabled(&task.id, false).unwrap();
    assert_eq!(changed(), 1);
    h.engine.delete_task(&task.id).unwrap();
    assert_eq!(changed(), 1);
    let e = h.engine.delete_task(&task.id).unwrap_err();
    assert_eq!(refusal(e), Refusal::TaskNotFound(task.id.clone()));
    assert_eq!(h.engine.tasks_of_bot(&bot.id).unwrap().len(), 0);
}

fn wiki(h: &Harness, bot: &Bot) -> Task {
    at(&h.engine, "2027-06-07 08:00");
    h.engine.create_task(new_task(bot, "Wiki update")).unwrap()
}

async fn tick_at(h: &Harness, text: &str) {
    at(&h.engine, text);
    h.engine.scheduler_tick().await;
    settle().await;
}

fn task_now(h: &Harness, id: &str) -> Task {
    h.engine.list_tasks().unwrap().into_iter().find(|t| t.id == id).expect("the task exists")
}

fn runs(h: &Harness, bot: &Bot) -> Vec<Session> {
    let mut sessions = h.engine.sessions_of(&bot.id).unwrap();
    sessions.sort_by_key(|s| s.number);
    sessions
}

fn first_message(h: &Harness, run: &Session) -> Message {
    h.engine.session_messages(&run.id).unwrap().remove(0)
}

fn prompt_of(h: &Harness, run: &Session) -> String {
    let pane = pane(h, run);
    h.rt.state().prompts.iter().rev().find(|(p, _)| *p == pane).map(|(_, text)| text.clone()).expect("a prompt")
}

async fn complete(h: &Harness, run: &Session) {
    let pane = pane(h, run);
    assert!(h.engine.handle_bridge(reply(&pane, "Done.")).await.ok);
    finish_turn(h, &pane).await;
}

async fn the_second_slot_is_skipped(h: &Harness, bot: &Bot, task: &Task, run: &Session) {
    tick_at(h, "2027-06-14 09:00").await;
    assert_eq!(runs(h, bot).len(), 1, "no new session");
    let task = task_now(h, &task.id);
    assert_eq!(task.skipped_at, Some(local("2027-06-14 09:00")));
    assert_eq!(task.next_run_at, Some(local("2027-06-21 09:00")));
    assert_eq!(task.last_run_session.as_deref(), Some(run.id.as_str()));
}

#[tokio::test]
async fn a_due_task_runs_once_in_a_new_session() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let task = wiki(&h, &bot);
    let mut ui = h.engine.subscribe_ui();

    tick_at(&h, "2027-06-07 08:59").await;
    assert!(runs(&h, &bot).is_empty(), "not due yet");
    assert!(events(&mut ui).is_empty(), "a tick with nothing due tells the UI nothing");

    tick_at(&h, "2027-06-07 09:00").await;
    let [run] = &runs(&h, &bot)[..] else { panic!("one run") };
    assert_eq!(run.title.as_deref(), Some("Wiki update · 2027-06-07"));
    assert_eq!(run.opened_by, None);
    let first = first_message(&h, run);
    assert_eq!((first.sender.as_str(), first.body.as_str()), (USER, "Update the wiki."));
    assert_eq!(first.status, MessageStatus::Delivered);
    assert_eq!(first.task_id.as_deref(), Some(task.id.as_str()));
    assert_eq!(first.task_name.as_deref(), Some("Wiki update"));
    assert_eq!(first.task_since, None);
    assert_eq!(
        prompt_of(&h, run),
        format!(
            "[Recurring task \"Wiki update\" from the user · every 7 days · first run · thread you↔user · \
             language: English]\nUpdate the wiki.\n\n{SCHEDULED_NOTE}"
        )
    );
    let task = task_now(&h, &task.id);
    assert_eq!(task.next_run_at, Some(local("2027-06-14 09:00")));
    assert_eq!(task.last_run_session.as_deref(), Some(run.id.as_str()));
    assert_eq!(task.skipped_at, None);
    let events = events(&mut ui);
    assert!(events.iter().any(|e| matches!(e, UiEvent::SessionAdded { session } if session.session.id == run.id)));
    assert!(events.contains(&UiEvent::TasksChanged));

    set_now(&h.engine, local_instant("2027-06-07 09:00") + chrono::Duration::seconds(30));
    h.engine.scheduler_tick().await;
    settle().await;
    assert_eq!(runs(&h, &bot).len(), 1, "a slot makes one run");
}

#[tokio::test]
async fn missed_runs_collapse_into_one() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let task = wiki(&h, &bot);

    tick_at(&h, "2027-06-29 10:00").await;
    let [run] = &runs(&h, &bot)[..] else { panic!("one run") };
    assert_eq!(run.title.as_deref(), Some("Wiki update · 2027-06-29"));
    assert_eq!(task_now(&h, &task.id).next_run_at, Some(local("2027-07-05 09:00")), "still on Mondays");

    tick_at(&h, "2027-06-29 10:01").await;
    assert_eq!(runs(&h, &bot).len(), 1);
}

#[tokio::test]
async fn a_busy_last_run_skips_the_slot_while_its_message_waits_in_the_queue() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let task = wiki(&h, &bot);
    h.engine.set_paused(true).await.unwrap();

    tick_at(&h, "2027-06-07 09:00").await;
    let [run] = &runs(&h, &bot)[..] else { panic!("the run is made while the bots are paused") };
    assert_eq!(first_message(&h, run).status, MessageStatus::Queued);
    assert!(h.rt.state().prompts.is_empty());

    the_second_slot_is_skipped(&h, &bot, &task, run).await;
}

#[tokio::test]
async fn a_busy_last_run_skips_the_slot_while_its_turn_goes_on() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let task = wiki(&h, &bot);

    tick_at(&h, "2027-06-07 09:00").await;
    let [run] = &runs(&h, &bot)[..] else { panic!("one run") };
    assert_eq!(first_message(&h, run).status, MessageStatus::Delivered);

    the_second_slot_is_skipped(&h, &bot, &task, run).await;
}

#[tokio::test]
async fn a_busy_last_run_skips_the_slot_while_it_waits_for_the_users_answer() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let task = wiki(&h, &bot);

    tick_at(&h, "2027-06-07 09:00").await;
    let [run] = &runs(&h, &bot)[..] else { panic!("one run") };
    let pane = pane(&h, run);
    assert!(h.engine.handle_bridge(ask(&pane, "Which pages?")).await.ok);
    finish_turn(&h, &pane).await;
    let waiting = h.engine.find_session_view(&run.id).await.unwrap().session;
    assert!(waiting.waiting_question_id.is_some());

    the_second_slot_is_skipped(&h, &bot, &task, run).await;
}

#[tokio::test]
async fn a_skip_after_missed_slots_records_the_latest_one() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    at(&h.engine, "2027-06-07 08:00");
    let task = h.engine.create_task(NewTask { every_days: 1, ..new_task(&bot, "Daily") }).unwrap();
    tick_at(&h, "2027-06-07 09:00").await;
    let [run] = &runs(&h, &bot)[..] else { panic!("one run") };
    let pane = pane(&h, run);
    assert!(h.engine.handle_bridge(ask(&pane, "Which pages?")).await.ok);
    finish_turn(&h, &pane).await;
    assert_eq!(task_now(&h, &task.id).next_run_at, Some(local("2027-06-08 09:00")));

    tick_at(&h, "2027-06-10 10:00").await;
    assert_eq!(runs(&h, &bot).len(), 1, "no new session");
    let after = task_now(&h, &task.id);
    assert_eq!(after.skipped_at, Some(local("2027-06-10 09:00")), "the latest slot, not the first one missed");
    assert_eq!(after.next_run_at, Some(local("2027-06-11 09:00")));
}

#[tokio::test]
async fn a_removed_last_run_does_not_hold_the_next_one() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let task = wiki(&h, &bot);

    tick_at(&h, "2027-06-07 09:00").await;
    let first = runs(&h, &bot).remove(0);
    h.engine.remove_session(&first.id).await.unwrap();
    assert_eq!(task_now(&h, &task.id).last_run_session, None);

    tick_at(&h, "2027-06-14 09:00").await;
    let [second] = &runs(&h, &bot)[..] else { panic!("the next slot runs") };
    let after = task_now(&h, &task.id);
    assert_eq!(after.last_run_session.as_deref(), Some(second.id.as_str()));
    assert_eq!(after.skipped_at, None);

    h.engine.archive_session(&second.id).await.unwrap();
    tick_at(&h, "2027-06-21 09:00").await;
    let all = runs(&h, &bot);
    assert_eq!(all.len(), 2, "{all:?}");
    let after = task_now(&h, &task.id);
    assert_eq!(after.last_run_session.as_deref(), Some(all[1].id.as_str()));
    assert_ne!(all[1].id, second.id);
    assert_eq!(after.skipped_at, None);
}

#[tokio::test]
async fn an_interrupted_run_neither_holds_nor_counts() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let task = wiki(&h, &bot);
    tick_at(&h, "2027-06-07 09:00").await;
    let first = runs(&h, &bot).remove(0);
    complete(&h, &first).await;
    let since = format!("· every 7 days · last run {} ·", minute(&first.created_at));

    tick_at(&h, "2027-06-14 09:00").await;
    let second = runs(&h, &bot).remove(1);
    assert_eq!(first_message(&h, &second).status, MessageStatus::Delivered);
    assert!(prompt_of(&h, &second).contains(&since), "{}", prompt_of(&h, &second));

    let again = relaunch(&h);
    at(&again.engine, "2027-06-21 09:00");
    again.engine.recover().await.unwrap();
    assert_eq!(first_message(&again, &second).status, MessageStatus::Interrupted);

    again.engine.scheduler_tick().await;
    settle().await;
    let all = runs(&again, &bot);
    assert_eq!(all.len(), 3, "the slot is not skipped");
    let after = task_now(&again, &task.id);
    assert_eq!(after.skipped_at, None);
    assert_eq!(after.last_run_session.as_deref(), Some(all[2].id.as_str()));
    let prompt = prompt_of(&again, &all[2]);
    assert!(prompt.contains(&since), "the interrupted run does not count: {prompt}");
}

#[tokio::test]
async fn a_stopped_run_does_not_count() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let task = wiki(&h, &bot);
    tick_at(&h, "2027-06-07 09:00").await;
    let first = runs(&h, &bot).remove(0);
    let pane = pane(&h, &first);

    h.engine.stop_turn(&first.id).await.unwrap();
    finish_turn(&h, &pane).await;
    let store = Store::open(&h.paths.db_path()).unwrap();
    assert_eq!(store.last_completed_run(&task.id).unwrap(), None);

    tick_at(&h, "2027-06-14 09:00").await;
    let second = runs(&h, &bot).remove(1);
    let prompt = prompt_of(&h, &second);
    assert!(prompt.contains("· every 7 days · first run ·"), "{prompt}");
}

#[tokio::test]
async fn a_retried_run_counts_once_it_completes() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let task = wiki(&h, &bot);
    tick_at(&h, "2027-06-07 09:00").await;
    let first = runs(&h, &bot).remove(0);
    let cut = first_message(&h, &first);

    let again = relaunch(&h);
    at(&again.engine, "2027-06-07 09:30");
    again.engine.recover().await.unwrap();
    let copy = again.engine.retry_message(&cut.id).unwrap();
    assert_eq!(copy.task_id.as_deref(), Some(task.id.as_str()));
    again.engine.pump_session(&first.id).await;
    let store = Store::open(&h.paths.db_path()).unwrap();
    assert_eq!(store.last_completed_run(&task.id).unwrap(), None, "not completed yet");

    complete(&again, &first).await;
    assert_eq!(store.last_completed_run(&task.id).unwrap(), Some(first.created_at.clone()));

    tick_at(&again, "2027-06-14 09:00").await;
    let second = runs(&again, &bot).remove(1);
    let prompt = prompt_of(&again, &second);
    assert!(prompt.contains(&format!("· every 7 days · last run {} ·", minute(&first.created_at))), "{prompt}");
}

#[tokio::test]
async fn run_now_leaves_the_schedule() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let task = wiki(&h, &bot);
    let mut ui = h.engine.subscribe_ui();

    let run = h.engine.run_task_now(&task.id).unwrap();
    settle().await;
    assert_eq!(run.title.as_deref(), Some("Wiki update · 2027-06-07"));
    assert_eq!(first_message(&h, &run).status, MessageStatus::Delivered);
    let after = task_now(&h, &task.id);
    assert_eq!(after.next_run_at, Some(local("2027-06-07 09:00")));
    assert_eq!(after.last_run_session.as_deref(), Some(run.id.as_str()));
    assert!(events(&mut ui).contains(&UiEvent::TasksChanged));

    let next = h.engine.run_task_now(&task.id).unwrap();
    settle().await;
    assert_ne!(next.id, run.id);
    assert_eq!(runs(&h, &bot).len(), 2);
    assert_eq!(first_message(&h, &next).status, MessageStatus::Delivered);
    assert_eq!(task_now(&h, &task.id).next_run_at, Some(local("2027-06-07 09:00")));

    let e = h.engine.run_task_now("01JNOTASK").unwrap_err();
    assert_eq!(refusal(e), Refusal::TaskNotFound("01JNOTASK".into()));
}

async fn create_bot(h: &Harness, pane: &str) -> BridgeResponse {
    let command = BridgeCommand::BotCreate {
        name: "SEO".into(),
        color: "#0ea5e9".into(),
        provider: "claude".into(),
        skills: vec![],
        instructions: "Manages SEO.".into(),
        recreate: false,
        model: None,
    };
    h.engine.handle_bridge(request(pane, command)).await
}

#[tokio::test]
async fn a_scheduled_turn_is_not_the_users_turn() {
    let h = harness();
    let checkout = tempfile::tempdir().unwrap();
    setup_team(&h, Some(checkout.path()));
    let master_id = h.engine.master_id().unwrap().expect("Master was created");
    let master = h.engine.list_bots().await.unwrap().into_iter().find(|v| v.bot.id == master_id).unwrap().bot;
    at(&h.engine, "2027-06-07 08:00");
    let task = NewTask { prompt: "Make an SEO bot.".into(), ..new_task(&master, "SEO") };
    h.engine.create_task(task).unwrap();

    tick_at(&h, "2027-06-07 09:00").await;
    let run = h.engine.sessions_of(&master.id).unwrap().remove(0);
    let e = create_bot(&h, &pane(&h, &run)).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::NotAllowed);
    assert!(e.message.starts_with("Create skills and bots only"), "{}", e.message);

    let (session, _) = h.engine.start_session_with_message(&master.id, "Make an SEO bot.", &[]).unwrap();
    h.engine.pump_session(&session.id).await;
    let made = create_bot(&h, &pane(&h, &session)).await;
    assert!(made.created.is_some(), "{:?}", made.error);
}

#[tokio::test(start_paused = true)]
async fn the_scheduler_ticks_on_its_own() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    wiki(&h, &bot);
    at(&h.engine, "2027-06-07 09:00");

    let scheduler = h.engine.run_scheduler(Duration::from_secs(60));
    settle().await;
    assert_eq!(runs(&h, &bot).len(), 1, "the first tick runs at once");
    complete(&h, &runs(&h, &bot)[0]).await;

    at(&h.engine, "2027-06-14 09:00");
    tokio::time::advance(Duration::from_secs(61)).await;
    settle().await;
    assert_eq!(runs(&h, &bot).len(), 2, "the next tick a minute later");

    h.engine.shutdown().await;
    tokio::time::advance(Duration::from_secs(61)).await;
    settle().await;
    assert!(scheduler.is_finished());
}

async fn bot_named(h: &Harness, handle: &str) -> Bot {
    h.engine.list_bots().await.unwrap().into_iter().find(|v| v.bot.handle == handle).unwrap().bot
}

async fn masters_turn(h: &Harness) -> (Session, String) {
    let checkout = tempfile::tempdir().unwrap();
    setup_team(h, Some(checkout.path()));
    let master = bot_named(h, "master").await;
    let (session, _) = h.engine.start_session_with_message(&master.id, "Plan my week.", &[]).unwrap();
    h.engine.pump_session(&session.id).await;
    let pane = pane(h, &session);
    (session, pane)
}

fn task_create(name: &str, bot: &str, every: u32, prompt: &str, recreate: bool) -> BridgeCommand {
    BridgeCommand::TaskCreate {
        name: name.into(),
        bot: bot.into(),
        every,
        at: None,
        from: None,
        prompt: prompt.into(),
        recreate,
    }
}

fn wiki_create() -> BridgeCommand {
    task_create("Wiki update", "@query", 7, "Check the repo.", false)
}

#[derive(Default)]
struct Edit {
    name: Option<String>,
    bot: Option<String>,
    every: Option<u32>,
    enabled: Option<bool>,
    prompt: Option<String>,
}

fn task_edit(task: &str) -> BridgeCommand {
    edit(task, Edit::default())
}

fn edit(task: &str, e: Edit) -> BridgeCommand {
    BridgeCommand::TaskEdit {
        task: task.into(),
        name: e.name,
        bot: e.bot,
        every: e.every,
        at: None,
        from: None,
        enabled: e.enabled,
        prompt: e.prompt,
    }
}

async fn run(h: &Harness, pane: &str, command: BridgeCommand) -> BridgeResponse {
    h.engine.handle_bridge(request(pane, command)).await
}

fn output(response: BridgeResponse) -> String {
    response.output.unwrap_or_else(|| panic!("no output: {:?}", response.error))
}

#[tokio::test]
async fn only_master_manages_tasks() {
    let h = harness();
    let (_, _) = masters_turn(&h).await;
    let query = bot_named(&h, "query").await;
    let (session, _) = h.engine.start_session_with_message(&query.id, "Hi.", &[]).unwrap();
    h.engine.pump_session(&session.id).await;
    let pane = pane(&h, &session);
    for (command, name) in [
        (BridgeCommand::Tasks, "tasks"),
        (wiki_create(), "task create"),
        (task_edit("Wiki update"), "task edit"),
        (BridgeCommand::TaskDelete { task: "Wiki update".into() }, "task delete"),
    ] {
        let e = run(&h, &pane, command).await.error.unwrap();
        assert_eq!(e.code, ErrorCode::NotAllowed);
        assert_eq!(e.message, format!("Only @master can use `sbot {name}`."));
    }
}

#[tokio::test]
async fn task_changes_need_the_users_turn() {
    let h = harness();
    let (master_session, mp) = masters_turn(&h).await;
    at(&h.engine, "2027-06-07 08:00");
    assert!(run(&h, &mp, wiki_create()).await.output.is_some());

    assert!(h.engine.handle_bridge(send(&mp, "@plan", "Plan it.")).await.ok);
    settle().await;
    let plan = bot_named(&h, "plan").await;
    let pp = pane(&h, &opened(&h, &plan, &master_session));
    assert!(h.engine.handle_bridge(reply(&pp, "Plan.")).await.ok);
    finish_turn(&h, &pp).await;
    finish_turn(&h, &mp).await;
    let denied = |r: BridgeResponse| {
        let e = r.error.expect("refused");
        assert_eq!(e.code, ErrorCode::NotAllowed);
        assert!(e.message.contains("user's message"), "{}", e.message);
    };
    denied(run(&h, &mp, task_create("Other", "@query", 7, "Go.", false)).await);
    denied(run(&h, &mp, task_edit("Wiki update")).await);
    denied(run(&h, &mp, BridgeCommand::TaskDelete { task: "Wiki update".into() }).await);
    assert!(run(&h, &mp, BridgeCommand::Tasks).await.output.is_some());

    let master = bot_named(&h, "master").await;
    let scheduled = NewTask { prompt: "Check.".into(), ..new_task(&master, "Self") };
    h.engine.create_task(scheduled).unwrap();
    tick_at(&h, "2027-06-08 09:00").await;
    let sessions = h.engine.sessions_of(&master.id).unwrap();
    let run_session = sessions.iter().find(|s| s.id != master_session.id).expect("the run");
    let rp = pane(&h, run_session);
    denied(run(&h, &rp, task_create("Other", "@query", 7, "Go.", false)).await);
    denied(run(&h, &rp, task_edit("Wiki update")).await);
    denied(run(&h, &rp, BridgeCommand::TaskDelete { task: "Wiki update".into() }).await);
    assert!(run(&h, &rp, BridgeCommand::Tasks).await.output.is_some());
}

#[tokio::test]
async fn master_creates_a_task_and_the_user_sees_it() {
    let h = harness();
    let (session, mp) = masters_turn(&h).await;
    at(&h.engine, "2027-06-07 08:00");
    let mut ui = h.engine.subscribe_ui();
    let text = output(run(&h, &mp, task_create("Wiki update", "@query", 7, "Check the repo.", false)).await);
    assert!(text.starts_with("Wiki update: @query, every 7 days at 09:00 from 2027-06-07, on\n"), "{text}");
    assert!(text.contains("  next run: 2027-06-07 09:00\n"), "{text}");
    let tasks = h.engine.list_tasks().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].prompt, "Check the repo.");
    assert!(events(&mut ui).contains(&UiEvent::TasksChanged));
    let notice = h
        .engine
        .session_messages(&session.id)
        .unwrap()
        .into_iter()
        .find(|m| m.system_code.as_deref() == Some("master_created_task"))
        .expect("a notice");
    assert_eq!(notice.system_params.unwrap(), serde_json::json!({"name": "Wiki update", "handle": "query"}));
}

#[tokio::test]
async fn a_repeated_create_returns_the_task() {
    let h = harness();
    let (_, mp) = masters_turn(&h).await;
    at(&h.engine, "2027-06-07 08:00");
    let first = output(run(&h, &mp, wiki_create()).await);
    let again = output(run(&h, &mp, wiki_create()).await);
    assert_eq!(again, format!("Already created.\n{first}"));
    assert_eq!(h.engine.list_tasks().unwrap().len(), 1);

    let e = run(&h, &mp, task_create("wiki UPDATE", "query", 7, "Check the repo.", false)).await;
    assert!(e.error.is_none(), "same inputs, same key in any case: {:?}", e.output);
    let e = run(&h, &mp, task_create("Wiki update", "@query", 3, "Check the repo.", false)).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::CreatedDifferently);

    let id = h.engine.list_tasks().unwrap().remove(0).id;
    h.engine.delete_task(&id).unwrap();
    let e = run(&h, &mp, wiki_create()).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::CreatedThenDeleted);
    assert!(e.message.contains("Wiki update"), "{}", e.message);
    assert!(h.engine.list_tasks().unwrap().is_empty());
    let text = output(run(&h, &mp, task_create("Wiki update", "@query", 7, "Check the repo.", true)).await);
    assert!(text.starts_with("Wiki update: @query"), "{text}");
    assert_eq!(h.engine.list_tasks().unwrap().len(), 1);
}

#[tokio::test]
async fn master_edits_by_name_in_any_case() {
    let h = harness();
    let (session, mp) = masters_turn(&h).await;
    at(&h.engine, "2027-06-07 08:00");
    output(run(&h, &mp, wiki_create()).await);

    let text = output(run(&h, &mp, edit("WIKI UPDATE", Edit { every: Some(10), ..Edit::default() })).await);
    assert!(text.starts_with("Wiki update: @query, every 10 days at 09:00 from 2027-06-07, on\n"), "{text}");
    assert!(h
        .engine
        .session_messages(&session.id)
        .unwrap()
        .iter()
        .any(|m| m.system_code.as_deref() == Some("master_changed_task")));

    let text = output(run(&h, &mp, edit("wiki update", Edit { enabled: Some(false), ..Edit::default() })).await);
    assert!(text.starts_with("Wiki update: @query, every 10 days at 09:00 from 2027-06-07, off\n"), "{text}");
    assert!(text.contains("  next run: none\n"), "{text}");
    let text = output(run(&h, &mp, edit("wiki update", Edit { enabled: Some(true), ..Edit::default() })).await);
    assert!(text.contains(", on\n"), "{text}");

    let change = Edit {
        prompt: Some("Check the docs.".into()),
        name: Some("Docs".into()),
        bot: Some("plan".into()),
        ..Edit::default()
    };
    let text = output(run(&h, &mp, edit("wiki update", change)).await);
    assert!(text.starts_with("Docs: @plan, every 10 days"), "{text}");
    assert!(text.ends_with("  prompt:\n    Check the docs."), "{text}");
    assert_eq!(h.engine.list_tasks().unwrap()[0].prompt, "Check the docs.");

    let e = run(&h, &mp, task_edit("Nope")).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::BadRequest);
    assert_eq!(e.message, "No recurring task named Nope; run `sbot tasks`.");
    let e = run(&h, &mp, task_edit("Docs")).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::BadRequest);
    assert_eq!(e.message, "Nothing to change: give --name, --bot, --every, --at, --from, --on, --off or --prompt.");
    output(run(&h, &mp, task_create("Other", "@query", 2, "Go.", false)).await);
    let e = run(&h, &mp, edit("Docs", Edit { name: Some("OTHER".into()), ..Edit::default() })).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::BadRequest);
    assert!(e.message.contains("OTHER"), "{}", e.message);

    let text = output(run(&h, &mp, BridgeCommand::TaskDelete { task: "other".into() }).await);
    assert_eq!(text, "Deleted the recurring task Other. Its past runs' sessions stay.");
    assert_eq!(h.engine.list_tasks().unwrap().len(), 1);
    let e = run(&h, &mp, BridgeCommand::TaskDelete { task: "other".into() }).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::BadRequest);
}

#[tokio::test]
async fn tasks_lists_every_task_with_its_prompt() {
    let h = harness_after(|_| {});
    let (_, mp) = masters_turn(&h).await;
    assert_eq!(output(run(&h, &mp, BridgeCommand::Tasks).await), "No recurring tasks yet.");
    let query = bot_named(&h, "query").await;
    let plan = bot_named(&h, "plan").await;
    at(&h.engine, "2027-05-31 08:00");
    let mut first = new_task(&query, "Wiki update");
    first.prompt = "Check the repo.".into();
    first.starts_on = Some("2027-05-31".into());
    let wiki = h.engine.create_task(first).unwrap();
    let off = h.engine.create_task(new_task(&plan, "Digest")).unwrap();
    h.engine.set_task_enabled(&off.id, false).unwrap();

    tick_at(&h, "2027-05-31 09:00").await;
    let run1 = h.engine.sessions_of(&query.id).unwrap().remove(0);
    complete(&h, &run1).await;
    tick_at(&h, "2027-06-07 09:00").await;
    let run2 = h.engine.sessions_of(&query.id).unwrap().into_iter().find(|s| s.id != run1.id).unwrap();
    tick_at(&h, "2027-06-14 09:00").await;
    let wiki_now = task_now(&h, &wiki.id);
    assert_eq!(wiki_now.skipped_at, Some(local("2027-06-14 09:00")));
    let completed = minute(&Store::open(&h.paths.db_path()).unwrap().last_completed_run(&wiki.id).unwrap().unwrap());

    let text = output(run(&h, &mp, BridgeCommand::Tasks).await);
    let expected = format!(
        "Wiki update: @query, every 7 days at 09:00 from 2027-05-31, on\n\
         \x20 next run: 2027-06-21 09:00\n\
         \x20 last run: {} (@query/{}) · last completed run: {completed}\n\
         \x20 skipped: 2027-06-14 09:00, the last run was still busy\n\
         \x20 prompt:\n\
         \x20   Check the repo.\n\
         Digest: @plan, every 7 days at 09:00 from {}, off\n\
         \x20 next run: none\n\
         \x20 prompt:\n\
         \x20   Update the wiki.",
        minute(wiki_now.last_run_at.as_deref().unwrap()),
        run2.number,
        off.starts_on,
    );
    assert_eq!(text, expected);

    h.engine.remove_session(&run2.id).await.unwrap();
    let text = output(run(&h, &mp, BridgeCommand::Tasks).await);
    assert!(text.contains("last run: (removed) · last completed run: "), "{text}");

    tick_at(&h, "2027-06-21 09:00").await;
    let db = rusqlite::Connection::open(h.paths.db_path()).unwrap();
    db.execute("UPDATE tasks SET last_run_at = ?1 WHERE id = ?2", [local("2027-06-21 09:00"), wiki.id.clone()])
        .unwrap();
    let text = output(run(&h, &mp, BridgeCommand::Tasks).await);
    assert!(text.contains("  last run: 2027-06-21 09:00 (@query/"), "{text}");
    assert!(!text.contains("skipped:"), "{text}");
}

#[tokio::test]
async fn editing_an_off_task_keeps_its_start_date() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let task = wiki(&h, &bot);
    h.engine.set_task_enabled(&task.id, false).unwrap();

    at(&h.engine, "2027-06-09 10:00");
    let edited = h.engine.update_task(&task.id, NewTask { every_days: 14, ..new_task(&bot, "Wiki update") }).unwrap();
    assert_eq!((edited.starts_on.as_str(), edited.enabled, edited.next_run_at.clone()), ("2027-06-07", false, None));
    let on = h.engine.set_task_enabled(&task.id, true).unwrap();
    assert_eq!(on.next_run_at, Some(local("2027-06-21 09:00")));

    h.engine.set_task_enabled(&task.id, false).unwrap();
    let (_, mp) = masters_turn(&h).await;
    let text = output(run(&h, &mp, edit("wiki update", Edit { every: Some(3), ..Edit::default() })).await);
    assert!(text.starts_with("Wiki update: @writer, every 3 days at 09:00 from 2027-06-07, off\n"), "{text}");
}
