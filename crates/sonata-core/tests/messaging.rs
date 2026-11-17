#![allow(clippy::await_holding_lock)]
mod support;

use std::sync::Arc;
use std::time::Duration;

use bridge_proto::*;
use sonata_core::model::*;
use sonata_core::store::Store;
use support::*;

fn code(r: BridgeResponse) -> Option<ErrorCode> {
    r.error.map(|e| e.code)
}

fn bot_thread(h: &Harness) -> Vec<Message> {
    let threads = h.engine.bot_threads().unwrap();
    assert_eq!(threads.len(), 1, "{threads:?}");
    h.engine.thread_messages(&threads[0].id).unwrap()
}

#[tokio::test]
async fn send_to_a_bot_continues_the_chain_and_starts_the_recipient() {
    let h = harness();
    let (researcher, writer, hello) = team(&h).await;
    let resp = h.engine.handle_bridge(send("w1:p1", "@writer", "Draft a haiku.")).await;
    assert!(resp.ok, "{resp:?}");
    assert_eq!(resp.to.as_deref(), Some("@writer"));
    settle().await;

    let m = &bot_thread(&h)[0];
    assert_eq!((m.sender.as_str(), m.recipient.as_str()), (researcher.id.as_str(), writer.id.as_str()));
    assert_eq!((m.chain_id.as_str(), m.hop, m.in_reply_to.as_deref()), (hello.chain_id.as_str(), 1, None));
    assert_eq!(m.status, MessageStatus::Delivered, "the writer was started and given the message");
    let s = h.rt.state();
    assert_eq!(s.starts.len(), 2);
    assert_eq!(
        s.prompts[1],
        ("w1:p2".to_string(), "[Message from @researcher · thread you↔@researcher]\nDraft a haiku.".to_string())
    );
}

#[tokio::test]
async fn a_send_returns_while_the_recipient_is_still_starting() {
    let h = harness();
    let (_, writer, _) = team(&h).await;
    h.rt.state().hang_starts = true;
    let resp = tokio::time::timeout(Duration::from_secs(1), h.engine.handle_bridge(send("w1:p1", "@writer", "Draft.")))
        .await
        .expect("sbot send must not wait for the recipient to start");
    assert!(resp.ok, "{resp:?}");
    settle().await;
    assert_eq!(status_of(&h, &writer.id).await, BotStatus::Starting);
    assert_eq!(bot_thread(&h)[0].status, MessageStatus::Queued);
}

#[tokio::test]
async fn a_reply_to_a_bot_goes_back_through_its_queue() {
    let h = harness();
    let (_, writer, _) = team(&h).await;
    h.engine.handle_bridge(send("w1:p1", "@writer", "Draft a haiku.")).await;
    h.engine.handle_bridge(reply("w1:p1", "I asked @writer.")).await;
    finish_turn(&h, "w1:p1").await;

    let resp = h.engine.handle_bridge(reply("w1:p2", "Autumn moon rises")).await;
    assert_eq!(resp.to.as_deref(), Some("@researcher"));
    settle().await;
    let s = h.rt.state();
    assert_eq!(
        s.prompts.last().unwrap(),
        &("w1:p1".to_string(), "[Message from @writer · thread you↔@writer]\nAutumn moon rises".to_string())
    );
    drop(s);
    let thread = bot_thread(&h);
    let answer = thread.iter().find(|m| m.sender == writer.id).unwrap();
    assert_eq!((answer.hop, answer.in_reply_to.as_deref()), (2, Some(thread[0].id.as_str())));
    assert_eq!(answer.status, MessageStatus::Delivered);
}

#[tokio::test]
async fn sending_to_the_user_answers_the_users_message() {
    let h = harness();
    let (researcher, _, hello) = team(&h).await;
    h.rt.state().output = "terminal text".into();
    let resp = h.engine.handle_bridge(send("w1:p1", "User", "Here you go.")).await;
    assert_eq!(resp.to.as_deref(), Some("user"));
    finish_turn(&h, "w1:p1").await;
    let thread = h.engine.user_thread(&researcher.id).unwrap();
    let answer = thread.iter().find(|m| m.body == "Here you go.").unwrap();
    assert_eq!((answer.status, answer.in_reply_to.as_deref()), (MessageStatus::Delivered, Some(hello.id.as_str())));
    assert!(thread.iter().all(|m| m.kind != MessageKind::RawFallback), "no fallback after an answer");
}

#[tokio::test]
async fn bad_recipients_and_empty_bodies_are_refused_and_nothing_is_stored() {
    let h = harness();
    team(&h).await;
    assert_eq!(code(h.engine.handle_bridge(send("w1:p1", "@writr", "x")).await), Some(ErrorCode::UnknownRecipient));
    assert_eq!(code(h.engine.handle_bridge(send("w1:p1", "@Researcher", "x")).await), Some(ErrorCode::SelfSend));
    assert_eq!(code(h.engine.handle_bridge(send("w1:p1", "@writer", " \n")).await), Some(ErrorCode::EmptyBody));
    assert_eq!(code(h.engine.handle_bridge(send("w9:p9", "@writer", "x")).await), Some(ErrorCode::NotABot));
    let unknown = h.engine.handle_bridge(send("w1:p1", "@writr", "x")).await.error.unwrap();
    assert!(unknown.message.contains("sbot bots"), "{}", unknown.message);
    assert!(h.engine.bot_threads().unwrap().is_empty());
}

#[tokio::test]
async fn the_hop_limit_stops_bot_chatter_but_not_messages_to_the_user() {
    let h = harness_after(|store| store.set_setting("hop_limit", "1").unwrap());
    let (_, writer, _) = team(&h).await;
    h.engine.handle_bridge(send("w1:p1", "@writer", "Draft.")).await;
    settle().await;
    for _ in 0..2 {
        let refused = h.engine.handle_bridge(reply("w1:p2", "Done.")).await.error.unwrap();
        assert_eq!(refused.code, ErrorCode::ChainLimitReached);
        assert!(refused.message.contains("sbot send user"), "{}", refused.message);
    }
    assert!(h.engine.handle_bridge(send("w1:p2", "user", "Here is the draft.")).await.ok);

    finish_turn(&h, "w1:p2").await;
    let fallback = bot_thread(&h).into_iter().find(|m| m.kind == MessageKind::RawFallback).expect("a fallback answer");
    assert_eq!(fallback.status, MessageStatus::Failed);
    let prompts = h.rt.state().prompts.len();
    h.engine.handle_bridge(reply("w1:p1", "Done.")).await;
    finish_turn(&h, "w1:p1").await;
    assert_eq!(h.rt.state().prompts.len(), prompts, "the refused fallback never reached the researcher");

    let notices = h
        .engine
        .user_thread(&writer.id)
        .unwrap()
        .into_iter()
        .filter(|m| m.system_code.as_deref() == Some("chain_limit_reached"))
        .count();
    assert_eq!(notices, 1, "one notice per turn, the fallback included");
}

#[tokio::test]
async fn a_send_outside_a_turn_starts_a_new_chain() {
    let h = harness();
    let (_, _, hello) = team(&h).await;
    h.engine.handle_bridge(reply("w1:p1", "Hi!")).await;
    finish_turn(&h, "w1:p1").await;
    assert!(h.engine.handle_bridge(send("w1:p1", "@writer", "Ping.")).await.ok);
    let m = &bot_thread(&h)[0];
    assert_eq!(m.hop, 0);
    assert_ne!(m.chain_id, hello.chain_id);
}

#[tokio::test]
async fn bots_lists_the_teammates_with_their_role_and_status() {
    let h = harness();
    team(&h).await;
    let resp = h.engine.handle_bridge(bots_request("w1:p1")).await;
    assert_eq!(
        resp.bots.unwrap(),
        vec![BotSummary {
            handle: "writer".into(),
            display_name: "Writer".into(),
            provider: "claude".into(),
            status: "offline".into(),
            role: "Writes drafts.".into(),
            waiting_for_user: false,
        }]
    );
}

#[tokio::test]
async fn raw_output_reaches_the_asking_bot_with_a_label() {
    let h = harness();
    team(&h).await;
    h.engine.handle_bridge(send("w1:p1", "@writer", "Draft.")).await;
    h.engine.handle_bridge(reply("w1:p1", "Asked.")).await;
    finish_turn(&h, "w1:p1").await;
    h.rt.state().output = "half a draft".into();
    finish_turn(&h, "w1:p2").await;

    let s = h.rt.state();
    let (pane, text) = s.prompts.last().unwrap();
    assert_eq!(pane, "w1:p1");
    assert!(text.contains("\n(@writer ended its turn without replying with sbot."), "{text}");
    assert!(text.contains("half a draft"), "{text}");
}

#[tokio::test]
async fn a_bot_whose_turn_ends_silently_still_answers_the_bot_that_asked() {
    let h = harness();
    let (_, writer, _) = team(&h).await;
    h.engine.handle_bridge(send("w1:p1", "@writer", "Draft.")).await;
    h.engine.handle_bridge(reply("w1:p1", "Asked.")).await;
    finish_turn(&h, "w1:p1").await;
    h.rt.state().output = String::new();
    finish_turn(&h, "w1:p2").await;

    let s = h.rt.state();
    let (pane, text) = s.prompts.last().unwrap();
    assert_eq!(pane, "w1:p1");
    assert!(
        text.starts_with("[Message from @writer · thread you↔@writer]\n(@writer finished without sending a reply"),
        "{text}"
    );
    drop(s);
    let codes: Vec<String> =
        h.engine.user_thread(&writer.id).unwrap().into_iter().filter_map(|m| m.system_code).collect();
    assert!(codes.contains(&"no_output_captured".to_string()), "{codes:?}");
}

#[tokio::test]
async fn a_fallback_to_a_bot_carries_the_terminal_text() {
    let h = harness();
    team(&h).await;
    h.engine.handle_bridge(send("w1:p1", "@writer", "Draft.")).await;
    h.engine.handle_bridge(reply("w1:p1", "Asked.")).await;
    finish_turn(&h, "w1:p1").await;
    h.rt.state().output = "Here is the draft.\n".into();
    finish_turn(&h, "w1:p2").await;
    let fallback = bot_thread(&h).into_iter().find(|m| m.kind == MessageKind::RawFallback).unwrap();
    assert_eq!((fallback.body.as_str(), fallback.status), ("Here is the draft.", MessageStatus::Delivered));
}

fn limit_notices(h: &Harness, bots: &[&Bot]) -> usize {
    bots.iter()
        .flat_map(|b| h.engine.user_thread(&b.id).unwrap())
        .filter(|m| m.system_code.as_deref() == Some("chain_limit_reached"))
        .count()
}

#[tokio::test]
async fn the_askers_silent_turn_on_an_answer_sends_the_answerer_nothing() {
    let h = harness_after(|_| {});
    let (researcher, writer, _) = team(&h).await;
    h.engine.handle_bridge(send("w1:p1", "@writer", "Draft a haiku.")).await;
    h.engine.handle_bridge(reply("w1:p1", "I asked @writer.")).await;
    finish_turn(&h, "w1:p1").await;
    assert!(h.engine.handle_bridge(reply("w1:p2", "Autumn moon rises")).await.ok);
    finish_turn(&h, "w1:p2").await;
    let to_writer = |h: &Harness| h.rt.state().prompts.iter().filter(|(pane, _)| pane == "w1:p2").count();
    let prompts_to_writer = to_writer(&h);

    h.rt.state().output = "Thanks, noted.".into();
    finish_turn(&h, "w1:p1").await;
    finish_turn(&h, "w1:p2").await;

    let thread = bot_thread(&h);
    let kinds: Vec<(bool, MessageKind)> = thread.iter().map(|m| (m.sender == researcher.id, m.kind)).collect();
    assert_eq!(kinds, vec![(true, MessageKind::Text), (false, MessageKind::Text)], "{thread:?}");
    assert_eq!(to_writer(&h), prompts_to_writer, "nothing reached the writer");
    assert_eq!(limit_notices(&h, &[&researcher, &writer]), 0);

    let next = relaunch(&h);
    next.engine.recover().await.unwrap();
    let answer = next.engine.thread_messages(&thread[1].thread_id).unwrap().remove(1);
    assert_eq!(answer.status, MessageStatus::Delivered);

    h.engine.queue_user_message(&researcher.id, "next").unwrap();
    h.engine.pump(&researcher.id).await;
    let s = h.rt.state();
    let (pane, text) = s.prompts.last().unwrap();
    assert_eq!(pane, "w1:p1");
    assert!(!text.contains("Reminder"), "{text}");
}

#[tokio::test]
async fn two_silent_bots_do_not_bounce_raw_output() {
    let h = harness();
    let (researcher, writer, _) = team(&h).await;
    h.engine.handle_bridge(send("w1:p1", "@writer", "Draft.")).await;
    h.engine.handle_bridge(reply("w1:p1", "Asked.")).await;
    finish_turn(&h, "w1:p1").await;
    h.rt.state().output = "half a draft".into();
    finish_turn(&h, "w1:p2").await;
    finish_turn(&h, "w1:p1").await;
    finish_turn(&h, "w1:p2").await;

    let thread = bot_thread(&h);
    let kinds: Vec<(bool, MessageKind)> = thread.iter().map(|m| (m.sender == researcher.id, m.kind)).collect();
    assert_eq!(kinds, vec![(true, MessageKind::Text), (false, MessageKind::RawFallback)], "{thread:?}");
    assert!(thread.iter().all(|m| m.status == MessageStatus::Delivered), "{thread:?}");
    assert_eq!(limit_notices(&h, &[&researcher, &writer]), 0);
}

async fn planner_with_two_sessions(h: &Harness) -> (Bot, Bot, [Session; 2], [String; 2]) {
    let reviewer = h.engine.create_bot(claude_bot("Reviewer")).unwrap();
    let planner = h.engine.create_bot(claude_bot("Planner")).unwrap();
    let (one, _) = h.engine.start_session_with_message(&planner.id, "Plan A", &[]).unwrap();
    let (two, _) = h.engine.start_session_with_message(&planner.id, "Plan B", &[]).unwrap();
    h.engine.pump(&planner.id).await;
    assert_eq!(latest_session(h, &planner.id), two.id);
    let panes = [pane(h, &one), pane(h, &two)];
    (planner, reviewer, [one, two], panes)
}

fn opened_by(h: &Harness, bot: &Bot, asker: &Session) -> Session {
    let sessions = h.engine.sessions_of(&bot.id).unwrap();
    sessions.into_iter().find(|s| s.opened_by.as_deref() == Some(asker.id.as_str())).expect("a session it opened")
}

fn queue_of(h: &Harness, session: &Session) -> Vec<String> {
    let store = Store::open(&h.paths.db_path()).unwrap();
    store.queued_for_session(&session.id).unwrap().into_iter().map(|m| m.body).collect()
}

fn last_prompt(h: &Harness, pane: &str) -> String {
    h.rt.state().prompts.iter().rev().find(|(p, _)| p == pane).map(|(_, text)| text.clone()).expect("a prompt")
}

#[tokio::test]
async fn each_asking_session_gets_its_own_session_of_the_bot_it_asks() {
    let h = harness_after(|_| {});
    let (_, reviewer, [p1, p2], [pane1, pane2]) = planner_with_two_sessions(&h).await;
    for pane in [&pane1, &pane2] {
        let resp = h.engine.handle_bridge(send(pane, "@reviewer", "Review my plan.")).await;
        assert!(resp.ok, "{resp:?}");
    }
    settle().await;
    let (r1, r2) = (opened_by(&h, &reviewer, &p1), opened_by(&h, &reviewer, &p2));
    assert_ne!(r1.id, r2.id);
    assert_eq!((r1.title.as_deref(), r2.title.as_deref()), (None, None));
    assert_eq!(h.engine.sessions_of(&reviewer.id).unwrap().len(), 2);

    assert!(h.engine.handle_bridge(reply(&pane(&h, &r1), "A is fine.")).await.ok);
    assert!(h.engine.handle_bridge(reply(&pane(&h, &r2), "B is fine.")).await.ok);
    assert_eq!((queue_of(&h, &p1), queue_of(&h, &p2)), (vec!["A is fine.".into()], vec!["B is fine.".into()]));

    assert!(h.engine.handle_bridge(send(&pane1, "@reviewer", "One more thing.")).await.ok);
    assert_eq!(h.engine.sessions_of(&reviewer.id).unwrap().len(), 2);
    assert_eq!(queue_of(&h, &r1), vec!["One more thing.".to_string()]);

    finish_turn(&h, &pane1).await;
    assert_eq!(last_prompt(&h, &pane1).lines().nth(1), Some("A is fine."));
}

#[tokio::test]
async fn a_session_another_bot_opened_writes_to_the_user_in_its_own_chat() {
    let h = harness_after(|_| {});
    let (_, reviewer, [p1, _], [pane1, _]) = planner_with_two_sessions(&h).await;
    h.engine.handle_bridge(send(&pane1, "@reviewer", "Review my plan.")).await;
    settle().await;
    let r1 = opened_by(&h, &reviewer, &p1);

    let resp = h.engine.handle_bridge(send(&pane(&h, &r1), "user", "FYI: the plan has a gap.")).await;
    assert_eq!(resp.to.as_deref(), Some("user"));
    let store = Store::open(&h.paths.db_path()).unwrap();
    let chat = store.thread_messages(&store.thread_for(USER, &r1.id).unwrap()).unwrap();
    let seen: Vec<(&str, Option<&str>, &str)> =
        chat.iter().map(|m| (m.body.as_str(), m.sender_session.as_deref(), m.recipient.as_str())).collect();
    assert_eq!(seen, vec![("FYI: the plan has a gap.", Some(r1.id.as_str()), USER)]);
}

#[tokio::test]
async fn a_send_to_the_session_that_asked_answers_it() {
    let h = harness_after(|_| {});
    let (planner, reviewer, [p1, p2], [pane1, _]) = planner_with_two_sessions(&h).await;
    h.engine.handle_bridge(send(&pane1, "@reviewer", "Review A.")).await;
    settle().await;
    let r1 = opened_by(&h, &reviewer, &p1);
    let r_pane = pane(&h, &r1);
    h.rt.state().pane_outputs.insert(r_pane.clone(), "terminal text".into());
    tokio::time::sleep(Duration::from_millis(5)).await;
    Store::open(&h.paths.db_path()).unwrap().touch_session(&p2.id).unwrap();
    assert_eq!(latest_session(&h, &planner.id), p2.id, "the asking session is not the planner's latest");

    let resp = h.engine.handle_bridge(send(&r_pane, "@planner", "A is fine.")).await;
    assert!(resp.ok, "{resp:?}");
    finish_turn(&h, &r_pane).await;

    assert_eq!((queue_of(&h, &p1), queue_of(&h, &p2)), (vec!["A is fine.".to_string()], vec![]));
    let store = Store::open(&h.paths.db_path()).unwrap();
    let thread = store.thread_messages(&store.thread_for(&p1.id, &r1.id).unwrap()).unwrap();
    let kinds: Vec<(&str, MessageKind)> = thread.iter().map(|m| (m.body.as_str(), m.kind)).collect();
    assert_eq!(kinds, vec![("Review A.", MessageKind::Text), ("A is fine.", MessageKind::Text)], "no fallback");
    assert_eq!(thread[1].in_reply_to.as_deref(), Some(thread[0].id.as_str()));
}

#[tokio::test]
async fn an_asking_session_that_is_not_its_bots_latest_gets_the_reply_and_the_fallback() {
    let h = harness_after(|_| {});
    let (planner, reviewer, [p1, p2], [pane1, _]) = planner_with_two_sessions(&h).await;
    h.engine.handle_bridge(send(&pane1, "@reviewer", "Review A.")).await;
    h.engine.handle_bridge(reply(&pane1, "Asked the reviewer.")).await;
    finish_turn(&h, &pane1).await;
    let r1 = opened_by(&h, &reviewer, &p1);
    let r_pane = pane(&h, &r1);

    tokio::time::sleep(Duration::from_millis(5)).await;
    Store::open(&h.paths.db_path()).unwrap().touch_session(&p2.id).unwrap();
    assert_eq!(latest_session(&h, &planner.id), p2.id);
    assert!(h.engine.handle_bridge(reply(&r_pane, "A is fine.")).await.ok);
    settle().await;
    assert_eq!(last_prompt(&h, &pane1).lines().nth(1), Some("A is fine."));
    finish_turn(&h, &r_pane).await;
    finish_turn(&h, &pane1).await;

    h.engine.handle_bridge(send(&pane1, "@reviewer", "Review A again.")).await;
    settle().await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    Store::open(&h.paths.db_path()).unwrap().touch_session(&p2.id).unwrap();
    h.engine.queue_user_message_to_session(&p2.id, "Plan B, part 2", &[]).unwrap();
    assert_eq!(latest_session(&h, &planner.id), p2.id);
    h.rt.state().pane_outputs.insert(r_pane.clone(), "A, half reviewed".into());
    finish_turn(&h, &r_pane).await;
    let text = last_prompt(&h, &pane1);
    assert!(text.contains("A, half reviewed"), "{text}");
    assert_eq!(queue_of(&h, &p2), vec!["Plan B, part 2".to_string()]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_sends_from_one_session_open_one_session_of_the_recipient() {
    const SENDS: usize = 8;
    let h = harness();
    let (_, writer, _) = team(&h).await;
    let start = Arc::new(tokio::sync::Barrier::new(SENDS));
    let sends: Vec<_> = (0..SENDS)
        .map(|i| {
            let (engine, start) = (h.engine.clone(), start.clone());
            tokio::spawn(async move {
                start.wait().await;
                engine.handle_bridge(send("w1:p1", "@writer", &format!("Part {i}."))).await
            })
        })
        .collect();
    for task in sends {
        let resp = task.await.unwrap();
        assert!(resp.ok, "{resp:?}");
    }

    let sessions = h.engine.sessions_of(&writer.id).unwrap();
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    let thread = bot_thread(&h);
    assert_eq!(thread.len(), SENDS);
    assert!(thread.iter().all(|m| m.recipient_session.as_deref() == Some(sessions[0].id.as_str())));
    tokio::time::timeout(Duration::from_secs(5), async {
        while h.rt.state().prompts.len() < 2 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the writer got its first message");
}
