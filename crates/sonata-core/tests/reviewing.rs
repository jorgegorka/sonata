mod support;

use bridge_proto::{BridgeCommand, BridgeResponse, ErrorCode};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use serde_json::json;
use sonata_core::model::*;
use sonata_core::schedule::local_minute;
use sonata_core::store::Store;
use support::*;

fn minute(stored: &str) -> String {
    local_minute(DateTime::parse_from_rfc3339(stored).unwrap().with_timezone(&Utc), &Local)
}

async fn bot_named(h: &Harness, handle: &str) -> Bot {
    h.engine.list_bots().await.unwrap().into_iter().find(|v| v.bot.handle == handle).unwrap().bot
}

async fn masters_turn(h: &Harness) -> (Session, String) {
    let checkout = tempfile::tempdir().unwrap();
    setup_team(h, Some(checkout.path()));
    let master = bot_named(h, "master").await;
    let (session, _) = h.engine.start_session_with_message(&master.id, "Review the week.", &[]).unwrap();
    h.engine.pump_session(&session.id).await;
    let pane = pane(h, &session);
    (session, pane)
}

async fn user_session(h: &Harness, handle: &str, body: &str) -> (Session, String) {
    let bot = bot_named(h, handle).await;
    let (session, _) = h.engine.start_session_with_message(&bot.id, body, &[]).unwrap();
    h.engine.pump_session(&session.id).await;
    let pane = pane(h, &session);
    (session, pane)
}

async fn run(h: &Harness, pane: &str, command: BridgeCommand) -> BridgeResponse {
    h.engine.handle_bridge(request(pane, command)).await
}

fn output(response: BridgeResponse) -> String {
    response.output.unwrap_or_else(|| panic!("no output: {:?}", response.error))
}

fn archive(references: &[&str]) -> BridgeCommand {
    BridgeCommand::SessionArchive { sessions: references.iter().map(|r| r.to_string()).collect() }
}

fn show(reference: &str) -> BridgeCommand {
    BridgeCommand::SessionShow { session: reference.into() }
}

fn stored_session(h: &Harness, session: &Session) -> Session {
    h.engine.sessions_of(&session.bot_id).unwrap().into_iter().find(|s| s.id == session.id).unwrap()
}

#[tokio::test]
async fn only_master_reads_and_archives_sessions() {
    let h = harness();
    let _ = masters_turn(&h).await;
    let (_, pane) = user_session(&h, "query", "Hi.").await;
    for (command, name) in [
        (BridgeCommand::Sessions { archived: false, all: false }, "sessions"),
        (show("@query/1"), "session show"),
        (archive(&["@query/1"]), "session archive"),
    ] {
        let e = run(&h, &pane, command).await.error.unwrap();
        assert_eq!(e.code, ErrorCode::NotAllowed);
        assert_eq!(e.message, format!("Only @master can use `sbot {name}`."));
    }
}

#[tokio::test]
async fn sessions_lists_the_sidebar_by_default() {
    let h = harness();
    let (_, mp) = masters_turn(&h).await;
    let (p1, pp) = user_session(&h, "plan", "TASK-1201 plan\nMore.").await;
    finish_turn(&h, &pp).await;
    let (p2, _) = user_session(&h, "plan", "Second.").await;
    let (p3, _) = user_session(&h, "plan", "Third.").await;
    h.engine.archive_session(&p3.id).await.unwrap();
    let mine = |text: String| text.lines().filter(|l| l.starts_with("@plan/")).map(String::from).collect::<Vec<_>>();

    let open = mine(output(run(&h, &mp, BridgeCommand::Sessions { archived: false, all: false }).await));
    assert_eq!(open.len(), 2, "{open:?}");
    let archived = mine(output(run(&h, &mp, BridgeCommand::Sessions { archived: true, all: false }).await));
    assert_eq!(archived.len(), 1);
    assert!(archived[0].starts_with("@plan/3 · "), "{archived:?}");
    let all = mine(output(run(&h, &mp, BridgeCommand::Sessions { archived: false, all: true }).await));
    assert_eq!(all.len(), 3);

    let p1 = stored_session(&h, &p1);
    assert_eq!(
        open.iter().find(|l| l.starts_with("@plan/1")).unwrap(),
        &format!(
            "@plan/1 · \"TASK-1201 plan\" · idle · opened by the user · created {} · last activity {}",
            minute(&p1.created_at),
            minute(&p1.last_activity_at)
        )
    );
    let p2 = stored_session(&h, &p2);
    assert!(open.iter().any(|l| l.starts_with("@plan/2 · \"Second.\" · working · opened by the user")), "{open:?}");
    let p3 = stored_session(&h, &p3);
    assert!(
        archived[0].ends_with(&format!(" · archived {}", minute(p3.archived_at.as_ref().unwrap()))),
        "{archived:?}"
    );
    assert!(p2.archived_at.is_none());
}

#[tokio::test]
async fn sessions_say_who_opened_them_and_none_when_empty() {
    let h = harness();
    let (_, mp) = masters_turn(&h).await;
    let text = output(run(&h, &mp, BridgeCommand::Sessions { archived: true, all: false }).await);
    assert_eq!(text, "No sessions.");
    assert!(h.engine.handle_bridge(send(&mp, "@plan", "Plan it.")).await.ok);
    settle().await;
    let text = output(run(&h, &mp, BridgeCommand::Sessions { archived: false, all: false }).await);
    assert!(text.contains("@plan/1 · \"Session 1\" · working · opened by @master/1 · created "), "{text}");
}

fn untimed(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| match l.strip_prefix('[') {
            Some(rest) if rest.len() > 18 && rest.as_bytes()[16] == b']' => rest[18..].to_string(),
            _ => l.to_string(),
        })
        .collect()
}

#[tokio::test]
async fn session_show_has_the_user_chat_and_bot_messages() {
    let h = harness_after(|_| {});
    let (_, _mp) = masters_turn(&h).await;
    let (_, mp) = {
        let master = bot_named(&h, "master").await;
        let session = h.engine.sessions_of(&master.id).unwrap().remove(0);
        (session.clone(), pane(&h, &session))
    };
    let (p1, pp) = user_session(&h, "plan", "Plan TASK-1201.").await;
    let file = h.paths.bots_root.join("plan.md");
    std::fs::write(&file, "# plan").unwrap();
    assert!(h.engine.handle_bridge(send_files(&pp, "@review", "Review this.", &[file.as_path()])).await.ok);
    settle().await;
    assert!(h.engine.handle_bridge(reply(&pp, "Planned.")).await.ok);
    finish_turn(&h, &pp).await;
    let review = bot_named(&h, "review").await;
    let r1 = opened(&h, &review, &p1);
    let rp = pane(&h, &r1);
    assert!(h.engine.handle_bridge(reply(&rp, "Fine.")).await.ok);
    finish_turn(&h, &rp).await;
    let store = Store::open(&h.paths.db_path()).unwrap();
    store.insert_message(&NewMessage::system(&p1, "chain_limit_reached", json!({"limit": 12}))).unwrap();

    let text = output(run(&h, &mp, show("PLAN/1")).await);
    let (header, rest) = text.split_once("\n---\n").expect("a separator");
    let p1 = stored_session(&h, &p1);
    assert_eq!(
        header,
        format!(
            "@plan/1 · \"Plan TASK-1201.\"\ncreated {} · last activity {}\nopened by: the user\nopened: @review/1",
            minute(&p1.created_at),
            minute(&p1.last_activity_at)
        )
    );
    let lines = untimed(rest);
    let attachment = lines.iter().find(|l| l.starts_with("attachments: ")).expect("attachments").clone();
    assert!(attachment.contains("plan.md"), "{attachment}");
    let without: Vec<&str> = lines.iter().map(String::as_str).filter(|l| !l.starts_with("attachments: ")).collect();
    assert_eq!(
        without,
        [
            "user → @plan/1",
            "Plan TASK-1201.",
            "@plan/1 → @review/1",
            "Review this.",
            "@plan/1 → user",
            "Planned.",
            "@review/1 → @plan/1",
            "Fine.",
            "notice: chain_limit_reached {\"limit\":12}",
        ]
    );
    let at = lines.iter().position(|l| l == "Review this.").unwrap();
    assert_eq!(lines[at + 1], attachment);
}

#[tokio::test]
async fn show_marks_raw_output_and_messages_that_did_not_arrive() {
    let h = harness();
    let (_, mp) = masters_turn(&h).await;
    let (p1, pp) = user_session(&h, "plan", "Hi.").await;
    h.rt.state().output = "Plain terminal words.".into();
    finish_turn(&h, &pp).await;
    let text = output(run(&h, &mp, show("@plan/1")).await);
    assert!(text.contains("@plan/1 → user (raw terminal output)\n"), "{text}");
    let plan = bot_named(&h, "plan").await;
    h.engine.queue_user_message(&plan.id, "Later.").unwrap();
    h.engine.stop_session(&p1.id).await.unwrap();
    let text = output(run(&h, &mp, show("@plan/1")).await);
    assert!(text.contains("user → @plan/1 (cancelled)\nLater."), "{text}");
}

#[tokio::test]
async fn show_marks_the_prompt_of_a_scheduled_run() {
    let h = harness();
    let (_, mp) = masters_turn(&h).await;
    let plan = bot_named(&h, "plan").await;
    let local = |text: &str| {
        let naive = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap();
        Local.from_local_datetime(&naive).unwrap().with_timezone(&Utc)
    };
    let created = local("2027-06-07 08:00");
    h.engine.set_clock(std::sync::Arc::new(move || created));
    h.engine
        .create_task(NewTask {
            name: "Docs".into(),
            bot_id: plan.id.clone(),
            prompt: "Check the docs.".into(),
            every_days: 7,
            at: "09:00".into(),
            starts_on: None,
        })
        .unwrap();
    let due = local("2027-06-07 09:00");
    h.engine.set_clock(std::sync::Arc::new(move || due));
    h.engine.scheduler_tick().await;
    settle().await;

    let text = output(run(&h, &mp, show("@plan/1")).await);
    let (_, rest) = text.split_once("\n---\n").expect("a separator");
    assert_eq!(untimed(rest)[..2], ["user → @plan/1 (recurring task \"Docs\")", "Check the docs."]);
}

#[tokio::test]
async fn session_references_are_checked() {
    let h = harness();
    let (_, mp) = masters_turn(&h).await;
    let (_, _) = user_session(&h, "plan", "Hi.").await;
    for ok in ["plan/1", "@PLAN/1", "@plan/1"] {
        assert!(output(run(&h, &mp, show(ok)).await).starts_with("@plan/1 · "), "{ok}");
    }
    for bad in ["@plan", "@plan/x", "@nobody/1", "@plan/9"] {
        let e = run(&h, &mp, show(bad)).await.error.unwrap();
        assert_eq!(e.code, ErrorCode::BadRequest, "{bad}");
        assert_eq!(e.message, format!("No session {bad}; run `sbot sessions`."));
    }
}

#[tokio::test]
async fn archive_refuses_a_working_session_and_archives_the_rest() {
    let h = harness_after(|_| {});
    let (m1, mp) = masters_turn(&h).await;
    let (p1, _) = user_session(&h, "plan", "Working.").await;
    let (r1, rp) = user_session(&h, "review", "Done.").await;
    finish_turn(&h, &rp).await;
    let (q1, qp) = user_session(&h, "query", "Queued.").await;
    finish_turn(&h, &qp).await;
    let store = Store::open(&h.paths.db_path()).unwrap();
    let queued = NewMessage {
        sender: USER.into(),
        recipient: q1.bot_id.clone(),
        sender_session: None,
        recipient_session: Some(q1.id.clone()),
        kind: MessageKind::Text,
        body: "Next.".into(),
        attachments: vec![],
        system_code: None,
        system_params: None,
        status: MessageStatus::Queued,
        chain_id: "c1".into(),
        hop: 0,
        in_reply_to: None,
    };
    store.insert_message(&queued).unwrap();

    let text = output(run(&h, &mp, archive(&["@plan/1", "@review/1", "@x/9", "@review/1", "@query/1"])).await);
    assert_eq!(
        text,
        "@plan/1: not archived: it is working\n@review/1: archived\n@x/9: no such session\n\
         @review/1: already archived\n@query/1: not archived: it is working"
    );
    assert!(stored_session(&h, &p1).archived_at.is_none());
    assert!(stored_session(&h, &r1).archived_at.is_some());
    assert!(stored_session(&h, &q1).archived_at.is_none());

    let notices = |h: &Harness| -> Vec<serde_json::Value> {
        h.engine
            .session_messages(&m1.id)
            .unwrap()
            .into_iter()
            .filter(|m| m.system_code.as_deref() == Some("master_archived_sessions"))
            .filter_map(|m| m.system_params)
            .collect()
    };
    assert_eq!(notices(&h), [json!({"sessions": "Review · Done."})]);

    let e = run(&h, &mp, archive(&["@plan/1", "@x/9"])).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::BadRequest);
    assert_eq!(e.message, "@plan/1: not archived: it is working\n@x/9: no such session");
    let e = run(&h, &mp, archive(&["@review/1", "@x/9"])).await.error.unwrap();
    assert_eq!(e.message, "@review/1: already archived\n@x/9: no such session");

    let text = output(run(&h, &mp, archive(&["@review/1", "REVIEW/1"])).await);
    assert_eq!(text, "@review/1: already archived\n@review/1: already archived");
    assert_eq!(notices(&h).len(), 1);
}

#[tokio::test]
async fn archive_takes_the_tree_and_never_masters_own() {
    let h = harness();
    let (m1, mp) = masters_turn(&h).await;
    assert!(h.engine.handle_bridge(send(&mp, "@plan", "Plan it.")).await.ok);
    settle().await;
    let plan = bot_named(&h, "plan").await;
    let p1 = opened(&h, &plan, &m1);
    let pp = pane(&h, &p1);
    assert!(h.engine.handle_bridge(send(&pp, "@review", "Review it.")).await.ok);
    settle().await;
    let review = bot_named(&h, "review").await;
    let r1 = opened(&h, &review, &p1);
    let rp = pane(&h, &r1);
    assert!(h.engine.handle_bridge(send(&rp, "@testing", "Test it.")).await.ok);
    settle().await;
    let testing = bot_named(&h, "testing").await;
    let t1 = opened(&h, &testing, &r1);
    let tp = pane(&h, &t1);
    assert!(h.engine.handle_bridge(ask(&tp, "Which one?")).await.ok);
    assert!(stored_session(&h, &t1).waiting_question_id.is_some());
    for _ in 0..3 {
        for p in [&tp, &rp, &pp] {
            finish_turn(&h, p).await;
        }
    }

    let e = run(&h, &mp, archive(&["@master/1"])).await.error.unwrap();
    assert_eq!(e.message, "@master/1: not archived: it contains this conversation");
    assert!(stored_session(&h, &m1).archived_at.is_none());

    let text = output(run(&h, &mp, archive(&["@plan/1"])).await);
    assert_eq!(text, "@plan/1: archived, with @review/1, @testing/1");
    for s in [&p1, &r1, &t1] {
        assert!(stored_session(&h, s).archived_at.is_some());
    }
    assert!(stored_session(&h, &t1).waiting_question_id.is_none(), "its question clears");
    let notice = h
        .engine
        .session_messages(&m1.id)
        .unwrap()
        .into_iter()
        .find(|m| m.system_code.as_deref() == Some("master_archived_sessions"))
        .expect("a notice");
    assert_eq!(notice.system_params.unwrap(), json!({"sessions": "Plan #1"}));
}

#[tokio::test]
async fn archive_works_in_a_scheduled_run() {
    let h = harness();
    let _ = masters_turn(&h).await;
    let (p1, pp) = user_session(&h, "plan", "Old work.").await;
    finish_turn(&h, &pp).await;
    let master = bot_named(&h, "master").await;
    let naive = NaiveDateTime::parse_from_str("2027-06-07 08:00", "%Y-%m-%d %H:%M").unwrap();
    let now = |t: NaiveDateTime| Local.from_local_datetime(&t).unwrap().with_timezone(&Utc);
    h.engine.set_clock(std::sync::Arc::new(move || now(naive)));
    h.engine
        .create_task(NewTask {
            name: "Tidy".into(),
            bot_id: master.id.clone(),
            prompt: "Archive old sessions.".into(),
            every_days: 7,
            at: "09:00".into(),
            starts_on: None,
        })
        .unwrap();
    let due = NaiveDateTime::parse_from_str("2027-06-07 09:00", "%Y-%m-%d %H:%M").unwrap();
    h.engine.set_clock(std::sync::Arc::new(move || now(due)));
    h.engine.scheduler_tick().await;
    settle().await;
    let run_session = h.engine.sessions_of(&master.id).unwrap().into_iter().find(|s| s.number == 2).expect("the run");
    let rp = pane(&h, &run_session);

    let text = output(run(&h, &rp, archive(&["@plan/1"])).await);
    assert_eq!(text, "@plan/1: archived");
    assert!(stored_session(&h, &p1).archived_at.is_some());
    let notices: Vec<_> = h
        .engine
        .session_messages(&run_session.id)
        .unwrap()
        .into_iter()
        .filter(|m| m.system_code.as_deref() == Some("master_archived_sessions"))
        .collect();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].system_params, Some(json!({"sessions": "Plan · Old work."})));
}
