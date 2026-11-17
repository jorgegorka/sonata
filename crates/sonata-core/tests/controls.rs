mod support;

use sonata_core::engine::{DialogAnswer, UiEvent};
use sonata_core::model::*;
use sonata_core::runtime::{AgentState, RuntimeEvent};
use std::time::Duration;
use support::*;

fn status_in(h: &Harness, bot_id: &str, body: &str) -> MessageStatus {
    h.engine.user_thread(bot_id).unwrap().into_iter().find(|m| m.body == body).unwrap().status
}

#[tokio::test]
async fn a_queued_message_can_be_cancelled_and_is_never_delivered() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "first").await;
    let second = h.engine.queue_user_message(&bot.id, "second").unwrap();
    let mut ui = h.engine.subscribe_ui();
    assert_eq!(h.engine.cancel_message(&second.id).await.unwrap().status, MessageStatus::Cancelled);
    assert!(matches!(ui.try_recv(), Ok(UiEvent::MessageUpdated { message }) if message.id == second.id));
    h.engine.handle_bridge(reply("w1:p1", "done")).await;
    finish_turn(&h, "w1:p1").await;
    assert_eq!(h.rt.state().prompts.len(), 1, "the cancelled message was not delivered");
}

#[tokio::test]
async fn a_message_being_delivered_cannot_be_cancelled() {
    let h = harness();
    let (_, first) = bot_with_delivered(&h, "first").await;
    let err = h.engine.cancel_message(&first.id).await.unwrap_err();
    assert!(err.to_string().contains("already being delivered"), "{err}");
}

#[tokio::test]
async fn retry_queues_a_copy_of_an_interrupted_message() {
    let h = harness();
    let (bot, first) = bot_with_delivered(&h, "first").await;
    h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: "w1:p1".into() }).await;
    settle().await;
    assert_eq!(status_in(&h, &bot.id, "first"), MessageStatus::Interrupted);
    let copy = h.engine.retry_message(&first.id).unwrap();
    assert_eq!((copy.body.as_str(), copy.status, copy.hop), ("first", MessageStatus::Queued, 0));
    assert_ne!(copy.chain_id, first.chain_id);
    h.engine.pump(&bot.id).await;
    {
        let s = h.rt.state();
        assert_eq!((s.starts.len(), s.prompts.len()), (2, 2));
    }
    assert!(h.engine.retry_message(&copy.id).is_err(), "only interrupted messages can be retried");
}

#[tokio::test]
async fn an_interrupted_message_between_bots_is_reported_and_retried_in_its_chain() {
    let h = harness();
    let (researcher, writer, _) = team(&h).await;
    h.engine.handle_bridge(send("w1:p1", "@writer", "Draft.")).await;
    settle().await;
    let thread = h.engine.bot_threads().unwrap().remove(0);
    let draft = h.engine.thread_messages(&thread.id).unwrap().remove(0);
    h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: "w1:p2".into() }).await;
    settle().await;
    assert_eq!(h.engine.thread_messages(&thread.id).unwrap()[0].status, MessageStatus::Interrupted);
    let notices = h
        .engine
        .user_thread(&researcher.id)
        .unwrap()
        .into_iter()
        .filter(|m| m.system_code.as_deref() == Some("bot_message_interrupted"))
        .count();
    assert_eq!(notices, 1, "the user hears about it in the researcher's chat");

    let copy = h.engine.retry_message(&draft.id).unwrap();
    assert_eq!((copy.sender.as_str(), copy.recipient.as_str()), (researcher.id.as_str(), writer.id.as_str()));
    assert_eq!(
        (copy.chain_id.as_str(), copy.hop, copy.status),
        (draft.chain_id.as_str(), draft.hop, MessageStatus::Queued)
    );
    assert!(draft.sender_session.is_some() && draft.recipient_session.is_some());
    assert_eq!((&copy.sender_session, &copy.recipient_session), (&draft.sender_session, &draft.recipient_session));
    assert_eq!(copy.thread_id, thread.id, "the same two sessions");
    h.engine.pump(&writer.id).await;
    let s = h.rt.state();
    let (pane, text) = s.prompts.last().unwrap();
    assert_eq!(pane, "w1:p3", "the writer started again");
    assert!(text.starts_with("[Message from @researcher"), "{text}");
}

#[tokio::test]
async fn stopping_a_bot_that_another_bot_asked_sends_the_asker_nothing() {
    let h = harness();
    let (researcher, writer, _) = team(&h).await;
    h.engine.handle_bridge(send("w1:p1", "@writer", "Draft.")).await;
    settle().await;
    h.rt.state().output = "half a draft".into();
    h.engine.stop_turn(&latest_session(&h, &writer.id)).await.unwrap();
    finish_turn(&h, "w1:p2").await;
    let thread = h.engine.bot_threads().unwrap().remove(0);
    let messages = h.engine.thread_messages(&thread.id).unwrap();
    assert!(messages.iter().all(|m| m.sender == researcher.id), "nothing went back to the researcher: {messages:?}");
}

#[tokio::test]
async fn stop_sends_escape_to_a_working_bot_only() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "first").await;
    h.engine.stop_turn(&latest_session(&h, &bot.id)).await.unwrap();
    assert_eq!(h.rt.state().keys, vec![("w1:p1".to_string(), vec!["esc".to_string()])]);
    h.engine.handle_bridge(reply("w1:p1", "stopped")).await;
    finish_turn(&h, "w1:p1").await;
    assert!(h.engine.stop_turn(&latest_session(&h, &bot.id)).await.is_err(), "nothing left to stop");
}

#[tokio::test]
async fn dialog_answers_press_enter_or_escape_only_while_the_bot_needs_attention() {
    let h = harness_after(|_| {});
    let (bot, _) = bot_with_delivered(&h, "first").await;
    assert!(h.engine.answer_dialog(&latest_session(&h, &bot.id), DialogAnswer::Cancel).await.is_err());
    assert!(h.rt.state().keys.is_empty());
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: "w1:p1".into(), state: AgentState::Blocked }).await;
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::NeedsAttention);
    h.engine.answer_dialog(&latest_session(&h, &bot.id), DialogAnswer::Continue).await.unwrap();
    h.engine.answer_dialog(&latest_session(&h, &bot.id), DialogAnswer::Cancel).await.unwrap();
    let keys: Vec<Vec<String>> = h.rt.state().keys.iter().map(|(_, k)| k.clone()).collect();
    assert_eq!(keys, vec![vec!["enter".to_string()], vec!["esc".to_string()]]);
    let offline = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let offline = new_session(&h, &offline.id);
    assert!(h.engine.answer_dialog(&offline.id, DialogAnswer::Continue).await.is_err());
}

#[tokio::test]
async fn the_snapshot_shows_a_running_bots_screen() {
    let h = harness_after(|_| {});
    let (bot, _) = bot_with_delivered(&h, "first").await;
    h.rt.state().screen = "Allow this? ❯ Yes".into();
    assert_eq!(h.engine.snapshot(&latest_session(&h, &bot.id)).await.unwrap().as_deref(), Some("Allow this? ❯ Yes"));
    let offline = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let offline = new_session(&h, &offline.id);
    assert_eq!(h.engine.snapshot(&offline.id).await.unwrap(), None);
}

#[tokio::test]
async fn the_terminal_view_reads_the_screen_with_its_colors_while_the_bot_runs() {
    let h = harness_after(|_| {});
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    h.rt.state().ansi_screen = "\u{1b}[32m❯\u{1b}[0m working".into();
    assert_eq!(
        h.engine.terminal(&latest_session(&h, &bot.id)).await.unwrap().as_deref(),
        Some("\u{1b}[32m❯\u{1b}[0m working")
    );
    let offline = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let offline = new_session(&h, &offline.id);
    assert_eq!(h.engine.terminal(&offline.id).await.unwrap(), None);
}

#[tokio::test]
async fn restart_interrupts_the_turn_and_resumes_the_session() {
    let h = harness();
    h.rt.state().session_id = Some("sess-1".into());
    let (bot, _) = bot_with_delivered(&h, "first").await;
    h.engine.queue_user_message(&bot.id, "second").unwrap();
    h.engine.restart_session(&latest_session(&h, &bot.id)).await.unwrap();
    settle().await;
    assert_eq!(status_in(&h, &bot.id, "first"), MessageStatus::Interrupted);
    {
        let s = h.rt.state();
        assert_eq!(s.stopped, vec!["w1:p1".to_string()]);
        assert_eq!(s.starts.len(), 2);
        assert!(s.starts[1].args.ends_with(&["--resume".to_string(), "sess-1".to_string()]));
        assert_eq!(s.prompts.last().unwrap().0, "w1:p2");
    }
    assert_eq!(status_in(&h, &bot.id, "second"), MessageStatus::Delivered);
}

#[tokio::test]
async fn pause_holds_deliveries_and_starts_until_resume() {
    let h = harness();
    let (bot, _) = bot_with_delivered(&h, "first").await;
    let writer = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let mut ui = h.engine.subscribe_ui();
    h.engine.set_paused(true).await.unwrap();
    assert!(h.engine.paused().unwrap());
    assert!(matches!(ui.try_recv(), Ok(UiEvent::Paused { paused: true })));
    h.engine.queue_user_message(&bot.id, "second").unwrap();
    h.engine.queue_user_message(&writer.id, "for the writer").unwrap();
    h.engine.pump(&bot.id).await;
    h.engine.pump(&writer.id).await;
    assert!(h.engine.handle_bridge(reply("w1:p1", "first answer")).await.ok);
    finish_turn(&h, "w1:p1").await;
    assert_eq!(status_in(&h, &bot.id, "first answer"), MessageStatus::Delivered);
    {
        let s = h.rt.state();
        assert_eq!((s.starts.len(), s.prompts.len()), (1, 1), "nothing delivered or started while paused");
    }
    assert_eq!(status_in(&h, &bot.id, "second"), MessageStatus::Queued);

    h.engine.set_paused(false).await.unwrap();
    settle().await;
    let s = h.rt.state();
    assert_eq!(s.starts.len(), 2, "the writer started");
    let mut delivered: Vec<&str> = s.prompts.iter().map(|(_, text)| text.lines().nth(1).unwrap()).collect();
    assert_eq!(delivered.remove(0), "first");
    delivered.sort();
    assert_eq!(delivered, vec!["for the writer", "second"]);
}

#[tokio::test(start_paused = true)]
async fn pausing_while_a_bot_starts_holds_its_first_delivery() {
    let h = harness();
    h.rt.state().start_delay = Some(Duration::from_secs(5));
    let bot = h.engine.create_bot(claude_bot("Researcher")).unwrap();
    h.engine.queue_user_message(&bot.id, "first").unwrap();
    let pump = tokio::spawn(h.engine.pump(&bot.id));
    while h.rt.state().starts.is_empty() {
        tokio::task::yield_now().await;
    }
    h.engine.set_paused(true).await.unwrap();
    pump.await.unwrap();
    assert_eq!(h.rt.state().starts.len(), 1, "the start that had begun finishes");
    assert!(h.rt.state().prompts.is_empty(), "but nothing is delivered while paused");
    assert_eq!(status_in(&h, &bot.id, "first"), MessageStatus::Queued);
}

#[tokio::test]
async fn a_paused_app_stays_paused_after_a_relaunch() {
    let h = harness_after(|store| store.set_setting("paused", "true").unwrap());
    assert!(h.engine.paused().unwrap());
    let bot = h.engine.create_bot(claude_bot("Researcher")).unwrap();
    h.engine.queue_user_message(&bot.id, "hi").unwrap();
    h.engine.recover().await.unwrap();
    assert!(h.rt.state().starts.is_empty());
}

#[tokio::test]
async fn a_message_between_bots_cut_off_by_quitting_is_reported_on_the_next_launch() {
    let h = harness_after(|_| {});
    let (researcher, _writer, _) = team(&h).await;
    h.engine.handle_bridge(send("w1:p1", "@writer", "Draft.")).await;
    settle().await;
    let thread = h.engine.bot_threads().unwrap().remove(0);
    let interrupted_notices = |h: &Harness| {
        h.engine
            .user_thread(&researcher.id)
            .unwrap()
            .into_iter()
            .filter(|m| m.system_code.as_deref() == Some("bot_message_interrupted"))
            .count()
    };
    h.engine.shutdown().await;
    h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: "w1:p2".into() }).await;
    settle().await;
    assert_eq!(h.engine.thread_messages(&thread.id).unwrap()[0].status, MessageStatus::Delivered);
    assert_eq!(interrupted_notices(&h), 0);

    let next = relaunch(&h);
    next.engine.recover().await.unwrap();
    assert_eq!(next.engine.thread_messages(&thread.id).unwrap()[0].status, MessageStatus::Interrupted);
    assert_eq!(interrupted_notices(&next), 1);
}

#[tokio::test]
async fn a_stopped_turn_for_a_bot_is_not_reported_as_interrupted_after_a_relaunch() {
    let h = harness_after(|_| {});
    let (researcher, writer, _) = team(&h).await;
    h.engine.handle_bridge(send("w1:p1", "@writer", "Draft.")).await;
    settle().await;
    h.engine.stop_turn(&latest_session(&h, &writer.id)).await.unwrap();
    finish_turn(&h, "w1:p2").await;
    let thread = h.engine.bot_threads().unwrap().remove(0);

    let next = relaunch(&h);
    next.engine.recover().await.unwrap();
    assert_eq!(next.engine.thread_messages(&thread.id).unwrap()[0].status, MessageStatus::Delivered);
    let notices = next
        .engine
        .user_thread(&researcher.id)
        .unwrap()
        .into_iter()
        .filter(|m| m.system_code.as_deref() == Some("bot_message_interrupted"))
        .count();
    assert_eq!(notices, 0, "the user stopped that turn; it was not cut off");
}

#[tokio::test]
async fn an_interrupted_message_is_retried_only_once() {
    let h = harness();
    let (bot, first) = bot_with_delivered(&h, "first").await;
    h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: "w1:p1".into() }).await;
    settle().await;
    let mut ui = h.engine.subscribe_ui();
    let copy = h.engine.retry_message(&first.id).unwrap();
    let err = h.engine.retry_message(&first.id).unwrap_err();
    assert!(err.to_string().contains("only an interrupted message can be retried"), "{err}");

    let firsts: Vec<Message> =
        h.engine.user_thread(&bot.id).unwrap().into_iter().filter(|m| m.body == "first").collect();
    let statuses: Vec<(&str, MessageStatus)> = firsts.iter().map(|m| (m.id.as_str(), m.status)).collect();
    assert_eq!(
        statuses,
        vec![(first.id.as_str(), MessageStatus::Retried), (copy.id.as_str(), MessageStatus::Queued)],
        "one copy, and the original shows it was retried"
    );
    let updated = std::iter::from_fn(|| ui.try_recv().ok()).find_map(|e| match e {
        UiEvent::MessageUpdated { message } if message.id == first.id => Some(message.status),
        _ => None,
    });
    assert_eq!(updated, Some(MessageStatus::Retried));
}

#[tokio::test]
async fn a_stop_whose_escape_never_arrived_does_not_end_the_chain() {
    let h = harness();
    let (researcher, writer, _) = team(&h).await;
    h.engine.handle_bridge(send("w1:p1", "@writer", "Draft.")).await;
    settle().await;
    h.rt.state().key_failures.push_back(sonata_core::runtime::RuntimeError::Other("herdr did not answer".into()));
    assert!(h.engine.stop_turn(&latest_session(&h, &writer.id)).await.is_err());
    h.rt.state().output = "the draft".into();
    finish_turn(&h, "w1:p2").await;
    let thread = h.engine.bot_threads().unwrap().remove(0);
    let answer = h.engine.thread_messages(&thread.id).unwrap().into_iter().find(|m| m.sender == writer.id);
    let answer = answer.expect("the researcher got the fallback answer");
    assert_eq!((answer.kind, answer.recipient.as_str()), (MessageKind::RawFallback, researcher.id.as_str()));
}
