#![allow(clippy::await_holding_lock)]
mod support;

use bridge_proto::{BotSummary, BridgeResponse, ErrorCode};
use sonata_core::engine::UiEvent;
use sonata_core::model::*;
use sonata_core::store::Store;
use support::*;

fn code(r: &BridgeResponse) -> Option<ErrorCode> {
    r.error.as_ref().map(|e| e.code)
}

fn error_text(r: &BridgeResponse) -> String {
    r.error.as_ref().map(|e| e.message.clone()).unwrap_or_default()
}

async fn now(h: &Harness, session: &Session) -> Session {
    h.engine.find_session_view(&session.id).await.unwrap().session
}

async fn open_request(h: &Harness, session: &Session) -> Option<String> {
    now(h, session).await.open_request_id
}

fn bot_messages(h: &Harness) -> Vec<Message> {
    h.engine.bot_threads().unwrap().iter().flat_map(|t| h.engine.thread_messages(&t.id).unwrap()).collect()
}

fn all_messages(h: &Harness) -> Vec<Message> {
    let store = Store::open(&h.paths.db_path()).unwrap();
    let mut all = bot_messages(h);
    for bot in store.list_bots().unwrap() {
        for session in store.sessions_of(&bot.id).unwrap() {
            all.extend(h.engine.session_messages(&session.id).unwrap());
        }
    }
    all
}

fn message(h: &Harness, id: &str) -> Message {
    all_messages(h).into_iter().find(|m| m.id == id).expect("the message is stored")
}

fn first_between(h: &Harness, a: &Session, b: &Session) -> Message {
    let thread = Store::open(&h.paths.db_path()).unwrap().thread_for(&a.id, &b.id).unwrap();
    h.engine.thread_messages(&thread).unwrap().remove(0)
}

fn last_prompt(h: &Harness, pane: &str) -> String {
    h.rt.state().prompts.iter().rev().find(|(p, _)| p == pane).map(|(_, text)| text.clone()).expect("a prompt")
}

fn waiting_for_user(bots: &[BotSummary], handle: &str) -> bool {
    bots.iter().find(|b| b.handle == handle).expect("listed").waiting_for_user
}

struct Asked {
    bots: [Bot; 3],
    sessions: [Session; 3],
    panes: [String; 3],
    request: Message,
    question: Message,
}

async fn asked(h: &Harness) -> Asked {
    let (bots, sessions, panes) = chain(h).await;
    let request = first_between(h, &sessions[0], &sessions[1]);
    let resp = h.engine.handle_bridge(ask(&panes[1], "Approve the plan?")).await;
    assert!(resp.ok, "{resp:?}");
    let question = message(h, resp.message_id.as_deref().unwrap());
    Asked { bots, sessions, panes, request, question }
}

async fn answered(h: &Harness, a: &Asked) -> Message {
    finish_turn(h, &a.panes[1]).await;
    let answer = h.engine.queue_user_message_to_session(&a.sessions[1].id, "Yes, go.", &[]).unwrap();
    h.engine.pump_session(&a.sessions[1].id).await;
    answer
}

async fn asked_outside_the_tree(h: &Harness) -> (Session, Session, String, Message) {
    let planner = h.engine.create_bot(claude_bot("Planner")).unwrap();
    let reviewer = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let store = Store::open(&h.paths.db_path()).unwrap();
    let p1 = store.create_session(&planner.id, None, None).unwrap();
    let r1 = store.create_session(&reviewer.id, None, None).unwrap();
    let request = store
        .insert_message(&NewMessage {
            sender: planner.id.clone(),
            recipient: reviewer.id.clone(),
            sender_session: Some(p1.id.clone()),
            recipient_session: Some(r1.id.clone()),
            kind: MessageKind::Text,
            body: "Review the plan.".into(),
            attachments: vec![],
            system_code: None,
            system_params: None,
            status: MessageStatus::Queued,
            chain_id: "chain-1".into(),
            hop: 0,
            in_reply_to: None,
        })
        .unwrap();
    h.engine.pump_session(&r1.id).await;
    let r_pane = pane(h, &r1);
    (p1, r1, r_pane, request)
}

#[tokio::test]
async fn ask_on_the_users_turn_answers_it_and_marks_the_session() {
    let h = harness_after(|_| {});
    let (researcher, _, hello) = team(&h).await;
    let mut rx = h.engine.subscribe_ui();
    let resp = h.engine.handle_bridge(ask("w1:p1", "Which date?")).await;
    assert!(resp.ok, "{resp:?}");
    assert_eq!(resp.to.as_deref(), Some("user"));
    let session = h.engine.sessions_of(&researcher.id).unwrap().remove(0);
    let question = h.engine.user_thread(&researcher.id).unwrap().into_iter().find(|m| m.body == "Which date?").unwrap();
    assert_eq!(resp.message_id.as_deref(), Some(question.id.as_str()));
    assert_eq!(session.waiting_question_id.as_deref(), Some(question.id.as_str()));
    assert!(session.waiting_since.is_some());
    assert_eq!(session.open_request_id, None, "the user's message is no bot's request");
    assert_eq!(question.in_reply_to.as_deref(), Some(hello.id.as_str()));
    assert_eq!(question.status, MessageStatus::Delivered);
    assert_eq!(h.engine.answered_by(&hello.id).unwrap(), None, "in_reply_to answers it");
    let seen = events(&mut rx);
    assert!(
        seen.iter().any(|e| matches!(e, UiEvent::SessionUpdated { session }
            if session.session.waiting_question_id.as_deref() == Some(question.id.as_str()))),
        "{seen:?}"
    );
    assert!(seen.iter().any(|e| matches!(e, UiEvent::MessageAdded { message } if message.id == question.id)));
    h.rt.state().output = "Which date works for you?".into();
    finish_turn(&h, "w1:p1").await;
    let thread = h.engine.user_thread(&researcher.id).unwrap();
    assert!(thread.iter().all(|m| m.kind != MessageKind::RawFallback), "{thread:?}");
}

#[tokio::test]
async fn a_question_with_options_marks_the_session_with_them() {
    let h = harness_after(|_| {});
    let (researcher, _, _) = team(&h).await;
    let mut rx = h.engine.subscribe_ui();
    let resp = h.engine.handle_bridge(ask_with("w1:p1", "Plan?", &["Approve", "Change the plan"])).await;
    assert!(resp.ok, "{resp:?}");
    let session = h.engine.sessions_of(&researcher.id).unwrap().remove(0);
    assert_eq!(session.waiting_options, vec!["Approve".to_string(), "Change the plan".to_string()]);
    let seen = events(&mut rx);
    assert!(
        seen.iter().any(|e| matches!(e, UiEvent::SessionUpdated { session }
            if session.session.waiting_options == ["Approve", "Change the plan"])),
        "{seen:?}"
    );
}

#[tokio::test]
async fn the_users_answer_clears_the_options() {
    let h = harness_after(|_| {});
    let (researcher, _, _) = team(&h).await;
    let resp = h.engine.handle_bridge(ask_with("w1:p1", "Plan?", &["Approve", "Change the plan"])).await;
    assert!(resp.ok, "{resp:?}");
    let session = h.engine.sessions_of(&researcher.id).unwrap().remove(0);
    h.engine.queue_user_message_to_session(&session.id, "Approve\n\nuse X", &[]).unwrap();
    let session = now(&h, &session).await;
    assert!(session.waiting_options.is_empty());
    assert_eq!(session.waiting_question_id, None);
}

#[tokio::test]
async fn a_newer_question_without_options_clears_the_old_ones() {
    let h = harness_after(|_| {});
    let (researcher, _, _) = team(&h).await;
    assert!(h.engine.handle_bridge(ask_with("w1:p1", "Plan?", &["Approve"])).await.ok);
    assert!(h.engine.handle_bridge(ask("w1:p1", "Anything else?")).await.ok);
    let session = h.engine.sessions_of(&researcher.id).unwrap().remove(0);
    assert!(session.waiting_options.is_empty());
    assert!(session.waiting_question_id.is_some());
}

#[tokio::test]
async fn options_that_break_a_rule_refuse_the_question() {
    let h = harness_after(|_| {});
    let (researcher, _, _) = team(&h).await;
    let long = "x".repeat(61);
    let cases: Vec<(Vec<&str>, &str)> = vec![
        (vec!["a", "b", "c", "d", "e"], "at most 4"),
        (vec!["  "], "empty"),
        (vec!["a\nb"], "one line"),
        (vec![long.as_str()], "60 characters"),
        (vec!["Approve", "approve"], "twice"),
    ];
    for (options, rule) in cases {
        let resp = h.engine.handle_bridge(ask_with("w1:p1", "Plan?", &options)).await;
        assert!(!resp.ok, "{options:?}");
        assert_eq!(code(&resp), Some(ErrorCode::BadRequest), "{options:?}");
        assert!(error_text(&resp).contains(rule), "{options:?}: {}", error_text(&resp));
        let session = h.engine.sessions_of(&researcher.id).unwrap().remove(0);
        assert_eq!(session.waiting_question_id, None, "{options:?}");
        assert!(session.waiting_options.is_empty());
    }
    let thread = h.engine.user_thread(&researcher.id).unwrap();
    assert!(thread.iter().all(|m| m.body != "Plan?"), "nothing is stored");
}

#[tokio::test]
async fn ask_on_a_bots_request_opens_it_and_tells_the_requesters_chat_without_a_turn() {
    let h = harness_after(|_| {});
    let (_, [p1, r1, _], [_, r_pane, _]) = chain(&h).await;
    let request_to_r1 = first_between(&h, &p1, &r1);
    let prompts = h.rt.state().prompts.len();
    let mut rx = h.engine.subscribe_ui();
    let resp = h.engine.handle_bridge(ask(&r_pane, "Approve the plan?")).await;
    assert!(resp.ok, "{resp:?}");
    assert_eq!(resp.to.as_deref(), Some("user"));
    finish_turn(&h, &r_pane).await;
    let question = message(&h, resp.message_id.as_deref().unwrap());
    assert_eq!((question.recipient.as_str(), question.in_reply_to.as_deref()), (USER, None));
    let r1 = now(&h, &r1).await;
    assert_eq!(r1.open_request_id.as_deref(), Some(request_to_r1.id.as_str()));
    assert_eq!(r1.waiting_question_id.as_deref(), Some(question.id.as_str()));
    assert_eq!(h.rt.state().prompts.len(), prompts, "P1 got no prompt");
    let notice = h
        .engine
        .session_messages(&p1.id)
        .unwrap()
        .into_iter()
        .find(|m| m.system_code.as_deref() == Some("session_waiting"))
        .unwrap();
    assert_eq!(notice.system_params.as_ref().unwrap()["handle"], "reviewer");
    let seen = events(&mut rx);
    assert!(seen.iter().any(|e| matches!(e, UiEvent::MessageAdded { message } if message.id == notice.id)));
    assert!(bot_messages(&h).iter().all(|m| m.kind != MessageKind::RawFallback), "no fallback to the planner");
    assert_eq!(h.engine.answered_by(&request_to_r1.id).unwrap().as_deref(), Some(question.id.as_str()));
}

#[tokio::test]
async fn the_users_answer_clears_the_mark_and_reminds_the_bot_of_the_open_request() {
    let h = harness_after(|_| {});
    let a = asked(&h).await;
    let r1 = &a.sessions[1];
    finish_turn(&h, &a.panes[1]).await;
    h.engine.queue_user_message_to_session(&r1.id, "Yes, go.", &[]).unwrap();
    h.engine.pump_session(&r1.id).await;
    let r1_now = now(&h, r1).await;
    assert_eq!((r1_now.waiting_question_id, r1_now.waiting_since), (None, None));
    assert_eq!(r1_now.open_request_id.as_deref(), Some(a.request.id.as_str()), "the answer leaves it open");
    assert_eq!(
        h.rt.state().prompts.last().unwrap().1,
        "[Message from user · thread you↔user · language: English]\nYes, go.\n\n\
         You are still working on @planner's request. When it is done, send the final result with `sbot result`."
    );
}

#[tokio::test]
async fn an_update_keeps_the_request_open_and_the_result_closes_it_without_fallbacks() {
    let h = harness_after(|_| {});
    let a = asked(&h).await;
    let ([p_pane, r_pane, _], [p1, r1, _]) = (&a.panes, &a.sessions);
    let users_answer = answered(&h, &a).await;
    h.rt.state().output = "Working on it.".into();
    assert!(h.engine.handle_bridge(send(r_pane, "@planner", "Halfway.")).await.ok);
    assert!(open_request(&h, r1).await.is_some(), "an update leaves the request open");
    let resp = h.engine.handle_bridge(result(r_pane, "Done.")).await;
    assert!(resp.ok, "{resp:?}");
    assert_eq!(resp.to.as_deref(), Some("@planner"));
    let done = message(&h, resp.message_id.as_deref().unwrap());
    assert_eq!(done.in_reply_to.as_deref(), Some(a.request.id.as_str()));
    assert_eq!(done.recipient_session.as_deref(), Some(p1.id.as_str()));
    assert_eq!(open_request(&h, r1).await, None);
    assert_eq!(h.engine.answered_by(&users_answer.id).unwrap().as_deref(), Some(done.id.as_str()));
    finish_turn(&h, r_pane).await;
    assert!(h.engine.handle_bridge(reply(p_pane, "The review is under way.")).await.ok);
    finish_turn(&h, p_pane).await;
    assert!(last_prompt(&h, p_pane).contains("Halfway."));
    assert!(h.engine.handle_bridge(reply(p_pane, "Thanks.")).await.ok);
    finish_turn(&h, p_pane).await;
    assert!(last_prompt(&h, p_pane).contains("Done."));
    finish_turn(&h, p_pane).await;
    finish_turn(&h, r_pane).await;
    let all = all_messages(&h);
    assert!(all.iter().all(|m| m.kind != MessageKind::RawFallback), "{all:?}");
}

#[tokio::test]
async fn the_requester_passing_on_the_answer_resolves_the_question_and_the_result_answers_it() {
    let h = harness_after(|_| {});
    let a = asked(&h).await;
    let ([p_pane, r_pane, _], [p1, r1, _]) = (&a.panes, &a.sessions);
    finish_turn(&h, r_pane).await;
    let resp = h.engine.handle_bridge(send(p_pane, "@reviewer", "The user says yes.")).await;
    assert!(resp.ok, "{resp:?}");
    settle().await;
    let passed = message(&h, resp.message_id.as_deref().unwrap());
    assert_eq!(passed.recipient_session.as_deref(), Some(r1.id.as_str()));
    let r1_now = now(&h, r1).await;
    assert_eq!((r1_now.waiting_question_id, r1_now.waiting_since), (None, None));
    let prompt = last_prompt(&h, r_pane);
    assert!(prompt.contains("The user says yes."), "{prompt}");
    assert!(!prompt.contains("sbot result"), "only the user's messages carry the reminder: {prompt}");
    let resp = h.engine.handle_bridge(result(r_pane, "Done.")).await;
    assert!(resp.ok, "{resp:?}");
    let done = message(&h, resp.message_id.as_deref().unwrap());
    assert_eq!(done.in_reply_to.as_deref(), Some(passed.id.as_str()), "it answers the message passed on");
    assert_eq!(done.recipient_session.as_deref(), Some(p1.id.as_str()));
    assert_eq!(open_request(&h, r1).await, None);
    assert_eq!(h.engine.answered_by(&passed.id).unwrap(), None, "in_reply_to answers it");
}

#[tokio::test]
async fn the_requesters_ack_of_a_progress_note_leaves_the_mark_and_its_fresh_message_clears_it() {
    let h = harness_after(|_| {});
    let a = asked(&h).await;
    let ([p_pane, r_pane, _], [p1, r1, _]) = (&a.panes, &a.sessions);
    answered(&h, &a).await;
    assert!(h.engine.handle_bridge(send(r_pane, "@planner", "Halfway.")).await.ok);
    let again = h.engine.handle_bridge(ask(r_pane, "And the dates?")).await.message_id.unwrap();
    finish_turn(&h, r_pane).await;
    assert!(h.engine.handle_bridge(reply(p_pane, "The review is under way.")).await.ok);
    finish_turn(&h, p_pane).await;
    assert!(last_prompt(&h, p_pane).contains("Halfway."));
    let ack = h.engine.handle_bridge(reply(p_pane, "Noted.")).await;
    assert_eq!(message(&h, ack.message_id.as_deref().unwrap()).recipient_session.as_deref(), Some(r1.id.as_str()));
    assert_eq!(now(&h, r1).await.waiting_question_id.as_deref(), Some(again.as_str()), "an ack answers no question");
    finish_turn(&h, p_pane).await;
    h.engine.queue_user_message_to_session(&p1.id, "Tell the reviewer: the 2nd.", &[]).unwrap();
    h.engine.pump_session(&p1.id).await;
    assert!(h.engine.handle_bridge(send(p_pane, "@reviewer", "The user says: the 2nd.")).await.ok);
    assert_eq!(now(&h, r1).await.waiting_question_id, None);
}

#[tokio::test]
async fn a_message_from_a_third_session_or_one_queued_earlier_leaves_the_mark() {
    let h = harness_after(|_| {});
    let (_, [_, r1, _], [_, r_pane, q_pane]) = chain(&h).await;
    h.engine.queue_user_message_to_session(&r1.id, "Also check the dates.", &[]).unwrap();
    let resp = h.engine.handle_bridge(ask(&r_pane, "Approve the plan?")).await;
    assert!(resp.ok, "{resp:?}");
    let question = resp.message_id.unwrap();
    finish_turn(&h, &r_pane).await;
    assert!(last_prompt(&h, &r_pane).contains("Also check the dates."), "delivered after the question");
    assert_eq!(now(&h, &r1).await.waiting_question_id.as_deref(), Some(question.as_str()));
    assert!(h.engine.handle_bridge(send(&q_pane, "@reviewer", "Tests pass.")).await.ok);
    settle().await;
    assert_eq!(now(&h, &r1).await.waiting_question_id.as_deref(), Some(question.as_str()), "Q1 is a third session");
}

#[tokio::test]
async fn a_newer_question_supersedes_the_old_one() {
    let h = harness_after(|_| {});
    let a = asked(&h).await;
    let (r_pane, r1) = (&a.panes[1], &a.sessions[1]);
    let resp = h.engine.handle_bridge(ask(r_pane, "And the dates?")).await;
    assert!(resp.ok, "{resp:?}");
    let second = resp.message_id.unwrap();
    assert_ne!(second, a.question.id);
    assert_eq!(now(&h, r1).await.waiting_question_id.as_deref(), Some(second.as_str()));
    finish_turn(&h, r_pane).await;
    h.engine.queue_user_message_to_session(&r1.id, "Yes, and the 2nd.", &[]).unwrap();
    assert_eq!(now(&h, r1).await.waiting_question_id, None);
}

#[tokio::test]
async fn archiving_clears_the_mark_and_the_open_request() {
    let h = harness_after(|_| {});
    let a = asked(&h).await;
    h.engine.archive_session(&a.sessions[0].id).await.unwrap();
    let r1 = now(&h, &a.sessions[1]).await;
    assert!(r1.archived_at.is_some(), "archived with the session that opened it");
    assert_eq!((r1.waiting_question_id, r1.waiting_since, r1.open_request_id), (None, None, None));
}

#[tokio::test]
async fn an_ask_after_a_clarifying_exchange_with_the_requester_opens_its_request() {
    let h = harness_after(|_| {});
    let (_, [p1, r1, _], [p_pane, r_pane, _]) = chain(&h).await;
    let request = first_between(&h, &p1, &r1);
    h.rt.state().output = "Working on it.".into();
    let question = h.engine.handle_bridge(send(&r_pane, "@planner", "Which release?")).await;
    let question = message(&h, question.message_id.as_deref().unwrap());
    assert_eq!(question.in_reply_to.as_deref(), Some(request.id.as_str()));
    finish_turn(&h, &r_pane).await;
    assert!(h.engine.handle_bridge(reply(&p_pane, "The review is under way.")).await.ok);
    finish_turn(&h, &p_pane).await;
    let answer = h.engine.handle_bridge(reply(&p_pane, "The October one.")).await.message_id.unwrap();
    finish_turn(&h, &p_pane).await;
    assert!(last_prompt(&h, &r_pane).contains("The October one."));

    let resp = h.engine.handle_bridge(ask(&r_pane, "Approve the plan?")).await;
    assert!(resp.ok, "{resp:?}");
    let r1_now = now(&h, &r1).await;
    assert_eq!(r1_now.open_request_id.as_deref(), Some(request.id.as_str()), "the request the exchange began with");
    assert_eq!(r1_now.waiting_question_id, resp.message_id);
    let notices: Vec<_> = h
        .engine
        .session_messages(&p1.id)
        .unwrap()
        .into_iter()
        .filter(|m| m.system_code.as_deref() == Some("session_waiting"))
        .collect();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert_eq!(notices[0].system_params.as_ref().unwrap()["handle"], "reviewer");
    assert_eq!(h.engine.answered_by(&answer).unwrap(), resp.message_id, "the question answers the turn");
    finish_turn(&h, &r_pane).await;

    let users_answer = h.engine.queue_user_message_to_session(&r1.id, "Yes, go.", &[]).unwrap();
    h.engine.pump_session(&r1.id).await;
    assert!(last_prompt(&h, &r_pane).contains("You are still working on @planner's request."));
    let done = h.engine.handle_bridge(result(&r_pane, "Done.")).await;
    assert_eq!(done.to.as_deref(), Some("@planner"), "{done:?}");
    let done = message(&h, done.message_id.as_deref().unwrap());
    assert_eq!(done.in_reply_to.as_deref(), Some(request.id.as_str()));
    assert_eq!(done.recipient_session.as_deref(), Some(p1.id.as_str()));
    assert_eq!(h.engine.answered_by(&users_answer.id).unwrap().as_deref(), Some(done.id.as_str()));
    assert_eq!(open_request(&h, &r1).await, None);
    finish_turn(&h, &r_pane).await;
    let all = all_messages(&h);
    assert!(all.iter().all(|m| m.kind != MessageKind::RawFallback), "{all:?}");
}

#[tokio::test]
async fn an_ask_on_the_answer_to_this_sessions_own_question_opens_no_request() {
    let h = harness_after(|_| {});
    let (researcher, writer, _) = team(&h).await;
    let mine = h.engine.sessions_of(&researcher.id).unwrap().remove(0);
    assert!(h.engine.handle_bridge(send("w1:p1", "@writer", "Which tone?")).await.ok);
    assert!(h.engine.handle_bridge(reply("w1:p1", "Checking with Writer.")).await.ok);
    finish_turn(&h, "w1:p1").await;
    let w1 = opened(&h, &writer, &mine);
    let w_pane = pane(&h, &w1);
    assert!(h.engine.handle_bridge(reply(&w_pane, "Friendly.")).await.ok);
    finish_turn(&h, &w_pane).await;
    assert!(last_prompt(&h, "w1:p1").contains("Friendly."));

    let resp = h.engine.handle_bridge(ask("w1:p1", "Friendly it is?")).await;
    assert!(resp.ok, "{resp:?}");
    let mine = now(&h, &mine).await;
    assert_eq!(mine.open_request_id, None, "Writer made no request");
    assert_eq!(mine.waiting_question_id, resp.message_id);
    let writers_chat = h.engine.session_messages(&w1.id).unwrap();
    assert!(writers_chat.iter().all(|m| m.system_code.as_deref() != Some("session_waiting")), "{writers_chat:?}");
}

#[tokio::test]
async fn result_without_an_open_request_in_a_session_a_bot_opened_points_to_that_bot() {
    let h = harness_after(|_| {});
    let (_, _, [_, r_pane, _]) = chain(&h).await;
    let resp = h.engine.handle_bridge(result(&r_pane, "Done.")).await;
    assert_eq!(code(&resp), Some(ErrorCode::NoOpenRequest), "{resp:?}");
    assert_eq!(
        error_text(&resp),
        "No bot is waiting for a result from this conversation. Send your result to @planner, who opened this \
         conversation, with `sbot send @planner`."
    );
}

#[tokio::test]
async fn result_without_an_open_request_is_refused() {
    let h = harness_after(|_| {});
    let (researcher, _, _) = team(&h).await;
    let resp = h.engine.handle_bridge(result("w1:p1", "Done.")).await;
    assert_eq!(code(&resp), Some(ErrorCode::NoOpenRequest), "{resp:?}");
    assert!(error_text(&resp).contains("sbot reply"), "{resp:?}");
    assert!(h.engine.user_thread(&researcher.id).unwrap().iter().all(|m| m.body != "Done."));
}

#[tokio::test]
async fn a_second_result_is_refused() {
    let h = harness_after(|_| {});
    let a = asked(&h).await;
    answered(&h, &a).await;
    let first = h.engine.handle_bridge(result(&a.panes[1], "Done.")).await;
    assert!(first.ok, "{first:?}");
    let again = h.engine.handle_bridge(result(&a.panes[1], "Done.")).await;
    assert_eq!(code(&again), Some(ErrorCode::NoOpenRequest), "{again:?}");
    assert_eq!(all_messages(&h).iter().filter(|m| m.body == "Done.").count(), 1);
}

#[tokio::test]
async fn a_result_to_an_archived_requester_is_refused_and_keeps_the_request() {
    let h = harness_after(|_| {});
    let (p1, r1, r_pane, request) = asked_outside_the_tree(&h).await;
    assert!(h.engine.handle_bridge(ask(&r_pane, "Approve the plan?")).await.ok);
    assert_eq!(open_request(&h, &r1).await.as_deref(), Some(request.id.as_str()));
    h.engine.archive_session(&p1.id).await.unwrap();
    assert!(now(&h, &r1).await.archived_at.is_none(), "R1 is not in P1's tree");
    let resp = h.engine.handle_bridge(result(&r_pane, "Done.")).await;
    assert_eq!(code(&resp), Some(ErrorCode::UnknownRecipient), "{resp:?}");
    assert!(error_text(&resp).contains("archived"), "{resp:?}");
    assert!(all_messages(&h).iter().all(|m| m.body != "Done."));
    assert_eq!(open_request(&h, &r1).await.as_deref(), Some(request.id.as_str()));
}

#[tokio::test]
async fn an_ask_on_a_request_from_an_archived_session_still_reaches_the_user() {
    let h = harness_after(|_| {});
    let (p1, r1, r_pane, request) = asked_outside_the_tree(&h).await;
    h.engine.archive_session(&p1.id).await.unwrap();
    let resp = h.engine.handle_bridge(ask(&r_pane, "Approve the plan?")).await;
    assert!(resp.ok, "{resp:?}");
    let r1 = now(&h, &r1).await;
    assert_eq!(r1.waiting_question_id, resp.message_id);
    assert_eq!(r1.open_request_id, None, "no session is left to take the result");
    assert_eq!(h.engine.answered_by(&request.id).unwrap(), resp.message_id, "the question answers the turn");
}

#[tokio::test]
async fn a_result_after_the_requester_was_deleted_is_refused() {
    let h = harness_after(|_| {});
    let a = asked(&h).await;
    answered(&h, &a).await;
    h.engine.delete_bot(&a.bots[0].id).await.unwrap();
    let resp = h.engine.handle_bridge(result(&a.panes[1], "Done.")).await;
    assert_eq!(code(&resp), Some(ErrorCode::UnknownRecipient), "{resp:?}");
    assert!(error_text(&resp).contains("no longer exists"), "{resp:?}");
    assert!(all_messages(&h).iter().all(|m| m.body != "Done."));
}

#[tokio::test]
async fn bots_shows_which_sessions_the_caller_opened_wait_for_the_user() {
    let h = harness_after(|_| {});
    let a = asked(&h).await;
    let [p_pane, r_pane, q_pane] = &a.panes;
    let bots = h.engine.handle_bridge(bots_request(p_pane)).await.bots.unwrap();
    assert!(waiting_for_user(&bots, "reviewer"));
    assert!(!waiting_for_user(&bots, "qa"));
    assert!(h.engine.handle_bridge(ask(q_pane, "Which browser?")).await.ok);
    let bots = h.engine.handle_bridge(bots_request(p_pane)).await.bots.unwrap();
    assert!(!waiting_for_user(&bots, "qa"));
    let bots = h.engine.handle_bridge(bots_request(r_pane)).await.bots.unwrap();
    assert!(waiting_for_user(&bots, "qa"));
}

#[tokio::test]
async fn a_result_saved_before_the_turn_end_is_not_cut_off_after_a_restart() {
    let h = harness_after(|_| {});
    let a = asked(&h).await;
    let users_answer = answered(&h, &a).await;
    assert!(h.engine.handle_bridge(result(&a.panes[1], "Done.")).await.ok);
    let cut_off = first_between(&h, &a.sessions[1], &a.sessions[2]);
    let h2 = relaunch(&h);
    h2.engine.recover().await.unwrap();
    assert_eq!(stored(&h2, &users_answer), MessageStatus::Delivered);
    assert_eq!(stored(&h2, &cut_off), MessageStatus::Interrupted, "recovery ran");
}

#[tokio::test]
async fn an_ask_on_a_bots_request_saved_before_the_turn_end_is_not_cut_off_after_a_restart() {
    let h = harness_after(|_| {});
    let a = asked(&h).await;
    let cut_off = first_between(&h, &a.sessions[1], &a.sessions[2]);
    let h2 = relaunch(&h);
    h2.engine.recover().await.unwrap();
    assert_eq!(stored(&h2, &a.request), MessageStatus::Delivered);
    assert_eq!(stored(&h2, &cut_off), MessageStatus::Interrupted, "recovery ran");
}
