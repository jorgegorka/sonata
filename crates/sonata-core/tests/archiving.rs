#![allow(clippy::await_holding_lock)]
mod support;

use std::time::Duration;

use bridge_proto::ErrorCode;
use sonata_core::engine::UiEvent;
use sonata_core::model::*;
use sonata_core::store::Store;
use sonata_core::{CoreError, Refusal};
use support::*;

fn now(h: &Harness, session: &Session) -> Session {
    h.engine.sessions_of(&session.bot_id).unwrap().into_iter().find(|s| s.id == session.id).unwrap()
}

fn first(h: &Harness, a: &str, b: &str) -> Message {
    let thread = Store::open(&h.paths.db_path()).unwrap().thread_for(a, b).unwrap();
    h.engine.thread_messages(&thread).unwrap().remove(0)
}

fn archived(err: &CoreError) -> bool {
    matches!(err, CoreError::Refused(Refusal::SessionArchived(_)))
}

fn updates(seen: &[UiEvent]) -> Vec<(String, bool, BotStatus)> {
    seen.iter()
        .filter_map(|e| match e {
            UiEvent::SessionUpdated { session } => {
                Some((session.session.id.clone(), session.session.archived_at.is_some(), session.status))
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn archiving_a_session_stops_it_and_the_sessions_it_opened_and_keeps_their_conversations() {
    let h = harness_after(|_| {});
    let ([planner, reviewer, qa], [p1, r1, q1], panes) = chain(&h).await;
    let more = h.engine.queue_user_message_to_session(&p1.id, "And the notes.", &[]).unwrap();
    let turns = [first(&h, USER, &p1.id), first(&h, &p1.id, &r1.id), first(&h, &r1.id, &q1.id)];
    let cli_ids: Vec<Option<String>> = [&p1, &r1, &q1].map(|s| now(&h, s).provider_session_id).to_vec();
    assert!(cli_ids.iter().all(Option::is_some), "each CLI conversation was saved when it started");
    let chat = h.engine.session_messages(&p1.id).unwrap().len();
    let mut rx = h.engine.subscribe_ui();

    let archived = h.engine.archive_session(&p1.id).await.unwrap();

    let ids: Vec<&str> = archived.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec![p1.id.as_str(), r1.id.as_str(), q1.id.as_str()], "the asked-for session first");
    assert_eq!(sorted(h.rt.state().stopped.clone()), sorted(panes.to_vec()), "each CLI was closed");
    for turn in &turns {
        assert_eq!(stored(&h, turn), MessageStatus::Interrupted, "{}", turn.body);
    }
    assert_eq!(stored(&h, &more), MessageStatus::Cancelled);
    for (session, cli_id) in [&p1, &r1, &q1].into_iter().zip(&cli_ids) {
        let session = now(&h, session);
        assert!(session.archived_at.is_some());
        assert_eq!(&session.provider_session_id, cli_id, "kept, so a restore resumes it");
    }
    assert_eq!(h.engine.bot_threads().unwrap().len(), 2, "their conversations are kept");
    assert_eq!(h.engine.session_messages(&p1.id).unwrap().len(), chat, "with no notice in an archived chat");
    for bot in [&planner, &reviewer, &qa] {
        assert_eq!(status_of(&h, &bot.id).await, BotStatus::Offline);
    }

    let seen = events(&mut rx);
    assert_eq!(updates(&seen), [&p1, &r1, &q1].map(|s| (s.id.clone(), true, BotStatus::Offline)).to_vec(), "{seen:?}");
    for s in [&p1, &r1, &q1] {
        assert!(
            seen.iter().any(|e| matches!(e, UiEvent::SessionStatus { session_id, status: BotStatus::Offline, .. } if *session_id == s.id)),
            "{seen:?}"
        );
    }
    assert!(!seen.iter().any(|e| matches!(e, UiEvent::SessionRemoved { .. })), "{seen:?}");
    assert!(h.engine.archive_session(&p1.id).await.unwrap().is_empty(), "archived already");
}

#[tokio::test]
async fn an_archived_session_refuses_the_users_messages_its_restart_and_retry() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(claude_bot("Planner")).unwrap();
    let (x, plan) = h.engine.start_session_with_message(&bot.id, "Plan the release.", &[]).unwrap();
    h.engine.pump_session(&x.id).await;
    h.engine.archive_session(&x.id).await.unwrap();
    assert_eq!(stored(&h, &plan), MessageStatus::Interrupted);

    let tmp = tempfile::tempdir().unwrap();
    let notes = tmp.path().join("notes.md");
    std::fs::write(&notes, "Spring.").unwrap();
    let files = [notes.to_string_lossy().into_owned()];
    let err = h.engine.queue_user_message_to_session(&x.id, "And the notes.", &files).unwrap_err();
    assert!(archived(&err), "{err}");
    let inbox = h.paths.bot_dir("planner").join("inbox");
    assert!(std::fs::read_dir(&inbox).map_or(true, |mut files| files.next().is_none()), "no file was copied");
    let err = h.engine.restart_session(&x.id).await.unwrap_err();
    assert!(archived(&err), "{err}");
    let err = h.engine.retry_message(&plan.id).unwrap_err();
    assert!(archived(&err), "{err}");
    assert_eq!(stored(&h, &plan), MessageStatus::Interrupted, "still there to retry after a restore");
    settle().await;
    assert_eq!(h.rt.state().starts.len(), 1, "nothing started it again");
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Offline);
}

#[tokio::test]
async fn archiving_is_refused_while_a_session_it_takes_along_starts_and_for_an_unknown_session() {
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
        let err = h.engine.archive_session(id).await.unwrap_err();
        assert!(matches!(err, CoreError::Refused(Refusal::SessionStarting)), "{err}");
    }
    assert!(h.rt.state().stopped.is_empty(), "nothing was stopped");
    assert_eq!((now(&h, &p1).archived_at, now(&h, &r1).archived_at), (None, None), "nothing was archived");

    let err = h.engine.archive_session("missing").await.unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::SessionNotFound(_))), "{err}");
}

#[tokio::test(start_paused = true)]
async fn a_session_archived_while_its_start_waits_ends_quietly() {
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
    Store::open(&h.paths.db_path()).unwrap().archive_session(&waiting.id, |_| Ok(())).unwrap().unwrap();
    pump.await.unwrap();

    assert_eq!(h.rt.state().starts.len(), 1, "the archived session's CLI never started");
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

#[tokio::test]
async fn a_session_answering_an_archived_session_cannot_reach_it_and_its_fallback_is_dropped() {
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
    let between = h.engine.bot_threads().unwrap();
    let said = h.engine.thread_messages(&between[0].id).unwrap().len();

    h.engine.archive_session(&r.id).await.unwrap();

    let notes = h.paths.bot_dir("planner").join("notes.md");
    std::fs::write(&notes, "Spring.").unwrap();
    for request in [reply(&x_pane, "The spring one."), send_files(&x_pane, "@reviewer", "The spring one.", &[&notes])] {
        let error = h.engine.handle_bridge(request).await.error.expect("refused");
        assert_eq!(error.code, ErrorCode::UnknownRecipient);
        assert!(error.message.contains("The conversation you are answering was archived"), "{}", error.message);
    }
    let inbox = h.paths.bot_dir("reviewer").join("inbox");
    assert!(std::fs::read_dir(&inbox).map_or(true, |mut files| files.next().is_none()), "no file was copied");
    h.rt.state().pane_outputs.insert(x_pane.clone(), "The spring one.".into());
    finish_turn(&h, &x_pane).await;

    assert_eq!(h.engine.sessions_of(&reviewer.id).unwrap().len(), 1, "no new session for the answer");
    assert_eq!(h.engine.thread_messages(&between[0].id).unwrap().len(), said, "nothing reached R");
    assert!(now(&h, &r).archived_at.is_some());
    assert_eq!(status_of(&h, &planner.id).await, BotStatus::Idle);
    let chat = h.engine.session_messages(&x.id).unwrap();
    let bodies: Vec<&str> = chat.iter().map(|m| m.body.as_str()).collect();
    assert_eq!(bodies, vec!["Plan the release.", "Asked the reviewer."], "no notice, no fallback: {chat:?}");
}

#[tokio::test]
async fn a_send_after_the_shared_session_was_archived_opens_a_new_one() {
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

    h.engine.archive_session(&r.id).await.unwrap();
    assert!(h.engine.handle_bridge(send(&x_pane, "@reviewer", "One more check.")).await.ok);
    settle().await;

    let sessions = h.engine.sessions_of(&reviewer.id).unwrap();
    assert_eq!(sessions.len(), 2);
    let fresh = sessions.iter().find(|s| s.id != r.id).unwrap();
    assert_eq!((fresh.opened_by.as_deref(), fresh.archived_at.as_deref()), (Some(x.id.as_str()), None));
    let (to, text) = h.rt.state().prompts.last().unwrap().clone();
    assert_eq!((to, text.lines().nth(1)), (pane(&h, fresh), Some("One more check.")));
    assert!(now(&h, &r).archived_at.is_some(), "the archived one stays archived");
}

#[tokio::test]
async fn messages_the_archived_tree_sent_out_stay_queued() {
    let h = harness_after(|_| {});
    let planner = h.engine.create_bot(claude_bot("Planner")).unwrap();
    let reviewer = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let (x, _) = h.engine.start_session_with_message(&planner.id, "Plan the release.", &[]).unwrap();
    h.engine.pump_session(&x.id).await;
    let x_pane = pane(&h, &x);
    assert!(h.engine.handle_bridge(send(&x_pane, "@reviewer", "Review the plan.")).await.ok);
    settle().await;
    let r = opened(&h, &reviewer, &x);
    assert!(h.engine.handle_bridge(reply(&pane(&h, &r), "Looks good.")).await.ok);
    let answer = h.engine.thread_messages(&first(&h, &x.id, &r.id).thread_id).unwrap().pop().unwrap();
    assert_eq!((answer.body.as_str(), answer.status), ("Looks good.", MessageStatus::Queued));

    h.engine.archive_session(&r.id).await.unwrap();
    assert_eq!(stored(&h, &answer), MessageStatus::Queued, "the helper's answer is not lost");
    finish_turn(&h, &x_pane).await;

    assert_eq!(stored(&h, &answer), MessageStatus::Delivered);
    let (to, text) = h.rt.state().prompts.last().unwrap().clone();
    assert_eq!((to.as_str(), text.contains("Looks good.")), (x_pane.as_str(), true));
}

#[tokio::test]
async fn restoring_puts_the_tree_back_stopped_and_the_next_message_resumes_its_cli_conversation() {
    let h = harness_after(|_| {});
    let ([planner, ..], [p1, r1, q1], _) = chain(&h).await;
    let cli_id = now(&h, &p1).provider_session_id.unwrap();
    h.engine.archive_session(&p1.id).await.unwrap();
    let starts = h.rt.state().starts.len();
    let mut rx = h.engine.subscribe_ui();

    let restored = h.engine.restore_session(&p1.id).await.unwrap();

    let ids: Vec<&str> = restored.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec![p1.id.as_str(), r1.id.as_str(), q1.id.as_str()]);
    for s in [&p1, &r1, &q1] {
        assert_eq!(now(&h, s).archived_at, None);
    }
    assert_eq!(updates(&events(&mut rx)), [&p1, &r1, &q1].map(|s| (s.id.clone(), false, BotStatus::Offline)).to_vec());
    settle().await;
    assert_eq!(h.rt.state().starts.len(), starts, "restored, stopped");
    assert_eq!(status_of(&h, &planner.id).await, BotStatus::Offline);

    h.engine.queue_user_message_to_session(&p1.id, "Pick it up again.", &[]).unwrap();
    h.engine.pump_session(&p1.id).await;
    let s = h.rt.state();
    let spec = s.starts.last().unwrap();
    assert_eq!(spec.agent, agent(&p1));
    assert!(spec.args.ends_with(&["--resume".to_string(), cli_id]), "{:?}", spec.args);
    drop(s);

    assert!(h.engine.restore_session(&p1.id).await.unwrap().is_empty(), "not archived");
    let err = h.engine.restore_session("missing").await.unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::SessionNotFound(_))), "{err}");
}

#[tokio::test]
async fn a_relaunch_leaves_archived_sessions_alone() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let [one, two] = [new_session(&h, &bot.id), new_session(&h, &bot.id)];
    h.engine.queue_user_message_to_session(&one.id, "review A", &[]).unwrap();
    h.engine.queue_user_message_to_session(&two.id, "review B", &[]).unwrap();
    Store::open(&h.paths.db_path()).unwrap().archive_session(&one.id, |_| Ok(())).unwrap().unwrap();

    let next = relaunch(&h);
    next.engine.recover().await.unwrap();
    settle().await;
    next.engine.pump_session(&one.id).await;

    let s = next.rt.state();
    let agents: Vec<&str> = s.starts.iter().map(|spec| spec.agent.as_str()).collect();
    assert_eq!(agents, vec![agent(&two).as_str()], "the archived session never starts");
}

#[tokio::test]
async fn deleting_a_bot_removes_its_archived_sessions_too() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(claude_bot("Planner")).unwrap();
    let (x, _) = h.engine.start_session_with_message(&bot.id, "Plan the release.", &[]).unwrap();
    h.engine.pump_session(&x.id).await;
    h.engine.archive_session(&x.id).await.unwrap();
    let mut rx = h.engine.subscribe_ui();

    h.engine.delete_bot(&bot.id).await.unwrap();

    assert!(h.engine.sessions_of(&bot.id).unwrap().is_empty());
    let seen = events(&mut rx);
    assert!(
        seen.iter().any(|e| matches!(e, UiEvent::SessionRemoved { session_id, .. } if *session_id == x.id)),
        "{seen:?}"
    );
}

#[tokio::test]
async fn renaming_a_session_cuts_it_like_a_title_and_a_blank_name_clears_it() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(claude_bot("Planner")).unwrap();
    let (x, _) = h.engine.start_session_with_message(&bot.id, "Plan the release.", &[]).unwrap();
    let mut rx = h.engine.subscribe_ui();
    let long = "a".repeat(70);
    let cut = format!("{}…", "a".repeat(59));

    let view = h.engine.rename_session(&x.id, "  Release 2.0\nwith notes").await.unwrap();
    assert_eq!((view.session.title.as_deref(), view.status), (Some("Release 2.0"), BotStatus::Offline));
    assert_eq!(now(&h, &x).title.as_deref(), Some("Release 2.0"));
    assert_eq!(h.engine.rename_session(&x.id, &long).await.unwrap().session.title, Some(cut.clone()));
    let view = h.engine.rename_session(&x.id, " \n ").await.unwrap();
    assert_eq!(view.session.title, None, "a blank name clears it: the UI shows Session 1 again");

    let titles: Vec<Option<String>> = events(&mut rx)
        .into_iter()
        .filter_map(|e| match e {
            UiEvent::SessionUpdated { session } => Some(session.session.title),
            _ => None,
        })
        .collect();
    assert_eq!(titles, vec![Some("Release 2.0".to_string()), Some(cut), None]);

    h.engine.archive_session(&x.id).await.unwrap();
    let view = h.engine.rename_session(&x.id, "Old plan").await.unwrap();
    assert_eq!(view.session.title.as_deref(), Some("Old plan"), "an archived session can be renamed");
    assert!(view.session.archived_at.is_some(), "and stays archived");
    let err = h.engine.rename_session("missing", "Plan").await.unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::SessionNotFound(_))), "{err}");
}
