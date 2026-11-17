#![allow(clippy::await_holding_lock)]
mod support;

use std::time::Duration;

use bridge_proto::ErrorCode;
use sonata_core::engine::UiEvent;
use sonata_core::model::*;
use sonata_core::runtime::{AgentState, RuntimeEvent};
use sonata_core::store::Store;
use sonata_core::{CoreError, Refusal};
use support::*;

async fn two_working_sessions(h: &Harness) -> (Bot, [Session; 2], [Message; 2], [String; 2]) {
    let bot = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let sessions = [new_session(h, &bot.id), new_session(h, &bot.id)];
    let messages = [
        h.engine.queue_user_message_to_session(&sessions[0].id, "review A", &[]).unwrap(),
        h.engine.queue_user_message_to_session(&sessions[1].id, "review B", &[]).unwrap(),
    ];
    h.engine.pump(&bot.id).await;
    let panes = {
        let s = h.rt.state();
        sessions.clone().map(|session| s.pane_of(&agent(&session)).expect("the session's CLI started"))
    };
    (bot, sessions, messages, panes)
}

fn in_thread(h: &Harness, of: &Message) -> Vec<Message> {
    h.engine.thread_messages(&of.thread_id).unwrap()
}

#[tokio::test]
async fn two_sessions_of_one_bot_work_at_once_each_on_its_own_pane() {
    let h = harness_after(|_| {});
    let (bot, [one, two], [a, b], [p1, p2]) = two_working_sessions(&h).await;
    assert_eq!((one.number, two.number), (1, 2));

    let s = h.rt.state();
    assert_eq!(s.starts.len(), 2);
    for (session, label) in [(&one, "reviewer-1"), (&two, "reviewer-2")] {
        let spec = s.starts.iter().find(|spec| spec.agent == agent(session)).expect("started under its agent name");
        assert_eq!((spec.label.as_str(), spec.handle.as_str()), (label, "reviewer"));
        assert_eq!(spec.agent.len(), 27);
        assert!(spec.agent.starts_with('s'));
        assert!(spec.agent.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()), "{}", spec.agent);
    }
    assert_ne!(s.starts[0].agent, s.starts[1].agent);
    assert_ne!(p1, p2);
    let mut prompts: Vec<(String, String)> =
        s.prompts.iter().map(|(pane, text)| (pane.clone(), text.lines().nth(1).unwrap().to_string())).collect();
    prompts.sort();
    assert_eq!(prompts, vec![(p1.clone(), "review A".to_string()), (p2.clone(), "review B".to_string())]);
    drop(s);

    for m in [&a, &b] {
        let err = h.engine.cancel_message(&m.id).await.unwrap_err();
        assert!(matches!(err, CoreError::Refused(Refusal::AlreadyDelivering)), "{err}");
    }
    assert_eq!((a.recipient_session.as_deref(), b.recipient_session.as_deref()), (Some(&*one.id), Some(&*two.id)));
    assert_ne!(a.thread_id, b.thread_id, "each session has its own chat with the user");
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
}

#[tokio::test]
async fn a_message_to_a_session_is_checked_like_any_message_from_the_user() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let one = new_session(&h, &bot.id);
    let err = h.engine.queue_user_message_to_session(&one.id, "  ", &[]).unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::MessageEmpty)), "{err}");
    let err = h.engine.queue_user_message_to_session("missing", "hi", &[]).unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::SessionNotFound(_))), "{err}");
}

#[tokio::test]
async fn one_session_ends_its_turn_while_the_other_keeps_working() {
    let h = harness_after(|_| {});
    let (bot, _, [a, b], [p1, p2]) = two_working_sessions(&h).await;
    h.rt.state().pane_outputs.insert(p1.clone(), "A looks fine".into());
    let mut ui = h.engine.subscribe_ui();

    finish_turn(&h, &p1).await;

    let thread_a = in_thread(&h, &a);
    assert!(thread_a.iter().any(|m| m.kind == MessageKind::RawFallback && m.body == "A looks fine"), "{thread_a:?}");
    let err = h.engine.cancel_message(&b.id).await.unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::AlreadyDelivering)), "session 2 still works: {err}");
    assert_eq!(in_thread(&h, &b).len(), 1);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working, "the bot works while one session does");

    h.engine.handle_bridge(reply(&p2, "B looks fine")).await;
    finish_turn(&h, &p2).await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Idle);
    let statuses: Vec<BotStatus> = std::iter::from_fn(|| ui.try_recv().ok())
        .filter_map(|e| match e {
            UiEvent::BotStatus { bot_id, status } if bot_id == bot.id => Some(status),
            _ => None,
        })
        .collect();
    assert_eq!(statuses, vec![BotStatus::Idle], "the bot's status changed once, when its last turn ended");
}

#[tokio::test]
async fn a_reply_from_a_sessions_pane_lands_in_that_sessions_chat() {
    let h = harness_after(|_| {});
    let (_, [_, two], [a, b], [_, p2]) = two_working_sessions(&h).await;
    let resp = h.engine.handle_bridge(reply(&p2, "B looks fine")).await;
    assert!(resp.ok, "{resp:?}");

    let answer = in_thread(&h, &b).into_iter().find(|m| m.body == "B looks fine").expect("in session 2's chat");
    assert_eq!(answer.sender_session.as_deref(), Some(two.id.as_str()));
    assert_eq!((answer.recipient.as_str(), answer.in_reply_to.as_deref()), (USER, Some(b.id.as_str())));
    assert!(in_thread(&h, &a).iter().all(|m| m.body != "B looks fine"));
}

#[tokio::test]
async fn each_session_saves_and_resumes_its_own_cli_conversation() {
    let h = harness_after(|_| {});
    let (bot, [one, two], _, [p1, p2]) = two_working_sessions(&h).await;
    {
        let mut s = h.rt.state();
        s.pane_session_ids.insert(p1.clone(), "cli-one".into());
        s.pane_session_ids.insert(p2.clone(), "cli-two".into());
    }
    for pane in [&p1, &p2] {
        h.engine.handle_bridge(reply(pane, "done")).await;
        finish_turn(&h, pane).await;
    }
    let saved = |h: &Harness, session: &Session| {
        h.engine.sessions_of(&bot.id).unwrap().into_iter().find(|s| s.id == session.id).unwrap().provider_session_id
    };
    assert_eq!((saved(&h, &one).as_deref(), saved(&h, &two).as_deref()), (Some("cli-one"), Some("cli-two")));

    let next = relaunch(&h);
    next.engine.queue_user_message_to_session(&one.id, "again A", &[]).unwrap();
    next.engine.queue_user_message_to_session(&two.id, "again B", &[]).unwrap();
    next.engine.pump(&bot.id).await;
    let s = next.rt.state();
    assert_eq!(s.starts.len(), 2);
    for (session, cli_id) in [(&one, "cli-one"), (&two, "cli-two")] {
        let spec = s.starts.iter().find(|spec| spec.agent == agent(session)).unwrap();
        assert!(spec.args.ends_with(&["--resume".to_string(), cli_id.to_string()]), "{:?}", spec.args);
    }
}

#[tokio::test]
async fn a_silent_turns_fallback_and_a_notice_land_in_the_session_they_concern() {
    let h = harness_after(|_| {});
    let (_, [one, two], [a, b], [p1, p2]) = two_working_sessions(&h).await;
    h.rt.state().pane_outputs.insert(p2.clone(), "B, half reviewed".into());
    finish_turn(&h, &p2).await;
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: p1.clone(), state: AgentState::Blocked }).await;

    let thread_b = in_thread(&h, &b);
    let fallback = thread_b.iter().find(|m| m.kind == MessageKind::RawFallback).expect("in session 2's chat");
    assert_eq!((fallback.body.as_str(), fallback.sender_session.as_deref()), ("B, half reviewed", Some(&*two.id)));
    let thread_a = in_thread(&h, &a);
    let blocked =
        thread_a.iter().find(|m| m.system_code.as_deref() == Some("bot_blocked")).expect("in session 1's chat");
    assert_eq!(blocked.sender_session.as_deref(), Some(one.id.as_str()));
    assert!(thread_a.iter().all(|m| m.kind != MessageKind::RawFallback));
    assert!(thread_b.iter().all(|m| m.system_code.is_none()));
}

#[tokio::test]
async fn a_bot_cannot_be_deleted_while_one_of_its_sessions_starts() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let [one, two] = [new_session(&h, &bot.id), new_session(&h, &bot.id)];
    h.engine.queue_user_message_to_session(&one.id, "review A", &[]).unwrap();
    h.engine.pump_session(&one.id).await;
    h.rt.state().hang_starts = true;
    h.engine.queue_user_message_to_session(&two.id, "review B", &[]).unwrap();
    let pump = tokio::spawn(h.engine.pump_session(&two.id));
    settle().await;
    assert_eq!(h.rt.state().starts.len(), 2, "session 2 is starting");

    let err = h.engine.delete_bot(&bot.id).await.unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::BotStarting)), "{err}");
    assert!(h.rt.state().stopped.is_empty());
    pump.abort();
}

#[tokio::test]
async fn deleting_a_bot_closes_the_cli_of_each_of_its_sessions() {
    let h = harness_after(|_| {});
    let (bot, _, [a, b], [p1, p2]) = two_working_sessions(&h).await;
    h.engine.delete_bot(&bot.id).await.unwrap();
    let mut stopped = h.rt.state().stopped.clone();
    stopped.sort();
    assert_eq!(stopped, vec![p1.clone(), p2]);
    for m in [&a, &b] {
        assert_eq!(h.engine.thread_messages(&m.thread_id).unwrap()[0].status, MessageStatus::Cancelled);
    }
    assert!(h.engine.handle_bridge(reply(&p1, "late")).await.error.is_some(), "no bot behind the pane any more");
}

#[tokio::test(start_paused = true)]
async fn a_bots_sessions_start_one_at_a_time() {
    let h = harness_after(|_| {});
    h.rt.state().start_delay = Some(Duration::from_secs(5));
    let bot = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let [one, two] = [new_session(&h, &bot.id), new_session(&h, &bot.id)];
    h.engine.queue_user_message_to_session(&one.id, "review A", &[]).unwrap();
    h.engine.queue_user_message_to_session(&two.id, "review B", &[]).unwrap();
    let pump = tokio::spawn(h.engine.pump(&bot.id));
    while h.rt.state().starts.is_empty() {
        tokio::task::yield_now().await;
    }
    settle().await;
    assert_eq!(h.rt.state().starts.len(), 1, "the sibling waits for the first start to finish");
    pump.await.unwrap();
    let s = h.rt.state();
    assert_eq!((s.starts.len(), s.prompts.len()), (2, 2));
}

#[tokio::test(start_paused = true)]
async fn a_cli_whose_session_went_away_during_its_start_is_closed() {
    let h = harness_after(|_| {});
    h.rt.state().start_delay = Some(Duration::from_secs(5));
    let bot = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    h.engine.queue_user_message(&bot.id, "review A").unwrap();
    let pump = tokio::spawn(h.engine.pump(&bot.id));
    while h.rt.state().starts.is_empty() {
        tokio::task::yield_now().await;
    }
    Store::open(&h.paths.db_path()).unwrap().delete_bot(&bot.id).unwrap().unwrap();
    pump.await.unwrap();
    let s = h.rt.state();
    assert_eq!(s.stopped, vec!["w1:p1".to_string()], "the new CLI was closed");
    assert!(s.prompts.is_empty());
    drop(s);
    assert!(h.engine.handle_bridge(reply("w1:p1", "late")).await.error.is_some(), "no live entry is left");
}

#[tokio::test]
async fn restarting_one_session_leaves_its_sibling_working() {
    let h = harness_after(|_| {});
    let (bot, [_, two], [a, b], [p1, p2]) = two_working_sessions(&h).await;
    h.engine.restart_session(&two.id).await.unwrap();
    settle().await;
    assert_eq!(h.rt.state().stopped, vec![p2]);
    assert_eq!(in_thread(&h, &b)[0].status, MessageStatus::Interrupted);
    assert_eq!(in_thread(&h, &a)[0].status, MessageStatus::Delivered);
    assert!(h.engine.handle_bridge(reply(&p1, "A is fine")).await.ok, "session 1's turn is still open");
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
}

#[tokio::test]
async fn the_watchdog_checks_each_session_on_its_own() {
    let h = harness_after(|_| {});
    let (bot, [one, _], [a, b], [p1, _]) = two_working_sessions(&h).await;
    {
        let mut s = h.rt.state();
        s.state = AgentState::Working;
        s.pane_states.insert(p1.clone(), AgentState::Blocked);
    }
    h.engine.watchdog_tick().await;

    assert_eq!(status_of(&h, &bot.id).await, BotStatus::NeedsAttention, "one session needs the user");
    let blocked: Vec<Message> =
        in_thread(&h, &a).into_iter().filter(|m| m.system_code.as_deref() == Some("bot_blocked")).collect();
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].sender_session.as_deref(), Some(one.id.as_str()));
    assert!(in_thread(&h, &b).iter().all(|m| m.system_code.is_none()));
    let err = h.engine.cancel_message(&b.id).await.unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::AlreadyDelivering)), "session 2 still works: {err}");
}

#[tokio::test]
async fn a_session_is_active_when_a_message_arrives_and_when_its_turn_starts() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let [one, two] = [new_session(&h, &bot.id), new_session(&h, &bot.id)];
    let order = |h: &Harness| h.engine.sessions_of(&bot.id).unwrap().into_iter().map(|s| s.number).collect::<Vec<_>>();
    let later = || std::thread::sleep(Duration::from_millis(5));
    h.engine.queue_user_message_to_session(&one.id, "A1", &[]).unwrap();
    h.engine.queue_user_message_to_session(&one.id, "A2", &[]).unwrap();
    h.engine.pump_session(&one.id).await;
    later();
    h.engine.queue_user_message_to_session(&two.id, "B1", &[]).unwrap();
    assert_eq!(order(&h), vec![2, 1], "a message arrived in session 2");
    later();
    let p1 = h.rt.state().pane_of(&agent(&one)).unwrap();
    h.engine.handle_bridge(reply(&p1, "A1 done")).await;
    finish_turn(&h, &p1).await;
    assert_eq!(h.rt.state().prompts.last().unwrap().1.lines().nth(1), Some("A2"));
    assert_eq!(order(&h), vec![1, 2], "session 1's next turn started");
}

#[tokio::test]
async fn the_first_message_starts_a_session_titled_by_its_first_line() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let (one, first) = h
        .engine
        .start_session_with_message(&bot.id, "\n  Review the login flow  \nIt logs out at random.", &[])
        .unwrap();
    assert_eq!((one.number, one.title.as_deref(), one.opened_by.as_deref()), (1, Some("Review the login flow"), None));
    assert_eq!((first.recipient_session.as_deref(), first.status), (Some(one.id.as_str()), MessageStatus::Queued));

    let (two, _) = h.engine.start_session_with_message(&bot.id, &"é".repeat(70), &[]).unwrap();
    assert_eq!(two.title, Some(format!("{}…", "é".repeat(59))));

    let src = tempfile::tempdir().unwrap();
    let notes = src.path().join("notes.md");
    std::fs::write(&notes, "# Notes").unwrap();
    let (three, files_only) =
        h.engine.start_session_with_message(&bot.id, " \n", &[notes.to_string_lossy().into_owned()]).unwrap();
    assert_eq!((three.number, three.title), (3, None));
    assert_eq!(files_only.attachments.len(), 1);

    let err = h.engine.start_session_with_message(&bot.id, " ", &[]).unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::MessageEmpty)), "{err}");
    let err = h.engine.start_session_with_message("missing", "hi", &[]).unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::BotNotFound(_))), "{err}");
    assert_eq!(h.engine.sessions_of(&bot.id).unwrap().len(), 3, "a refused message starts no session");

    let second = h.engine.queue_user_message_to_session(&one.id, "And the signup flow.", &[]).unwrap();
    assert_eq!((second.recipient_session.as_deref(), &second.thread_id), (Some(one.id.as_str()), &first.thread_id));
    assert_eq!(h.engine.sessions_of(&bot.id).unwrap().len(), 3);
    h.engine.pump_session(&one.id).await;
    let s = h.rt.state();
    assert_eq!(s.prompts.len(), 1);
    assert_eq!(s.prompts[0].0, s.pane_of(&agent(&one)).unwrap());
    assert!(s.prompts[0].1.contains("\n\n  Review the login flow  \nIt logs out at random."), "{}", s.prompts[0].1);
}

#[tokio::test]
async fn a_relaunch_cuts_off_one_sessions_turn_and_keeps_the_turn_its_sibling_began() {
    let ids = std::sync::Mutex::new(Vec::new());
    let h = harness_after(|store| {
        let bot = store.insert_bot(&claude_bot("Reviewer"), "reviewer").unwrap();
        let [one, two] = [(); 2].map(|()| store.create_session(&bot.id, None, None).unwrap());
        let to = |session: &Session, body: &str, status| NewMessage {
            sender: USER.into(),
            recipient: bot.id.clone(),
            sender_session: None,
            recipient_session: Some(session.id.clone()),
            kind: MessageKind::Text,
            body: body.into(),
            attachments: vec![],
            system_code: None,
            system_params: None,
            status,
            chain_id: sonata_core::ids::new_id(),
            hop: 0,
            in_reply_to: None,
        };
        let cut = store.insert_message(&to(&one, "cut off by the quit", MessageStatus::Delivered)).unwrap();
        let waiting = store.insert_message(&to(&one, "still waiting", MessageStatus::Queued)).unwrap();
        *ids.lock().unwrap() = vec![bot.id, one.id, two.id, cut.id, waiting.id];
    });
    let [bot_id, one, two, cut, waiting] = ids.into_inner().unwrap().try_into().unwrap();
    let begun = h.engine.queue_user_message_to_session(&two, "review B", &[]).unwrap();
    h.engine.pump_session(&two).await;

    h.engine.recover().await.unwrap();

    let store = Store::open(&h.paths.db_path()).unwrap();
    let status = |id: &str| store.get_message(id).unwrap().unwrap().status;
    assert_eq!(status(&cut), MessageStatus::Interrupted);
    assert_eq!(status(&begun.id), MessageStatus::Delivered, "session 2's turn goes on");
    assert_eq!(status(&waiting), MessageStatus::Delivered);
    let err = h.engine.cancel_message(&begun.id).await.unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::AlreadyDelivering)), "still in flight: {err}");
    let s = h.rt.state();
    let session = |id: &str| store.get_session(id).unwrap().unwrap();
    let prompts: Vec<(String, Option<&str>)> =
        s.prompts.iter().map(|(pane, text)| (pane.clone(), text.lines().nth(1))).collect();
    assert_eq!(
        prompts,
        vec![
            (s.pane_of(&agent(&session(&two))).unwrap(), Some("review B")),
            (s.pane_of(&agent(&session(&one))).unwrap(), Some("still waiting")),
        ]
    );
    assert_eq!(store.sessions_of(&bot_id).unwrap().len(), 2);
}

async fn start_another_session(h: &Harness, bot: &Bot) {
    let session = new_session(h, &bot.id);
    h.engine.queue_user_message_to_session(&session.id, "hello", &[]).unwrap();
    h.engine.pump(&bot.id).await;
    assert!(h.rt.state().pane_of(&agent(&session)).is_some(), "the session's CLI started");
}

#[tokio::test]
async fn a_later_session_start_keeps_the_folder_files_until_something_changes() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let agents = h.paths.bots_root.join(&bot.handle).join("AGENTS.md");
    let marker = |text: &str| std::fs::write(&agents, text).unwrap();
    let kept = || std::fs::read_to_string(&agents).unwrap() == "marker";

    start_another_session(&h, &bot).await;
    assert!(std::fs::read_to_string(&agents).unwrap().contains("Reviewer"));
    marker("marker");
    start_another_session(&h, &bot).await;
    assert!(kept(), "a second session leaves the files alone");

    h.engine.update_bot(&bot.id, NewBot { instructions: "Be brief.".into(), ..claude_bot("Reviewer") }).unwrap();
    start_another_session(&h, &bot).await;
    assert!(!kept(), "an edit rewrites them");

    marker("marker");
    start_another_session(&h, &bot).await;
    assert!(kept());
    h.engine.set_language("fr").unwrap();
    start_another_session(&h, &bot).await;
    assert!(!kept(), "a language change rewrites them");

    marker("marker");
    start_another_session(&h, &bot).await;
    assert!(kept());
    std::fs::remove_file(&agents).unwrap();
    start_another_session(&h, &bot).await;
    assert!(agents.is_file(), "a missing AGENTS.md is written again");
}

fn position(events: &[UiEvent], found: impl Fn(&UiEvent) -> bool) -> usize {
    events.iter().position(found).expect("the event was sent")
}

#[tokio::test]
async fn the_first_message_announces_its_session_before_the_message() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let mut rx = h.engine.subscribe_ui();
    let (session, message) = h.engine.start_session_with_message(&bot.id, "Review the diff", &[]).unwrap();
    let seen = events(&mut rx);
    let added = position(&seen, |e| matches!(e, UiEvent::SessionAdded { session: s } if s.session.id == session.id));
    let msg = position(&seen, |e| matches!(e, UiEvent::MessageAdded { message: m } if m.id == message.id));
    assert!(added < msg, "{seen:?}");
    let UiEvent::SessionAdded { session: view } = &seen[added] else { unreachable!() };
    assert_eq!((view.session.title.as_deref(), view.status), (Some("Review the diff"), BotStatus::Offline));
    assert_eq!(seen.len(), 2, "no further events: {seen:?}");

    let other = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let first = h.engine.queue_user_message(&other.id, "Hi").unwrap();
    let second = h.engine.queue_user_message(&other.id, "Again").unwrap();
    let seen = events(&mut rx);
    let added = position(&seen, |e| matches!(e, UiEvent::SessionAdded { .. }));
    assert!(added < position(&seen, |e| matches!(e, UiEvent::MessageAdded { message: m } if m.id == first.id)));
    assert_eq!(seen.iter().filter(|e| matches!(e, UiEvent::SessionAdded { .. })).count(), 1, "{seen:?}");
    let upd = position(&seen, |e| matches!(e, UiEvent::SessionUpdated { .. }));
    assert!(upd < position(&seen, |e| matches!(e, UiEvent::MessageAdded { message: m } if m.id == second.id)));
}

#[tokio::test]
async fn a_send_that_opens_a_session_announces_it_before_the_message() {
    let h = harness();
    let (_, writer, _) = team(&h).await;
    let mut rx = h.engine.subscribe_ui();
    assert!(h.engine.handle_bridge(send("w1:p1", "@writer", "Draft.")).await.ok);
    let seen = events(&mut rx);
    let session = h.engine.sessions_of(&writer.id).unwrap().remove(0);
    let added = position(&seen, |e| matches!(e, UiEvent::SessionAdded { session: s } if s.session.id == session.id));
    let msg = position(
        &seen,
        |e| matches!(e, UiEvent::MessageAdded { message: m } if m.recipient_session.as_deref() == Some(&*session.id)),
    );
    assert!(added < msg, "{seen:?}");
    assert_eq!(seen.iter().filter(|e| matches!(e, UiEvent::SessionAdded { .. })).count(), 1, "{seen:?}");

    h.engine.handle_bridge(reply("w1:p1", "Asked.")).await;
    finish_turn(&h, "w1:p1").await;
    let mut rx = h.engine.subscribe_ui();
    assert!(h.engine.handle_bridge(send("w1:p1", "@writer", "More.")).await.ok);
    let seen = events(&mut rx);
    assert!(!seen.iter().any(|e| matches!(e, UiEvent::SessionAdded { .. })), "{seen:?}");
    assert!(seen.iter().any(|e| matches!(e, UiEvent::SessionUpdated { session: s } if s.session.id == session.id)));
}

#[tokio::test]
async fn a_session_status_change_is_sent_with_its_ids_next_to_the_bots_aggregate() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let [one, two] = [new_session(&h, &bot.id), new_session(&h, &bot.id)];
    let mut rx = h.engine.subscribe_ui();
    h.engine.queue_user_message_to_session(&one.id, "review A", &[]).unwrap();
    h.engine.pump(&bot.id).await;
    let p1 = pane(&h, &one);
    let seen = events(&mut rx);
    let statuses: Vec<_> = seen
        .iter()
        .filter_map(|e| match e {
            UiEvent::SessionStatus { session_id, bot_id, status } => {
                Some((session_id.as_str(), bot_id.as_str(), *status))
            }
            _ => None,
        })
        .collect();
    assert!(statuses.contains(&(one.id.as_str(), bot.id.as_str(), BotStatus::Starting)), "{statuses:?}");
    assert!(statuses.contains(&(one.id.as_str(), bot.id.as_str(), BotStatus::Working)), "{statuses:?}");
    assert!(statuses.iter().all(|s| s.0 == one.id), "the idle session sent nothing: {statuses:?}");
    assert!(seen.iter().any(|e| matches!(e, UiEvent::BotStatus { status: BotStatus::Working, .. })));

    h.engine.queue_user_message_to_session(&two.id, "review B", &[]).unwrap();
    h.engine.pump(&bot.id).await;
    events(&mut rx);
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: p1, state: AgentState::Blocked }).await;
    let seen = events(&mut rx);
    assert!(
        seen.contains(&UiEvent::SessionStatus {
            session_id: one.id.clone(),
            bot_id: bot.id.clone(),
            status: BotStatus::NeedsAttention
        }),
        "{seen:?}"
    );
    assert!(seen.contains(&UiEvent::BotStatus { bot_id: bot.id.clone(), status: BotStatus::NeedsAttention }));
    assert_eq!(seen.iter().filter(|e| matches!(e, UiEvent::SessionStatus { .. })).count(), 1, "{seen:?}");
}

#[tokio::test]
async fn a_message_to_a_session_updates_its_activity() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let one = new_session(&h, &bot.id);
    let before = one.last_activity_at.clone();
    tokio::time::sleep(Duration::from_millis(5)).await;
    let mut rx = h.engine.subscribe_ui();
    h.engine.queue_user_message_to_session(&one.id, "review A", &[]).unwrap();
    let seen = events(&mut rx);
    let updated = seen
        .iter()
        .find_map(|e| match e {
            UiEvent::SessionUpdated { session } if session.session.id == one.id => Some(session.clone()),
            _ => None,
        })
        .expect("SessionUpdated");
    assert!(updated.session.last_activity_at > before, "{} vs {before}", updated.session.last_activity_at);
    assert!(
        position(&seen, |e| matches!(e, UiEvent::SessionUpdated { .. }))
            < position(&seen, |e| matches!(e, UiEvent::MessageAdded { .. }))
    );

    tokio::time::sleep(Duration::from_millis(5)).await;
    h.engine.pump(&bot.id).await;
    let seen = events(&mut rx);
    let delivered = seen
        .iter()
        .find_map(|e| match e {
            UiEvent::SessionUpdated { session } => Some(session.clone()),
            _ => None,
        })
        .expect("SessionUpdated at delivery");
    assert!(delivered.session.last_activity_at > updated.session.last_activity_at);
    assert_eq!(delivered.session.last_activity_at, h.engine.sessions_of(&bot.id).unwrap()[0].last_activity_at);
}

#[tokio::test]
async fn a_message_the_hop_limit_refused_is_no_activity() {
    let h = harness_after(|store| store.set_setting("hop_limit", "1").unwrap());
    let (researcher, writer, _) = team(&h).await;
    h.engine.handle_bridge(send("w1:p1", "@writer", "Draft.")).await;
    settle().await;
    let before = h.engine.sessions_of(&researcher.id).unwrap()[0].last_activity_at.clone();
    tokio::time::sleep(Duration::from_millis(5)).await;
    let mut rx = h.engine.subscribe_ui();
    finish_turn(&h, "w1:p2").await;
    let seen = events(&mut rx);
    assert!(
        seen.iter().any(|e| matches!(e, UiEvent::MessageAdded { message: m } if m.status == MessageStatus::Failed)),
        "{seen:?}"
    );
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, UiEvent::SessionUpdated { session } if session.session.bot_id == researcher.id)),
        "{seen:?}"
    );
    assert_eq!(h.engine.sessions_of(&researcher.id).unwrap()[0].last_activity_at, before);
    let _ = writer;
}

#[tokio::test]
async fn sessions_are_listed_most_recent_first_with_their_statuses() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let other = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let [one, two] = [new_session(&h, &bot.id), new_session(&h, &bot.id)];
    let theirs = new_session(&h, &other.id);
    tokio::time::sleep(Duration::from_millis(5)).await;
    h.engine.queue_user_message_to_session(&one.id, "review A", &[]).unwrap();
    h.engine.pump(&bot.id).await;

    let mine = h.engine.list_sessions(&bot.id).await.unwrap();
    let ids: Vec<_> = mine.iter().map(|v| (v.session.id.as_str(), v.status)).collect();
    assert_eq!(ids, vec![(one.id.as_str(), BotStatus::Working), (two.id.as_str(), BotStatus::Offline)]);

    let all = h.engine.all_sessions().await.unwrap();
    let ids: Vec<_> = all.iter().map(|v| v.session.id.as_str()).collect();
    assert_eq!(ids, vec![one.id.as_str(), two.id.as_str(), theirs.id.as_str()]);
    assert_eq!(all[0].status, BotStatus::Working);

    let json = serde_json::to_value(&all[0]).unwrap();
    assert_eq!(json["status"], "working");
    assert_eq!(json["id"], one.id.as_str());
    assert_eq!(json["bot_id"], bot.id.as_str());
}

#[tokio::test]
async fn a_message_from_a_session_is_activity_of_that_session_too() {
    let h = harness();
    let (researcher, writer, _) = team(&h).await;
    let session_of = |bot: &Bot| h.engine.sessions_of(&bot.id).unwrap().remove(0);
    let before = session_of(&researcher).last_activity_at;
    tokio::time::sleep(Duration::from_millis(5)).await;

    let mut rx = h.engine.subscribe_ui();
    assert!(h.engine.handle_bridge(reply("w1:p1", "Hi!")).await.ok);
    let seen = events(&mut rx);
    let sender = session_of(&researcher);
    assert!(sender.last_activity_at > before, "{} vs {before}", sender.last_activity_at);
    let upd = position(
        &seen,
        |e| matches!(e, UiEvent::SessionUpdated { session: s } if s.session.id == sender.id && s.session.last_activity_at == sender.last_activity_at),
    );
    assert!(upd < position(&seen, |e| matches!(e, UiEvent::MessageAdded { .. })), "{seen:?}");

    tokio::time::sleep(Duration::from_millis(5)).await;
    let mut rx = h.engine.subscribe_ui();
    assert!(h.engine.handle_bridge(send("w1:p1", "@writer", "Draft.")).await.ok);
    let seen = events(&mut rx);
    let (sender, recipient) = (session_of(&researcher), session_of(&writer));
    assert_eq!(sender.last_activity_at, recipient.last_activity_at);
    let sender_upd =
        position(&seen, |e| matches!(e, UiEvent::SessionUpdated { session: s } if s.session.id == sender.id));
    let added = position(&seen, |e| matches!(e, UiEvent::SessionAdded { session: s } if s.session.id == recipient.id));
    assert!(added < position(&seen, |e| matches!(e, UiEvent::MessageAdded { .. })), "{seen:?}");
    assert!(sender_upd < position(&seen, |e| matches!(e, UiEvent::MessageAdded { .. })), "{seen:?}");

    finish_turn(&h, "w1:p2").await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    let mut rx = h.engine.subscribe_ui();
    assert!(h.engine.handle_bridge(send("w1:p1", "@writer", "More.")).await.ok);
    let seen = events(&mut rx);
    for s in [session_of(&researcher), session_of(&writer)] {
        let n = seen
            .iter()
            .filter(|e| matches!(e, UiEvent::SessionUpdated { session } if session.session.id == s.id))
            .count();
        assert_eq!(n, 1, "{seen:?}");
    }
}

async fn session_status(h: &Harness, bot_id: &str, session: &Session) -> BotStatus {
    h.engine
        .all_sessions()
        .await
        .unwrap()
        .into_iter()
        .find(|v| v.session.id == session.id && v.session.bot_id == bot_id)
        .unwrap()
        .status
}

#[tokio::test]
async fn stopping_a_session_interrupts_its_turn_cancels_its_queue_and_closes_its_cli() {
    let h = harness_after(|_| {});
    let (bot, [one, two], [a, b], [p1, p2]) = two_working_sessions(&h).await;
    h.rt.state().pane_session_ids.insert(p1.clone(), "cli-one".into());
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: p1.clone(), state: AgentState::Working }).await;
    let queued = h.engine.queue_user_message_to_session(&one.id, "and then C", &[]).unwrap();
    let mut rx = h.engine.subscribe_ui();

    h.engine.stop_session(&one.id).await.unwrap();

    assert_eq!(stored(&h, &a), MessageStatus::Interrupted);
    assert_eq!(stored(&h, &queued), MessageStatus::Cancelled);
    assert_eq!(h.rt.state().stopped, vec![p1.clone()]);
    assert_eq!(session_status(&h, &bot.id, &one).await, BotStatus::Offline);
    assert_eq!(session_status(&h, &bot.id, &two).await, BotStatus::Working, "the other session is unaffected");
    assert_eq!(stored(&h, &b), MessageStatus::Delivered);
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
    let seen = events(&mut rx);
    assert!(
        seen.iter().any(|e| matches!(e, UiEvent::MessageUpdated { message } if message.id == queued.id)),
        "{seen:?}"
    );
    assert!(
        seen.iter().any(|e| matches!(e, UiEvent::SessionStatus { session_id, status: BotStatus::Offline, .. } if *session_id == one.id)),
        "{seen:?}"
    );
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: p2.clone(), state: AgentState::Working }).await;

    let retried = h.engine.retry_message(&a.id).unwrap();
    h.engine.pump_session(&one.id).await;
    let s = h.rt.state();
    let spec = s.starts.last().unwrap();
    assert_eq!(spec.agent, agent(&one));
    assert_eq!(s.starts.len(), 3);
    drop(s);
    assert_eq!(stored(&h, &retried), MessageStatus::Delivered);
}

#[tokio::test]
async fn a_stopped_session_resumes_its_saved_cli_conversation_with_the_next_message() {
    let h = harness_after(|_| {});
    let (bot, [one, _two], _, [p1, p2]) = two_working_sessions(&h).await;
    h.rt.state().pane_session_ids.insert(p1.clone(), "cli-one".into());
    h.engine.handle_bridge(reply(&p1, "done")).await;
    finish_turn(&h, &p1).await;
    h.engine.stop_session(&one.id).await.unwrap();
    assert_eq!(h.rt.state().stopped, vec![p1.clone()]);
    assert_eq!(saved_session_of(&h, &bot.id, &one).as_deref(), Some("cli-one"));

    h.engine.queue_user_message_to_session(&one.id, "again", &[]).unwrap();
    h.engine.pump_session(&one.id).await;
    let s = h.rt.state();
    let spec = s.starts.iter().rfind(|spec| spec.agent == agent(&one)).unwrap();
    assert!(spec.args.ends_with(&["--resume".to_string(), "cli-one".to_string()]), "{:?}", spec.args);
    assert_ne!(s.pane_of(&agent(&one)).unwrap(), p1);
    assert_ne!(s.pane_of(&agent(&one)).unwrap(), p2);
}

fn saved_session_of(h: &Harness, bot_id: &str, session: &Session) -> Option<String> {
    h.engine.sessions_of(bot_id).unwrap().into_iter().find(|s| s.id == session.id).unwrap().provider_session_id
}

#[tokio::test]
async fn stopping_refuses_an_unknown_or_starting_session_and_accepts_a_stopped_one() {
    let h = harness_after(|_| {});
    let err = h.engine.stop_session("missing").await.unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::SessionNotFound(_))), "{err}");

    let bot = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let one = new_session(&h, &bot.id);
    h.engine.stop_session(&one.id).await.unwrap();
    h.engine.stop_session(&one.id).await.unwrap();
    assert!(h.rt.state().stopped.is_empty());
    assert_eq!(session_status(&h, &bot.id, &one).await, BotStatus::Offline);

    h.rt.state().start_delay = Some(Duration::from_secs(5));
    h.engine.queue_user_message_to_session(&one.id, "go", &[]).unwrap();
    let engine = h.engine.clone();
    let id = one.id.clone();
    tokio::time::pause();
    let pump = tokio::spawn(async move { engine.pump_session(&id).await });
    while h.rt.state().starts.is_empty() {
        tokio::task::yield_now().await;
    }
    let err = h.engine.stop_session(&one.id).await.unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::SessionStarting)), "{err}");
    pump.await.unwrap();
}

async fn idle_session(h: &Harness, name: &str) -> (Bot, Session, String) {
    let bot = h.engine.create_bot(claude_bot(name)).unwrap();
    let session = new_session(h, &bot.id);
    h.engine.queue_user_message_to_session(&session.id, "hi", &[]).unwrap();
    h.engine.pump(&bot.id).await;
    let pane = pane(h, &session);
    h.engine.handle_bridge(reply(&pane, "done")).await;
    finish_turn(h, &pane).await;
    (bot, session, pane)
}

#[tokio::test(start_paused = true)]
async fn an_idle_session_stops_after_thirty_minutes_and_its_sibling_goes_on() {
    let h = harness_after(|_| {});
    let (bot, one, p1) = idle_session(&h, "Reviewer").await;
    let two = new_session(&h, &bot.id);
    h.engine.queue_user_message_to_session(&two.id, "work", &[]).unwrap();
    h.engine.pump(&bot.id).await;
    let p2 = pane(&h, &two);
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: p2.clone(), state: AgentState::Working }).await;
    assert_eq!(session_status(&h, &bot.id, &one).await, BotStatus::Idle);
    h.rt.state().pane_states.insert(p2.clone(), AgentState::Working);
    let mut rx = h.engine.subscribe_ui();

    tokio::time::sleep(Duration::from_secs(29 * 60)).await;
    h.engine.watchdog_tick().await;
    assert_eq!(session_status(&h, &bot.id, &one).await, BotStatus::Idle, "29 minutes is not enough");
    assert!(h.rt.state().stopped.is_empty());

    tokio::time::sleep(Duration::from_secs(61)).await;
    h.engine.watchdog_tick().await;
    assert_eq!(h.rt.state().stopped, vec![p1.clone()]);
    assert_eq!(session_status(&h, &bot.id, &one).await, BotStatus::Offline);
    assert_eq!(session_status(&h, &bot.id, &two).await, BotStatus::Working, "a working session is never stopped");
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
    let seen = events(&mut rx);
    assert!(
        seen.iter().any(|e| matches!(e, UiEvent::SessionStatus { session_id, status: BotStatus::Offline, .. } if *session_id == one.id)),
        "{seen:?}"
    );
    assert!(h.engine.user_thread(&bot.id).unwrap().iter().all(|m| m.system_code.is_none()), "no notice");

    h.engine.queue_user_message_to_session(&one.id, "back", &[]).unwrap();
    h.engine.pump_session(&one.id).await;
    assert_ne!(pane(&h, &one), p1);
}

#[tokio::test(start_paused = true)]
async fn the_idle_time_counts_from_the_end_of_the_last_turn_and_waiting_sessions_stay() {
    let h = harness_after(|_| {});
    let (bot, one, p1) = idle_session(&h, "Reviewer").await;
    tokio::time::sleep(Duration::from_secs(25 * 60)).await;
    h.engine.queue_user_message_to_session(&one.id, "more", &[]).unwrap();
    h.engine.pump_session(&one.id).await;
    h.engine.handle_bridge(reply(&p1, "done again")).await;
    finish_turn(&h, &p1).await;
    tokio::time::sleep(Duration::from_secs(10 * 60)).await;
    h.engine.watchdog_tick().await;
    assert_eq!(session_status(&h, &bot.id, &one).await, BotStatus::Idle);

    h.rt.state().state = AgentState::Blocked;
    h.engine.watchdog_tick().await;
    assert_eq!(session_status(&h, &bot.id, &one).await, BotStatus::NeedsAttention);
    tokio::time::sleep(Duration::from_secs(40 * 60)).await;
    h.rt.state().state = AgentState::Blocked;
    h.engine.watchdog_tick().await;
    assert_eq!(session_status(&h, &bot.id, &one).await, BotStatus::NeedsAttention);
    assert!(h.rt.state().stopped.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_session_with_a_queued_message_is_not_stopped_while_idle() {
    let h = harness_after(|_| {});
    let (bot, one, _) = idle_session(&h, "Reviewer").await;
    h.engine.set_paused(true).await.unwrap();
    let queued = h.engine.queue_user_message_to_session(&one.id, "later", &[]).unwrap();
    tokio::time::sleep(Duration::from_secs(31 * 60)).await;
    h.engine.watchdog_tick().await;
    assert_eq!(session_status(&h, &bot.id, &one).await, BotStatus::Idle);
    assert!(h.rt.state().stopped.is_empty());
    assert_eq!(stored(&h, &queued), MessageStatus::Queued);

    h.engine.stop_session(&one.id).await.unwrap();
    assert_eq!(stored(&h, &queued), MessageStatus::Cancelled);
    assert_eq!(session_status(&h, &bot.id, &one).await, BotStatus::Offline);
}

#[tokio::test(start_paused = true)]
async fn a_paused_app_still_stops_its_idle_sessions() {
    let h = harness_after(|_| {});
    let (bot, one, p1) = idle_session(&h, "Reviewer").await;
    h.engine.set_paused(true).await.unwrap();
    tokio::time::sleep(Duration::from_secs(31 * 60)).await;
    h.engine.watchdog_tick().await;
    assert_eq!(h.rt.state().stopped, vec![p1]);
    assert_eq!(session_status(&h, &bot.id, &one).await, BotStatus::Offline);
}

#[tokio::test]
async fn removing_a_session_removes_the_sessions_it_opened_with_their_threads_and_messages() {
    let h = harness_after(|_| {});
    let ([planner, reviewer, qa], [p1, r1, q1], panes) = chain(&h).await;
    assert!(h.engine.handle_bridge(send(&panes[1], "user", "FYI: reviewing.")).await.ok);
    let (r2, r2_first) = h.engine.start_session_with_message(&reviewer.id, "Another review.", &[]).unwrap();
    let store = Store::open(&h.paths.db_path()).unwrap();
    let thread = |a: &str, b: &str| store.thread_for(a, b).unwrap();
    let [user_p1, p1_r1, r1_q1, user_r1] =
        [thread(USER, &p1.id), thread(&p1.id, &r1.id), thread(&r1.id, &q1.id), thread(USER, &r1.id)];
    let all = [&user_p1, &p1_r1, &r1_q1, &user_r1];
    let messages: Vec<String> = all.iter().flat_map(|t| h.engine.thread_messages(t).unwrap()).map(|m| m.id).collect();
    assert_eq!(messages.len(), 4);
    let mut rx = h.engine.subscribe_ui();

    let removed = h.engine.remove_session(&p1.id).await.unwrap();

    let ids: Vec<&str> = removed.sessions.iter().map(|r| r.session.id.as_str()).collect();
    assert_eq!(ids, vec![p1.id.as_str(), r1.id.as_str(), q1.id.as_str()], "the asked-for session first");
    assert_eq!(sorted(removed.thread_ids.clone()), sorted(all.iter().map(|t| t.to_string()).collect()));
    assert_eq!(sorted(removed.messages.iter().map(|m| m.id.clone()).collect()), sorted(messages));
    for t in all {
        assert!(h.engine.thread_messages(t).unwrap().is_empty(), "the thread's messages went with it");
    }
    assert!(h.engine.sessions_of(&planner.id).unwrap().is_empty());
    assert!(h.engine.sessions_of(&qa.id).unwrap().is_empty());
    let left: Vec<String> = h.engine.sessions_of(&reviewer.id).unwrap().into_iter().map(|s| s.id).collect();
    assert_eq!(left, vec![r2.id.clone()], "a session nobody removed stays");
    assert_eq!(h.engine.thread_messages(&r2_first.thread_id).unwrap().len(), 1, "with its chat");
    assert!(h.engine.bot_threads().unwrap().is_empty());
    assert_eq!(sorted(h.rt.state().stopped.clone()), sorted(panes.to_vec()), "each CLI was closed");

    let seen = events(&mut rx);
    let removals: Vec<(String, String, Vec<String>)> = seen
        .iter()
        .filter_map(|e| match e {
            UiEvent::SessionRemoved { session_id, bot_id, thread_ids } => {
                Some((session_id.clone(), bot_id.clone(), sorted(thread_ids.clone())))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        removals,
        vec![
            (p1.id.clone(), planner.id.clone(), sorted(vec![user_p1.clone(), p1_r1.clone()])),
            (r1.id.clone(), reviewer.id.clone(), sorted(vec![p1_r1.clone(), r1_q1.clone(), user_r1.clone()])),
            (q1.id.clone(), qa.id.clone(), vec![r1_q1.clone()]),
        ]
    );
    for bot in [&planner, &reviewer, &qa] {
        assert!(seen.contains(&UiEvent::BotStatus { bot_id: bot.id.clone(), status: BotStatus::Offline }), "{seen:?}");
    }
    assert!(
        !seen.iter().any(|e| matches!(e, UiEvent::SessionStatus { .. })),
        "a removed session has no status: {seen:?}"
    );
    let json = serde_json::to_value(seen.iter().find(|e| matches!(e, UiEvent::SessionRemoved { .. }))).unwrap();
    assert_eq!(json["type"], "session_removed");
    assert_eq!(json["session_id"], p1.id.as_str());

    let err = h.engine.remove_session(&p1.id).await.unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::SessionNotFound(_))), "{err}");
    assert!(h.engine.handle_bridge(reply(&panes[1], "late")).await.error.is_some(), "no session behind the pane");
}

#[tokio::test]
async fn removing_two_running_sessions_of_one_bot_sends_no_status_for_either() {
    let h = harness_after(|_| {});
    let planner = h.engine.create_bot(claude_bot("Planner")).unwrap();
    let writer = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let reviewer = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let (p1, _) = h.engine.start_session_with_message(&planner.id, "Plan the release.", &[]).unwrap();
    h.engine.pump_session(&p1.id).await;
    let p_pane = pane(&h, &p1);
    for to in ["@writer", "@reviewer"] {
        assert!(h.engine.handle_bridge(send(&p_pane, to, "Have a look.")).await.ok);
    }
    settle().await;
    let w1 = opened(&h, &writer, &p1);
    assert!(h.engine.handle_bridge(send(&pane(&h, &w1), "@reviewer", "Check my part.")).await.ok);
    settle().await;
    let (v1, v2) = (opened(&h, &reviewer, &p1), opened(&h, &reviewer, &w1));
    assert_ne!(v1.id, v2.id);
    for v in [&v1, &v2] {
        assert_eq!(session_status(&h, &reviewer.id, v).await, BotStatus::Working);
    }
    let mut rx = h.engine.subscribe_ui();

    h.engine.remove_session(&p1.id).await.unwrap();

    let seen = events(&mut rx);
    let removed: Vec<&str> = seen
        .iter()
        .filter_map(|e| match e {
            UiEvent::SessionRemoved { session_id, .. } => Some(session_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(sorted(removed.iter().map(|s| s.to_string()).collect()), sorted(vec![p1.id, w1.id, v1.id, v2.id]));
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, UiEvent::SessionStatus { session_id, .. } if removed.contains(&session_id.as_str()))),
        "a removed session has no status: {seen:?}"
    );
    let reviewer_statuses: Vec<BotStatus> = seen
        .iter()
        .filter_map(|e| match e {
            UiEvent::BotStatus { bot_id, status } if *bot_id == reviewer.id => Some(*status),
            _ => None,
        })
        .collect();
    assert_eq!(reviewer_statuses, vec![BotStatus::Offline], "{seen:?}");
    assert_eq!(status_of(&h, &reviewer.id).await, BotStatus::Offline);
}

#[tokio::test]
async fn removing_a_session_another_one_opened_keeps_the_one_that_opened_it() {
    let h = harness_after(|_| {});
    let ([planner, reviewer, qa], [p1, r1, q1], [p_pane, r_pane, q_pane]) = chain(&h).await;
    let store = Store::open(&h.paths.db_path()).unwrap();
    let (user_p1, p1_r1) = (store.thread_for(USER, &p1.id).unwrap(), store.thread_for(&p1.id, &r1.id).unwrap());

    let removed = h.engine.remove_session(&r1.id).await.unwrap();

    let ids: Vec<&str> = removed.sessions.iter().map(|r| r.session.id.as_str()).collect();
    assert_eq!(ids, vec![r1.id.as_str(), q1.id.as_str()]);
    assert!(removed.thread_ids.contains(&p1_r1), "{:?}", removed.thread_ids);
    assert_eq!(h.engine.sessions_of(&planner.id).unwrap().len(), 1, "the session that opened it stays");
    assert_eq!(h.engine.thread_messages(&user_p1).unwrap().len(), 1, "with its chat");
    assert!(h.engine.thread_messages(&p1_r1).unwrap().is_empty(), "its thread with the removed session goes");
    assert!(h.engine.sessions_of(&qa.id).unwrap().is_empty());
    assert_eq!(sorted(h.rt.state().stopped.clone()), sorted(vec![r_pane, q_pane]));
    assert_eq!(status_of(&h, &planner.id).await, BotStatus::Working, "P1 goes on with its turn");

    assert!(h.engine.handle_bridge(send(&p_pane, "@reviewer", "Review it again.")).await.ok);
    settle().await;
    let again = opened(&h, &reviewer, &p1);
    assert_ne!(again.id, r1.id);
    assert_eq!(h.engine.sessions_of(&reviewer.id).unwrap().len(), 1);
}

#[tokio::test]
async fn removing_is_refused_while_a_session_it_takes_along_starts_and_for_an_unknown_session() {
    let h = harness_after(|_| {});
    let planner = h.engine.create_bot(claude_bot("Planner")).unwrap();
    let reviewer = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let (p1, _) = h.engine.start_session_with_message(&planner.id, "Plan the release.", &[]).unwrap();
    h.engine.pump_session(&p1.id).await;
    h.rt.state().hang_starts = true;
    assert!(h.engine.handle_bridge(send(&pane(&h, &p1), "@reviewer", "Review the plan.")).await.ok);
    settle().await;
    let r1 = opened(&h, &reviewer, &p1);
    assert_eq!(h.rt.state().starts.len(), 2, "R1 is starting");

    for id in [&p1.id, &r1.id] {
        let err = h.engine.remove_session(id).await.unwrap_err();
        assert!(matches!(err, CoreError::Refused(Refusal::SessionStarting)), "{err}");
    }
    let err = h.engine.restart_session(&r1.id).await.unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::SessionStarting)), "{err}");
    assert!(h.rt.state().stopped.is_empty(), "nothing was stopped");
    assert_eq!(h.engine.sessions_of(&planner.id).unwrap().len(), 1);
    assert_eq!(h.engine.sessions_of(&reviewer.id).unwrap().len(), 1);
    assert_eq!(h.engine.bot_threads().unwrap().len(), 1, "nothing was deleted");

    let err = h.engine.remove_session("missing").await.unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::SessionNotFound(_))), "{err}");
}

#[tokio::test]
async fn removing_a_session_deletes_the_inbox_files_of_its_messages_to_bots_and_nothing_else() {
    let h = harness_after(|_| {});
    let reviewer = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    h.engine.create_bot(claude_bot("Writer")).unwrap();
    let src = tempfile::tempdir().unwrap();
    let file = |path: &str| {
        let path = src.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "text").unwrap();
        path.to_string_lossy().into_owned()
    };
    let (one, to_one) = h
        .engine
        .start_session_with_message(&reviewer.id, "Read these.", &[file("a/notes.md"), file("b/notes.md")])
        .unwrap();
    let (_, to_two) = h.engine.start_session_with_message(&reviewer.id, "And this.", &[file("other.md")]).unwrap();
    h.engine.pump_session(&one.id).await;
    let p1 = pane(&h, &one);
    let folder = h.paths.bot_dir("reviewer");
    let (report, draft) = (folder.join("report.md"), folder.join("draft.md"));
    for own in [&report, &draft] {
        std::fs::write(own, "mine").unwrap();
    }
    assert!(h.engine.handle_bridge(send_files(&p1, "user", "The report.", &[&report])).await.ok);
    assert!(h.engine.handle_bridge(send_files(&p1, "@writer", "A draft.", &[&draft])).await.ok);
    settle().await;
    let to_writer = h.engine.thread_messages(&h.engine.bot_threads().unwrap()[0].id).unwrap().remove(0);
    let path = |m: &Message, i: usize| std::path::PathBuf::from(&m.attachments[i].path);
    let gone = [path(&to_one, 0), path(&to_one, 1), path(&to_writer, 0)];
    assert!(gone[1].file_name().unwrap().to_string_lossy().starts_with(&format!("{}-2-", to_one.id)));
    assert!(path(&to_writer, 0).starts_with(h.paths.bot_dir("writer").join("inbox")));
    for f in &gone {
        assert!(f.is_file(), "{f:?}");
    }

    h.engine.remove_session(&one.id).await.unwrap();

    for f in &gone {
        assert!(!f.exists(), "{f:?} went with its message");
    }
    assert!(path(&to_two, 0).is_file(), "another session's inbox file stays");
    assert!(report.is_file() && draft.is_file(), "files the bot sent from its own folder stay");
}

#[tokio::test]
async fn removing_a_session_leaves_files_alone_when_the_inbox_leads_elsewhere() {
    let h = harness_after(|_| {});
    let reviewer = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let src = tempfile::tempdir().unwrap();
    let notes = src.path().join("notes.md");
    std::fs::write(&notes, "text").unwrap();
    let (one, to_one) = h
        .engine
        .start_session_with_message(&reviewer.id, "Read this.", &[notes.to_string_lossy().into_owned()])
        .unwrap();
    let inbox = h.paths.bot_dir("reviewer").join("inbox");
    let elsewhere = tempfile::tempdir().unwrap();
    let name = std::path::Path::new(&to_one.attachments[0].path).file_name().unwrap().to_owned();
    std::fs::copy(&to_one.attachments[0].path, elsewhere.path().join(&name)).unwrap();
    std::fs::remove_dir_all(&inbox).unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), &inbox).unwrap();

    h.engine.remove_session(&one.id).await.unwrap();

    assert!(elsewhere.path().join(&name).is_file(), "nothing outside the bot's inbox is deleted");
    assert!(h.engine.sessions_of(&reviewer.id).unwrap().is_empty());
}

#[tokio::test]
async fn a_session_answering_a_removed_session_cannot_reach_it_and_its_fallback_is_dropped() {
    let h = harness_after(|_| {});
    let planner = h.engine.create_bot(claude_bot("Planner")).unwrap();
    let reviewer = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let (x, _) = h.engine.start_session_with_message(&planner.id, "Plan the release.", &[]).unwrap();
    h.engine.pump_session(&x.id).await;
    let x_pane = pane(&h, &x);
    assert!(h.engine.handle_bridge(send(&x_pane, "@reviewer", "Review the plan.")).await.ok);
    settle().await;
    let r = opened(&h, &reviewer, &x);
    let r_pane = pane(&h, &r);
    assert!(h.engine.handle_bridge(reply(&r_pane, "Looks good.")).await.ok);
    finish_turn(&h, &r_pane).await;
    assert!(h.engine.handle_bridge(reply(&x_pane, "Asked the reviewer.")).await.ok);
    finish_turn(&h, &x_pane).await;
    finish_turn(&h, &x_pane).await;
    assert!(h.engine.handle_bridge(send(&r_pane, "@planner", "Which release?")).await.ok);
    settle().await;
    let (to, text) = h.rt.state().prompts.last().unwrap().clone();
    assert_eq!((to.as_str(), text.lines().nth(1)), (x_pane.as_str(), Some("Which release?")));

    h.engine.remove_session(&r.id).await.unwrap();

    let notes = h.paths.bot_dir("planner").join("notes.md");
    std::fs::write(&notes, "Spring.").unwrap();
    for request in [reply(&x_pane, "The spring one."), send_files(&x_pane, "@reviewer", "The spring one.", &[&notes])] {
        let error = h.engine.handle_bridge(request).await.error.expect("refused");
        assert_eq!(error.code, ErrorCode::UnknownRecipient);
        assert!(error.message.contains("The conversation you are answering was removed"), "{}", error.message);
    }
    let inbox = h.paths.bot_dir("reviewer").join("inbox");
    assert!(std::fs::read_dir(&inbox).map_or(true, |mut files| files.next().is_none()), "no file was copied");
    h.rt.state().pane_outputs.insert(x_pane.clone(), "The spring one.".into());
    finish_turn(&h, &x_pane).await;

    assert!(h.engine.sessions_of(&reviewer.id).unwrap().is_empty(), "the removed session never comes back");
    assert!(h.engine.bot_threads().unwrap().is_empty(), "nothing was sent to it");
    assert_eq!(status_of(&h, &planner.id).await, BotStatus::Idle);
    let chat = h.engine.user_thread(&planner.id).unwrap();
    let bodies: Vec<&str> = chat.iter().map(|m| m.body.as_str()).collect();
    assert_eq!(bodies, vec!["Plan the release.", "Asked the reviewer."], "no notice, no fallback: {chat:?}");
}

#[tokio::test(start_paused = true)]
async fn a_session_removed_while_its_start_waits_ends_quietly() {
    let h = harness_after(|_| {});
    h.rt.state().start_delay = Some(Duration::from_secs(5));
    let bot = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let [one, two] = [new_session(&h, &bot.id), new_session(&h, &bot.id)];
    h.engine.queue_user_message_to_session(&one.id, "review A", &[]).unwrap();
    h.engine.queue_user_message_to_session(&two.id, "review B", &[]).unwrap();
    let mut rx = h.engine.subscribe_ui();
    let pump = tokio::spawn(h.engine.pump(&bot.id));
    while h.rt.state().starts.is_empty() {
        tokio::task::yield_now().await;
    }
    settle().await;
    let first = h.rt.state().starts[0].agent.clone();
    let (started, waiting) = if first == agent(&one) { (one, two) } else { (two, one) };
    Store::open(&h.paths.db_path()).unwrap().remove_session(&waiting.id, |_| Ok(())).unwrap().unwrap();
    pump.await.unwrap();

    assert_eq!(h.rt.state().starts.len(), 1, "the removed session's CLI never started");
    let seen = events(&mut rx);
    assert!(
        !seen.iter().any(|e| matches!(e, UiEvent::SessionStatus { session_id, status: BotStatus::NeedsAttention, .. } if *session_id == waiting.id)),
        "{seen:?}"
    );
    let p = pane(&h, &started);
    assert!(h.engine.handle_bridge(reply(&p, "done")).await.ok);
    finish_turn(&h, &p).await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Idle);
}
