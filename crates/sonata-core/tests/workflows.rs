mod support;

use bridge_proto::{BridgeCommand, BridgeResponse, ErrorCode};
use serde_json::json;
use sonata_core::attachments::AttachmentError;
use sonata_core::engine::UiEvent;
use sonata_core::model::*;
use sonata_core::refusal::Refusal;
use sonata_core::runtime::{AgentState, RuntimeEvent};
use sonata_core::store::{NewRun, RunMoved, Store, VisitOpen};
use sonata_core::workflow::prompts::{finish_line, open_step_line, reminder_body};
use sonata_core::workflow::text::parse_text;
use sonata_core::CoreError;
use support::*;

fn step(n: usize, bot: &Bot, arrows: Vec<Arrow>) -> Step {
    Step {
        id: format!("s{n}"),
        name: format!("Step {n}"),
        bot_id: bot.id.clone(),
        instructions: format!("Do step {n}."),
        arrows,
        pos: None,
    }
}

fn arrow(to: &str) -> Arrow {
    Arrow { to: to.into(), outcome: None }
}

fn flow(bots: &[&Bot]) -> NewWorkflow {
    let steps = bots
        .iter()
        .enumerate()
        .map(|(i, bot)| {
            let to = if i + 1 < bots.len() { format!("s{}", i + 2) } else { DONE.to_string() };
            step(i + 1, bot, vec![arrow(&to)])
        })
        .collect();
    NewWorkflow {
        name: "Flow".into(),
        when_to_use: String::new(),
        def: WorkflowDef { version: WORKFLOW_FORMAT, start: Some("s1".into()), steps, start_pos: None, done_pos: None },
    }
}

fn ticket_flow(plan: &Bot, feature: &Bot, review: &Bot, testing: &Bot) -> NewWorkflow {
    let named = |to: &str, outcome: &str| Arrow { to: to.into(), outcome: Some(outcome.into()) };
    let mut wf = flow(&[plan, feature, review, testing]);
    wf.name = "Example review workflow".into();
    for (step, name) in wf.def.steps.iter_mut().zip(["Plan", "Implement", "Review", "Test"]) {
        step.name = name.into();
    }
    wf.def.steps[2].arrows = vec![named("s4", "approved"), named("s2", "changes requested")];
    wf.def.steps[3].arrows = vec![named(DONE, "passed"), named("s2", "failed")];
    wf
}

fn refusal(e: CoreError) -> Refusal {
    match e {
        CoreError::Refused(r) => r,
        other => panic!("expected a refusal, got {other}"),
    }
}

#[test]
fn a_workflow_is_saved_listed_changed_and_deleted() {
    let h = harness();
    let a = h.engine.create_bot(claude_bot("Alpha")).unwrap();
    let b = h.engine.create_bot(claude_bot("Beta")).unwrap();
    let mut ui = h.engine.subscribe_ui();
    assert!(h.engine.list_workflows().unwrap().is_empty());

    let saved = h.engine.create_workflow(flow(&[&a])).unwrap();
    assert_eq!(h.engine.list_workflows().unwrap(), vec![saved.clone()]);

    let mut edit = flow(&[&a, &b]);
    edit.when_to_use = "For two".into();
    let changed = h.engine.update_workflow(&saved.id, edit.clone()).unwrap();
    assert_eq!(changed.id, saved.id);
    assert_eq!(changed.def, edit.def);
    assert_eq!(h.engine.list_workflows().unwrap(), vec![changed.clone()]);

    let gone = h.engine.delete_workflow(&saved.id).unwrap();
    assert_eq!(gone.id, saved.id);
    assert!(h.engine.list_workflows().unwrap().is_empty());
    assert_eq!(refusal(h.engine.delete_workflow(&saved.id).unwrap_err()), Refusal::WorkflowNotFound(saved.id.clone()));

    let seen = events(&mut ui);
    assert_eq!(seen.iter().filter(|e| **e == UiEvent::WorkflowsChanged).count(), 3);
}

#[test]
fn a_name_is_unique_ignoring_case() {
    let h = harness();
    let a = h.engine.create_bot(claude_bot("Alpha")).unwrap();
    let first = h.engine.create_workflow(flow(&[&a])).unwrap();
    let mut again = flow(&[&a]);
    again.name = "flow".into();
    assert_eq!(refusal(h.engine.create_workflow(again).unwrap_err()), Refusal::WorkflowNameTaken("flow".into()));

    let mut other = flow(&[&a]);
    other.name = " Other ".into();
    let other = h.engine.create_workflow(other).unwrap();
    assert_eq!(other.name, "Other");
    let mut clash = flow(&[&a]);
    clash.name = "FLOW".into();
    assert_eq!(
        refusal(h.engine.update_workflow(&other.id, clash).unwrap_err()),
        Refusal::WorkflowNameTaken("FLOW".into())
    );
    let mut own = flow(&[&a]);
    own.name = "FLOW".into();
    assert_eq!(h.engine.update_workflow(&first.id, own).unwrap().name, "FLOW");
    let names: Vec<_> = h.engine.list_workflows().unwrap().into_iter().map(|w| w.name).collect();
    assert_eq!(names, vec!["FLOW", "Other"]);
}

#[test]
fn a_workflow_that_cannot_run_is_not_saved() {
    let h = harness();
    let a = h.engine.create_bot(claude_bot("Alpha")).unwrap();
    let mut wf = flow(&[&a]);
    wf.def.steps[0].bot_id = String::new();
    assert_eq!(
        refusal(h.engine.create_workflow(wf).unwrap_err()),
        Refusal::WorkflowStepNoBot { step: "Step 1".into() }
    );
    let mut wf = flow(&[&a]);
    wf.def.steps[0].bot_id = "gone".into();
    assert_eq!(
        refusal(h.engine.create_workflow(wf).unwrap_err()),
        Refusal::WorkflowStepNoBot { step: "Step 1".into() }
    );
    assert!(h.engine.list_workflows().unwrap().is_empty());
}

#[test]
fn workflows_survive_a_relaunch() {
    let mut bot_id = String::new();
    let h = harness_after(|store| {
        let bot = store.insert_bot(&claude_bot("Alpha"), "alpha").unwrap();
        bot_id = bot.id.clone();
        let wf = flow_for(&bot);
        store.insert_workflow(&wf).unwrap();
    });
    let listed = h.engine.list_workflows().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].def.steps[0].bot_id, bot_id);
    let again = relaunch(&h);
    assert_eq!(again.engine.list_workflows().unwrap(), listed);
}

fn flow_for(bot: &Bot) -> NewWorkflow {
    flow(&[bot])
}

#[test]
fn the_store_finds_a_workflow_by_name_and_records_its_creation() {
    use sonata_core::store::Store;
    let store = Store::open_in_memory().unwrap();
    let bot = store.insert_bot(&claude_bot("Alpha"), "alpha").unwrap();
    let wf = store.insert_workflow(&flow(&[&bot])).unwrap();
    assert_eq!(store.get_workflow_by_name("FLOW").unwrap(), Some(wf.clone()));
    assert_eq!(store.get_workflow(&wf.id).unwrap(), Some(wf));
    assert_eq!(store.get_workflow("nope").unwrap(), None);
}

fn prompt_of(h: &Harness, session: &Session) -> String {
    let pane = pane(h, session);
    h.rt.state().prompts.iter().rev().find(|(p, _)| *p == pane).map(|(_, text)| text.clone()).expect("a prompt")
}

fn first_message(h: &Harness, session: &Session) -> Message {
    h.engine.session_messages(&session.id).unwrap().remove(0)
}

fn plan_and_feature(h: &Harness) -> (Bot, Bot, Workflow) {
    let plan = h.engine.create_bot(claude_bot("Plan")).unwrap();
    let feature = h.engine.create_bot(claude_bot("Feature")).unwrap();
    let wf = h.engine.create_workflow(flow(&[&plan, &feature])).unwrap();
    (plan, feature, wf)
}

#[tokio::test]
async fn a_run_starts_at_the_first_step_in_a_new_session() {
    let h = harness_after(|_| {});
    let (plan, feature, wf) = plan_and_feature(&h);
    let session = h.engine.start_workflow(&wf.id, "TASK-1 build the export", &[]).unwrap();
    settle().await;

    assert_eq!(session.bot_id, plan.id);
    assert_eq!(session.opened_by, None);
    assert_eq!(session.title.as_deref(), Some("Flow · TASK-1 build the export"));
    let prompt = prompt_of(&h, &session);
    assert!(
        prompt.starts_with(
            "[Workflow \"Flow\" · step \"Step 1\" · visit 1 · started by the user · thread you↔user · \
             language: English]\nRequest:\nTASK-1 build the export"
        ),
        "{prompt}"
    );
    assert!(prompt.contains("Your step:\nDo step 1."), "{prompt}");
    assert!(prompt.contains(&finish_line(&[])), "{prompt}");
    assert!(h.engine.sessions_of(&feature.id).unwrap().is_empty(), "the next step's bot waits");

    let request = first_message(&h, &session);
    assert_eq!((request.sender.as_str(), request.status, request.hop), (USER, MessageStatus::Delivered, 0));
    let mark = request.workflow_mark.clone().expect("the request is marked");
    assert_eq!((mark.role, mark.visit), (MarkRole::Request, 1));
    let store = Store::open(&h.paths.db_path()).unwrap();
    let run = store.get_run(&mark.run_id).unwrap().expect("the run is stored");
    let [visit] = &store.run_visits(&run.id).unwrap()[..] else { panic!("one visit") };
    assert_eq!(
        mark,
        WorkflowMark {
            run_id: run.id.clone(),
            visit_id: visit.id.clone(),
            workflow: "Flow".into(),
            step: "Step 1".into(),
            visit: 1,
            role: MarkRole::Request,
            outcome: None,
        }
    );
    assert_eq!(store.get_message(&request.id).unwrap().unwrap().workflow_mark, Some(mark));
    assert_eq!((visit.ord, visit.step_id.as_str(), visit.n), (1, "s1", 1));
    assert_eq!(visit.session_id.as_deref(), Some(session.id.as_str()));
    assert_eq!(visit.request_message, request.id);
    assert_eq!((visit.result_message.as_deref(), visit.ended_at.as_deref(), visit.reminded), (None, None, false));
    assert_eq!(run.workflow_id.as_deref(), Some(wf.id.as_str()));
    assert_eq!((run.name.as_str(), run.request.as_str()), ("Flow", "TASK-1 build the export"));
    assert_eq!(run.def, wf.def);
    assert_eq!((run.status, run.pause.as_ref(), run.ended_at.as_deref()), (RunStatus::Running, None, None));
    assert_eq!((run.opener_session.as_deref(), run.root_session.as_deref()), (None, Some(session.id.as_str())));
    assert_eq!(store.run_session_of(&run.id, &plan.id).unwrap(), Some((session.id.clone(), 0)));
    assert_eq!(store.run_session_of(&run.id, &feature.id).unwrap(), None);
}

#[tokio::test]
async fn the_requests_files_reach_the_first_bots_inbox() {
    let h = harness_after(|_| {});
    let (plan, _, wf) = plan_and_feature(&h);
    let src = tempfile::tempdir().unwrap();
    let spec = src.path().join("spec.md");
    std::fs::write(&spec, "# Spec").unwrap();
    let spec = spec.to_string_lossy().into_owned();

    let gone = src.path().join("gone.md").to_string_lossy().into_owned();
    assert_eq!(
        refusal(h.engine.start_workflow(&wf.id, "Build it.", std::slice::from_ref(&gone)).unwrap_err()),
        Refusal::Attachment(AttachmentError::NotFound(gone))
    );
    assert!(h.engine.sessions_of(&plan.id).unwrap().is_empty());

    let session = h.engine.start_workflow(&wf.id, "", std::slice::from_ref(&spec)).unwrap();
    settle().await;
    assert_eq!(session.title.as_deref(), Some("Flow"));
    let request = first_message(&h, &session);
    let stored = format!("{}-spec.md", request.id);
    let copy = h.paths.bot_dir("plan").join("inbox").join(&stored);
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "# Spec");
    assert_eq!(request.attachments, vec![Attachment { name: "spec.md".into(), path: copy.to_string_lossy().into() }]);
    let prompt = prompt_of(&h, &session);
    assert!(!prompt.contains("Request:"), "{prompt}");
    assert!(prompt.ends_with(&format!("\n\nAttachments:\n- `inbox/{stored}`")), "{prompt}");
    let store = Store::open(&h.paths.db_path()).unwrap();
    let run = store.get_run(&request.workflow_mark.unwrap().run_id).unwrap().unwrap();
    assert_eq!(run.request_files, vec![spec], "the run keeps the user's own path");
}

#[tokio::test]
async fn a_run_is_refused_when_a_steps_bot_is_gone() {
    let h = harness();
    let (plan, feature, wf) = plan_and_feature(&h);
    h.engine.delete_bot(&feature.id).await.unwrap();
    let e = h.engine.start_workflow(&wf.id, "TASK-1 build the export", &[]).unwrap_err();
    assert_eq!(refusal(e), Refusal::WorkflowStepNoBot { step: "Step 2".into() });
    assert!(h.engine.sessions_of(&plan.id).unwrap().is_empty(), "no session is opened");
    assert!(h.rt.state().prompts.is_empty());
}

#[tokio::test]
async fn an_empty_request_is_refused() {
    let h = harness();
    let (plan, _, wf) = plan_and_feature(&h);
    assert_eq!(refusal(h.engine.start_workflow(&wf.id, " \n ", &[]).unwrap_err()), Refusal::MessageEmpty);
    assert_eq!(
        refusal(h.engine.start_workflow("01JNOFLOW", "TASK-1", &[]).unwrap_err()),
        Refusal::WorkflowNotFound("01JNOFLOW".into())
    );
    assert!(h.engine.sessions_of(&plan.id).unwrap().is_empty());
}

#[tokio::test]
async fn a_retried_step_request_keeps_its_mark() {
    let h = harness();
    let (_, _, wf) = plan_and_feature(&h);
    let session = h.engine.start_workflow(&wf.id, "TASK-1 build the export", &[]).unwrap();
    settle().await;
    let request = first_message(&h, &session);
    h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: pane(&h, &session) }).await;
    settle().await;
    assert_eq!(stored(&h, &request), MessageStatus::Interrupted);
    let copy = h.engine.retry_message(&request.id).unwrap();
    assert!(copy.workflow_mark.is_some());
    assert_eq!(copy.workflow_mark, request.workflow_mark);
    h.engine.pump_session(&session.id).await;
    let prompt = prompt_of(&h, &session);
    assert!(prompt.starts_with("[Workflow \"Flow\" · step \"Step 1\" · visit 1 · started by the user ·"), "{prompt}");
    assert!(h.engine.handle_bridge(result(&pane(&h, &session), "The plan.")).await.ok);
}

fn master_run(store: &Store) -> (Bot, Session, RunMoved) {
    let master = store.insert_bot(&claude_bot("Master"), "master").unwrap();
    let plan = store.insert_bot(&claude_bot("Plan"), "plan").unwrap();
    let wf = store.insert_workflow(&flow(&[&plan])).unwrap();
    let asker = store.create_session(&master.id, Some("Ship TASK-1"), None).unwrap();
    let run_id = "01JRUN".to_string();
    let visit_id = "01JVISIT".to_string();
    let message_id = "01JREQUEST".to_string();
    let run = NewRun {
        id: run_id.clone(),
        workflow_id: wf.id.clone(),
        name: wf.name.clone(),
        def: wf.def.clone(),
        request: "TASK-1".into(),
        request_files: vec![],
        opener_session: Some(asker.id.clone()),
    };
    let first = VisitOpen {
        visit_id: visit_id.clone(),
        step_id: "s1".into(),
        n: 1,
        bot_id: plan.id.clone(),
        session: None,
        title: "Flow · TASK-1".into(),
        opened_by: Some(asker.id.clone()),
        message_id: message_id.clone(),
        message: NewMessage {
            sender: USER.into(),
            recipient: plan.id.clone(),
            sender_session: None,
            recipient_session: None,
            kind: MessageKind::Text,
            body: "Request:\nTASK-1".into(),
            attachments: vec![],
            system_code: None,
            system_params: None,
            status: MessageStatus::Queued,
            chain_id: "01JCHAIN".into(),
            hop: 0,
            in_reply_to: None,
        },
        mark: WorkflowMark {
            run_id: run_id.clone(),
            visit_id,
            workflow: wf.name.clone(),
            step: "Step 1".into(),
            visit: 1,
            role: MarkRole::Request,
            outcome: None,
        },
        seen_ord: 0,
    };
    let moved = store.start_workflow_run(&run, &first).unwrap();
    (plan, asker, moved)
}

#[tokio::test]
async fn a_run_a_bot_started_sits_under_its_session_and_names_it() {
    let mut ids = (String::new(), String::new());
    let h = harness_after(|store| {
        let (plan, asker, moved) = master_run(store);
        let opened = moved.opened.expect("the first visit");
        assert!(opened.created);
        assert_eq!(opened.session.opened_by.as_deref(), Some(asker.id.as_str()));
        assert_eq!(opened.message.id, "01JREQUEST");
        assert_eq!(opened.message.recipient_session.as_deref(), Some(opened.session.id.as_str()));
        assert_eq!((moved.result, moved.finished), (None, None));
        assert_eq!(moved.run.opener_session.as_deref(), Some(asker.id.as_str()));
        assert_eq!(moved.run.root_session.as_deref(), Some(opened.session.id.as_str()));
        assert_eq!(store.run_session_of("01JRUN", &plan.id).unwrap(), Some((opened.session.id.clone(), 0)));
        ids = (plan.id, opened.session.id);
    });
    let (plan_id, session_id) = ids;
    h.engine.pump(&plan_id).await;
    let session = h.engine.sessions_of(&plan_id).unwrap().remove(0);
    assert_eq!(session.id, session_id);
    let prompt = prompt_of(&h, &session);
    assert!(
        prompt.starts_with(
            "[Workflow \"Flow\" · step \"Step 1\" · visit 1 · started by @master · thread you↔user · \
             language: English]\nRequest:\nTASK-1"
        ),
        "{prompt}"
    );
}

struct Ticket {
    feature: Bot,
    review: Bot,
    testing: Bot,
    wf: Workflow,
}

fn ticket(h: &Harness) -> Ticket {
    let plan = h.engine.create_bot(claude_bot("Plan")).unwrap();
    let feature = h.engine.create_bot(claude_bot("Feature")).unwrap();
    let review = h.engine.create_bot(claude_bot("Review")).unwrap();
    let testing = h.engine.create_bot(claude_bot("Testing")).unwrap();
    let wf = h.engine.create_workflow(ticket_flow(&plan, &feature, &review, &testing)).unwrap();
    Ticket { feature, review, testing, wf }
}

fn only_session(h: &Harness, bot: &Bot) -> Session {
    let sessions = h.engine.sessions_of(&bot.id).unwrap();
    let [session] = &sessions[..] else { panic!("@{} has {} sessions", bot.handle, sessions.len()) };
    session.clone()
}

async fn hand_in(h: &Harness, session: &Session, body: &str, outcome: Option<&str>) -> BridgeResponse {
    let pane = pane(h, session);
    let req = match outcome {
        Some(outcome) => result_with(&pane, body, outcome),
        None => result(&pane, body),
    };
    let response = h.engine.handle_bridge(req).await;
    finish_turn(h, &pane).await;
    response
}

async fn to_review(h: &Harness, t: &Ticket) -> (Session, Session) {
    let root = h.engine.start_workflow(&t.wf.id, "TASK-1 build the export", &[]).unwrap();
    settle().await;
    assert!(hand_in(h, &root, "The plan.", None).await.ok);
    let feature = only_session(h, &t.feature);
    assert!(hand_in(h, &feature, "Built it.", None).await.ok);
    (root, only_session(h, &t.review))
}

fn run_of(h: &Harness, session: &Session) -> (WorkflowRun, Vec<Visit>) {
    let mark = first_message(h, session).workflow_mark.expect("a run session starts with a marked visit");
    let store = Store::open(&h.paths.db_path()).unwrap();
    (store.get_run(&mark.run_id).unwrap().expect("the run"), store.run_visits(&mark.run_id).unwrap())
}

fn refused(response: BridgeResponse) -> (ErrorCode, String) {
    let e = response.error.expect("a refusal");
    (e.code, e.message)
}

#[tokio::test]
async fn an_outcome_picks_the_arrow() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (root, review) = to_review(&h, &t).await;
    let review_pane = pane(&h, &review);

    let response = h.engine.handle_bridge(result_with(&review_pane, "LGTM", "approved")).await;
    assert!(response.ok, "{response:?}");
    assert_eq!(
        response.output.as_deref(),
        Some(
            "Result recorded for step \"Review\" (outcome \"approved\"). The workflow moves on to step \"Test\" \
             (@testing). End your turn."
        )
    );
    finish_turn(&h, &review_pane).await;

    let testing = only_session(&h, &t.testing);
    assert_eq!(testing.opened_by.as_deref(), Some(root.id.as_str()), "the run's sessions hang under its root");
    let prompt = prompt_of(&h, &testing);
    assert!(
        prompt.starts_with("[Workflow \"Example review workflow\" · step \"Test\" · visit 1 · started by the user ·"),
        "{prompt}"
    );
    assert!(prompt.contains("--- Step \"Review\" (@review), visit 1, outcome \"approved\":\nLGTM"), "{prompt}");

    let chat = h.engine.session_messages(&review.id).unwrap();
    let lgtm = chat.iter().find(|m| m.body == "LGTM").expect("the result is in Review's chat");
    assert_eq!(
        (lgtm.sender.as_str(), lgtm.recipient.as_str(), lgtm.status),
        (t.review.id.as_str(), USER, MessageStatus::Delivered)
    );
    let mark = lgtm.workflow_mark.clone().expect("the result is marked");
    assert_eq!(
        (mark.role, mark.step.as_str(), mark.visit, mark.outcome.as_deref()),
        (MarkRole::Result, "Review", 1, Some("approved"))
    );
    assert!(chat.iter().all(|m| m.kind != MessageKind::RawFallback && m.system_code.is_none()), "{chat:?}");

    let (run, visits) = run_of(&h, &review);
    assert_eq!((run.status, run.pause), (RunStatus::Running, None));
    let steps: Vec<(&str, u32, bool)> =
        visits.iter().map(|v| (v.step_id.as_str(), v.n, v.ended_at.is_some())).collect();
    assert_eq!(steps, vec![("s1", 1, true), ("s2", 1, true), ("s3", 1, true), ("s4", 1, false)]);
    assert_eq!(visits[2].outcome.as_deref(), Some("approved"));
    assert_eq!(visits[2].result_message.as_deref(), Some(lgtm.id.as_str()));
    assert_eq!((visits[0].outcome.as_deref(), visits[1].outcome.as_deref()), (None, None), "no outcome to pick");
    assert_eq!(visits[3].session_id.as_deref(), Some(testing.id.as_str()));
}

#[tokio::test]
async fn a_missing_or_unknown_outcome_is_refused_and_nothing_moves() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (_, review) = to_review(&h, &t).await;
    let review_pane = pane(&h, &review);

    let missing = h.engine.handle_bridge(result(&review_pane, "LGTM")).await;
    assert_eq!(
        refused(missing),
        (
            ErrorCode::BadRequest,
            "Step \"Review\" has several outcomes. Add --outcome with one of: \"approved\", \"changes requested\"."
                .into()
        )
    );
    let unknown = h.engine.handle_bridge(result_with(&review_pane, "LGTM", "maybe")).await;
    assert_eq!(
        refused(unknown),
        (
            ErrorCode::BadRequest,
            "\"maybe\" is not an outcome of step \"Review\". Use one of: \"approved\", \"changes requested\".".into()
        )
    );
    settle().await;
    assert!(h.engine.sessions_of(&t.testing.id).unwrap().is_empty(), "no session is opened");
    let (run, visits) = run_of(&h, &review);
    assert_eq!((run.status, visits.len()), (RunStatus::Running, 3));
    assert_eq!((visits[2].ended_at.as_deref(), visits[2].result_message.as_deref()), (None, None));
    assert!(h.engine.session_messages(&review.id).unwrap().iter().all(|m| m.body != "LGTM"));

    let again = h.engine.handle_bridge(result_with(&review_pane, "LGTM", " Approved ")).await;
    assert!(again.ok, "{again:?}");
    let (_, visits) = run_of(&h, &review);
    assert_eq!(visits[2].outcome.as_deref(), Some("approved"));
}

#[tokio::test]
async fn a_loop_back_resumes_the_same_session_with_only_what_it_has_not_seen() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (_, review) = to_review(&h, &t).await;
    let feature = only_session(&h, &t.feature);

    let response = hand_in(&h, &review, "Fix the header.", Some("changes requested")).await;
    assert_eq!(
        response.output.as_deref(),
        Some(
            "Result recorded for step \"Review\" (outcome \"changes requested\"). The workflow moves on to step \
             \"Implement\" (@feature). End your turn."
        )
    );
    assert_eq!(only_session(&h, &t.feature).id, feature.id, "the same conversation");
    let prompt = prompt_of(&h, &feature);
    assert!(prompt.starts_with("[Workflow \"Example review workflow\" · step \"Implement\" · visit 2 ·"), "{prompt}");
    assert!(
        prompt.contains(
            "Results so far:\n--- Step \"Review\" (@review), visit 1, outcome \"changes requested\":\nFix the header.\n\n\
             Your step:\nDo step 2."
        ),
        "{prompt}"
    );
    assert!(!prompt.contains("Request:"), "{prompt}");
    assert!(!prompt.contains("Built it."), "its own result: {prompt}");
    assert!(!prompt.contains("The plan."), "seen on its first visit: {prompt}");

    assert!(hand_in(&h, &feature, "Fixed the header.", None).await.ok);
    assert_eq!(only_session(&h, &t.review).id, review.id);
    let prompt = prompt_of(&h, &review);
    assert!(prompt.contains("· step \"Review\" · visit 2 ·"), "{prompt}");
    assert!(
        prompt.contains("Results so far:\n--- Step \"Implement\" (@feature), visit 2:\nFixed the header."),
        "{prompt}"
    );
    assert_eq!(prompt.matches("--- Step").count(), 1, "{prompt}");
    assert!(hand_in(&h, &review, "Now the footer.", Some("changes requested")).await.ok);
    let prompt = prompt_of(&h, &feature);
    assert!(prompt.contains("· step \"Implement\" · visit 3 ·"), "{prompt}");
    assert!(prompt.contains("--- Step \"Review\" (@review), visit 2, outcome \"changes requested\":\nNow the footer."));
    assert_eq!(prompt.matches("--- Step").count(), 1, "seen on visit 2: {prompt}");
}

#[tokio::test]
async fn a_first_visit_sees_every_earlier_result() {
    let h = harness_after(|_| {});
    let a = h.engine.create_bot(claude_bot("Alpha")).unwrap();
    let b = h.engine.create_bot(claude_bot("Beta")).unwrap();
    let c = h.engine.create_bot(claude_bot("Gamma")).unwrap();
    let wf = h.engine.create_workflow(flow(&[&a, &b, &c])).unwrap();
    let root = h.engine.start_workflow(&wf.id, "Do the three things.", &[]).unwrap();
    settle().await;
    assert!(hand_in(&h, &root, "First result.", None).await.ok);
    assert!(hand_in(&h, &only_session(&h, &b), "Second result.", None).await.ok);

    let prompt = prompt_of(&h, &only_session(&h, &c));
    assert!(
        prompt.contains(
            "Request:\nDo the three things.\n\nResults so far:\n\
             --- Step \"Step 1\" (@alpha), visit 1:\nFirst result.\n\n\
             --- Step \"Step 2\" (@beta), visit 1:\nSecond result.\n\n\
             Your step:\nDo step 3."
        ),
        "{prompt}"
    );
}

#[tokio::test]
async fn an_arrow_to_done_finishes_the_run() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (root, review) = to_review(&h, &t).await;
    assert!(hand_in(&h, &review, "LGTM", Some("approved")).await.ok);
    let testing = only_session(&h, &t.testing);
    let testing_pane = pane(&h, &testing);

    let response = h.engine.handle_bridge(result_with(&testing_pane, "All green.", "passed")).await;
    assert_eq!(
        response.output.as_deref(),
        Some("Result recorded for step \"Test\" (outcome \"passed\"). The workflow is finished. End your turn.")
    );
    let (run, visits) = run_of(&h, &root);
    assert_eq!((run.status, run.pause), (RunStatus::Finished, None));
    assert!(run.ended_at.is_some());
    assert_eq!(visits.len(), 4);
    assert!(visits.iter().all(|v| v.ended_at.is_some()));
    assert_eq!(visits[3].outcome.as_deref(), Some("passed"));
    let notices: Vec<Message> = h
        .engine
        .session_messages(&root.id)
        .unwrap()
        .into_iter()
        .filter(|m| m.system_code.as_deref() == Some("workflow_finished"))
        .collect();
    let [notice] = &notices[..] else { panic!("one notice: {notices:?}") };
    assert_eq!(
        notice.system_params,
        Some(json!({"name": "Example review workflow", "step": "Test", "outcome": "passed"}))
    );

    let again = h.engine.handle_bridge(result(&testing_pane, "One more thing.")).await;
    assert_eq!(refused(again).0, ErrorCode::NoOpenRequest);
}

#[tokio::test]
async fn a_last_step_without_outcomes_finishes_with_an_empty_one() {
    let h = harness_after(|_| {});
    let solo = h.engine.create_bot(claude_bot("Solo")).unwrap();
    let wf = h.engine.create_workflow(flow(&[&solo])).unwrap();
    let root = h.engine.start_workflow(&wf.id, "One thing.", &[]).unwrap();
    settle().await;
    let response = h.engine.handle_bridge(result(&pane(&h, &root), "Done.")).await;
    assert_eq!(
        response.output.as_deref(),
        Some("Result recorded for step \"Step 1\". The workflow is finished. End your turn.")
    );
    let notice = h
        .engine
        .session_messages(&root.id)
        .unwrap()
        .into_iter()
        .find(|m| m.system_code.as_deref() == Some("workflow_finished"))
        .expect("the notice");
    assert_eq!(notice.system_params, Some(json!({"name": "Flow", "step": "Step 1", "outcome": ""})));
    assert_eq!(run_of(&h, &root).0.status, RunStatus::Finished);
}

#[tokio::test]
async fn a_results_files_reach_the_next_bot() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (_, review) = to_review(&h, &t).await;
    let report = h.paths.bot_dir("review").join("report.md");
    std::fs::write(&report, "# Report").unwrap();
    let report = report.to_string_lossy().into_owned();

    let command = BridgeCommand::Result {
        body: "LGTM, see the report.".into(),
        files: vec![report.clone()],
        outcome: Some("approved".into()),
    };
    assert!(h.engine.handle_bridge(request(&pane(&h, &review), command)).await.ok);
    settle().await;

    let testing = only_session(&h, &t.testing);
    let visit = first_message(&h, &testing);
    let stored = format!("{}-report.md", visit.id);
    let copy = h.paths.bot_dir("testing").join("inbox").join(&stored);
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "# Report");
    assert_eq!(visit.attachments, vec![Attachment { name: "report.md".into(), path: copy.to_string_lossy().into() }]);
    let prompt = prompt_of(&h, &testing);
    assert!(prompt.ends_with(&format!("\n\nAttachments:\n- `inbox/{stored}`")), "{prompt}");
    let chat = h.engine.session_messages(&review.id).unwrap();
    let result = chat.iter().find(|m| m.body == "LGTM, see the report.").expect("the result");
    assert_eq!(result.attachments, vec![Attachment { name: "report.md".into(), path: report }]);
}

#[tokio::test]
async fn a_file_gone_before_the_next_visit_is_left_out() {
    let h = harness_after(|_| {});
    let a = h.engine.create_bot(claude_bot("Alpha")).unwrap();
    let b = h.engine.create_bot(claude_bot("Beta")).unwrap();
    let c = h.engine.create_bot(claude_bot("Gamma")).unwrap();
    let wf = h.engine.create_workflow(flow(&[&a, &b, &c])).unwrap();
    let root = h.engine.start_workflow(&wf.id, "Do the three things.", &[]).unwrap();
    settle().await;
    let with_file = |pane: &str, body: &str, file: &std::path::Path| {
        let files = vec![file.to_string_lossy().into_owned()];
        request(pane, BridgeCommand::Result { body: body.into(), files, outcome: None })
    };
    let notes = h.paths.bot_dir("alpha").join("notes.md");
    std::fs::write(&notes, "# Notes").unwrap();
    let root_pane = pane(&h, &root);
    assert!(h.engine.handle_bridge(with_file(&root_pane, "Notes attached.", &notes)).await.ok);
    finish_turn(&h, &root_pane).await;
    let beta = only_session(&h, &b);
    assert_eq!(first_message(&h, &beta).attachments.len(), 1, "Beta got the notes");

    std::fs::remove_file(&notes).unwrap();
    let draft = h.paths.bot_dir("beta").join("draft.md");
    std::fs::write(&draft, "# Draft").unwrap();
    assert!(h.engine.handle_bridge(with_file(&pane(&h, &beta), "Draft attached.", &draft)).await.ok);
    settle().await;

    let gamma = only_session(&h, &c);
    let visit = first_message(&h, &gamma);
    let names: Vec<&str> = visit.attachments.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, vec!["draft.md"]);
    let inbox: Vec<String> = std::fs::read_dir(h.paths.bot_dir("gamma").join("inbox"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(inbox, vec![format!("{}-draft.md", visit.id)]);
    let prompt = prompt_of(&h, &gamma);
    assert!(prompt.contains("--- Step \"Step 1\" (@alpha), visit 1:\nNotes attached."), "{prompt}");
    assert!(prompt.ends_with(&format!("\n\nAttachments:\n- `inbox/{}-draft.md`", visit.id)), "{prompt}");
}

#[tokio::test]
async fn two_runs_keep_their_own_sessions() {
    let h = harness_after(|_| {});
    let (plan, feature, wf) = plan_and_feature(&h);
    let first = h.engine.start_workflow(&wf.id, "TASK-1 build the export", &[]).unwrap();
    let second = h.engine.start_workflow(&wf.id, "TASK-2 fix the import", &[]).unwrap();
    settle().await;
    assert_ne!(first.id, second.id);
    assert_eq!(h.engine.sessions_of(&plan.id).unwrap().len(), 2);
    assert!(hand_in(&h, &first, "Plan for TASK-1.", None).await.ok);
    assert!(hand_in(&h, &second, "Plan for TASK-2.", None).await.ok);

    assert_eq!(h.engine.sessions_of(&feature.id).unwrap().len(), 2);
    for (root, own, other) in [(&first, "TASK-1", "TASK-2"), (&second, "TASK-2", "TASK-1")] {
        let session = opened(&h, &feature, root);
        let prompt = prompt_of(&h, &session);
        assert!(prompt.contains(&format!("--- Step \"Step 1\" (@plan), visit 1:\nPlan for {own}.")), "{prompt}");
        assert!(!prompt.contains(other), "{prompt}");
        assert_eq!(run_of(&h, &session).0.root_session.as_deref(), Some(root.id.as_str()));
    }
}

const NOT_A_STEP: &str = "--outcome is only for a workflow step, and this conversation has no open step.";

#[tokio::test]
async fn an_outcome_outside_an_open_step_is_refused() {
    let h = harness_after(|_| {});
    let (bot, _) = bot_with_delivered(&h, "hello").await;
    let plain = pane(&h, &only_session(&h, &bot));
    let response = h.engine.handle_bridge(result_with(&plain, "Done.", "approved")).await;
    assert_eq!(refused(response), (ErrorCode::BadRequest, NOT_A_STEP.into()));

    let solo = h.engine.create_bot(claude_bot("Solo")).unwrap();
    let wf = h.engine.create_workflow(flow(&[&solo])).unwrap();
    let root = h.engine.start_workflow(&wf.id, "One thing.", &[]).unwrap();
    settle().await;
    let root_pane = pane(&h, &root);
    assert!(h.engine.handle_bridge(result(&root_pane, "Done.")).await.ok);
    let response = h.engine.handle_bridge(result_with(&root_pane, "Again.", "approved")).await;
    assert_eq!(refused(response), (ErrorCode::BadRequest, NOT_A_STEP.into()));
}

const PAUSED: &str = "The workflow is paused for the user. End your turn.";

fn prompts_of(h: &Harness, session: &Session) -> Vec<String> {
    let pane = pane(h, session);
    h.rt.state().prompts.iter().filter(|(p, _)| *p == pane).map(|(_, text)| text.clone()).collect()
}

#[tokio::test]
async fn a_fourth_visit_pauses_the_run() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (_, review) = to_review(&h, &t).await;
    let feature = only_session(&h, &t.feature);
    for round in 1..=2 {
        assert!(hand_in(&h, &review, &format!("Fix round {round}."), Some("changes requested")).await.ok);
        assert!(hand_in(&h, &feature, &format!("Fixed round {round}."), None).await.ok);
    }
    let response = hand_in(&h, &review, "Still wrong.", Some("changes requested")).await;
    assert_eq!(
        response.output,
        Some(format!("Result recorded for step \"Review\" (outcome \"changes requested\"). {PAUSED}"))
    );
    let (run, visits) = run_of(&h, &review);
    assert_eq!((run.status, run.pause), (RunStatus::Paused, Some(PauseReason::VisitCap { step_id: "s2".into() })));
    assert!(visits.iter().all(|v| v.ended_at.is_some()), "{visits:?}");
    assert_eq!(visits.iter().filter(|v| v.step_id == "s2").count(), 3);
    assert!(prompt_of(&h, &feature).contains("· visit 3 ·"));
    assert!(prompts_of(&h, &feature).iter().all(|p| !p.contains("· visit 4 ·")), "@feature gets no 4th prompt");

    h.engine.another_round(&run.id).unwrap();
    settle().await;
    let prompt = prompt_of(&h, &feature);
    assert!(prompt.starts_with("[Workflow \"Example review workflow\" · step \"Implement\" · visit 4 ·"), "{prompt}");
    assert!(
        prompt.contains("--- Step \"Review\" (@review), visit 3, outcome \"changes requested\":\nStill wrong."),
        "{prompt}"
    );
    let (run, visits) = run_of(&h, &review);
    assert_eq!((run.status, run.pause), (RunStatus::Running, None));
    assert_eq!(run.extra_visits.get("s2"), Some(&1));
    let last = visits.last().expect("the 4th visit");
    assert_eq!((last.step_id.as_str(), last.n, last.ended_at.as_deref()), ("s2", 4, None));
    assert_eq!(refusal(h.engine.another_round(&run.id).unwrap_err()), Refusal::WorkflowRunNotPaused);
    assert_eq!(
        refusal(h.engine.another_round("01JNORUN").unwrap_err()),
        Refusal::WorkflowRunNotFound("01JNORUN".into())
    );
}

#[tokio::test]
async fn a_step_whose_bot_is_gone_pauses_the_run() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (_, review) = to_review(&h, &t).await;
    h.engine.delete_bot(&t.testing.id).await.unwrap();
    let response = hand_in(&h, &review, "LGTM", Some("approved")).await;
    assert_eq!(response.output, Some(format!("Result recorded for step \"Review\" (outcome \"approved\"). {PAUSED}")));
    let (run, visits) = run_of(&h, &review);
    assert_eq!((run.status, run.pause), (RunStatus::Paused, Some(PauseReason::BotMissing { step_id: "s4".into() })));
    assert_eq!((visits.len(), visits[2].outcome.as_deref()), (3, Some("approved")));
}

#[tokio::test]
async fn a_late_result_clears_its_steps_pause() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (_, review) = to_review(&h, &t).await;
    let review_pane = pane(&h, &review);
    let (run, _) = run_of(&h, &review);
    let pause_at = |step: &str| {
        let pause = serde_json::to_string(&PauseReason::NoResult { step_id: step.into() }).unwrap();
        let db = rusqlite::Connection::open(h.paths.db_path()).unwrap();
        db.execute("UPDATE workflow_runs SET status = 'paused', pause = ?2 WHERE id = ?1", [&run.id, &pause]).unwrap();
    };
    let report = h.paths.bot_dir("review").join("report.md");
    std::fs::write(&report, "# Report").unwrap();
    let with_report = || {
        let files = vec![report.to_string_lossy().into_owned()];
        let command = BridgeCommand::Result { body: "LGTM".into(), files, outcome: Some("approved".into()) };
        request(&review_pane, command)
    };

    pause_at("s2");
    let response = h.engine.handle_bridge(with_report()).await;
    assert_eq!(refused(response).0, ErrorCode::NoOpenRequest);
    assert!(h.engine.sessions_of(&t.testing.id).unwrap().is_empty());
    let inbox = h.paths.bot_dir("testing").join("inbox");
    assert!(!inbox.exists() || std::fs::read_dir(&inbox).unwrap().next().is_none(), "the inbox copy is removed");

    pause_at("s3");
    let response = h.engine.handle_bridge(with_report()).await;
    assert!(response.ok, "{response:?}");
    let (run, visits) = run_of(&h, &review);
    assert_eq!((run.status, run.pause), (RunStatus::Running, None));
    assert_eq!((visits.len(), visits[2].outcome.as_deref()), (4, Some("approved")));
}

#[tokio::test]
async fn a_step_takes_a_result_only_once_its_request_was_delivered() {
    let h = harness_after(|_| {});
    let a = h.engine.create_bot(claude_bot("Alpha")).unwrap();
    let b = h.engine.create_bot(claude_bot("Beta")).unwrap();
    let wf = h.engine.create_workflow(flow(&[&a, &a, &b])).unwrap();
    let root = h.engine.start_workflow(&wf.id, "Do the three things.", &[]).unwrap();
    settle().await;
    let root_pane = pane(&h, &root);
    let response = h.engine.handle_bridge(result(&root_pane, "One.")).await;
    assert_eq!(
        response.output.as_deref(),
        Some("Result recorded for step \"Step 1\". The workflow moves on to step \"Step 2\" (@alpha). End your turn.")
    );

    let again = h.engine.handle_bridge(result(&root_pane, "Two?")).await;
    assert_eq!(refused(again).0, ErrorCode::NoOpenRequest);
    let (run, visits) = run_of(&h, &root);
    assert_eq!(run.status, RunStatus::Running);
    let steps: Vec<(&str, bool)> = visits.iter().map(|v| (v.step_id.as_str(), v.ended_at.is_some())).collect();
    assert_eq!(steps, vec![("s1", true), ("s2", false)]);
    let step_two = h.engine.session_messages(&root.id).unwrap();
    let step_two = step_two.iter().find(|m| m.id == visits[1].request_message).expect("step 2's request");
    assert_eq!(step_two.status, MessageStatus::Queued);
    assert!(h.engine.session_messages(&root.id).unwrap().iter().all(|m| m.body != "Two?"));
    assert!(h.engine.sessions_of(&b.id).unwrap().is_empty());

    finish_turn(&h, &root_pane).await;
    assert!(prompt_of(&h, &root).starts_with("[Workflow \"Flow\" · step \"Step 2\" · visit 1 ·"));
    let response = h.engine.handle_bridge(result(&root_pane, "Two.")).await;
    assert_eq!(
        response.output.as_deref(),
        Some("Result recorded for step \"Step 2\". The workflow moves on to step \"Step 3\" (@beta). End your turn.")
    );
    let (_, visits) = run_of(&h, &root);
    assert_eq!(visits.iter().filter(|v| v.ended_at.is_some()).count(), 2);
}

#[tokio::test]
async fn a_result_on_a_bots_message_marks_that_message_answered() {
    let h = harness_after(|_| {});
    let a = h.engine.create_bot(claude_bot("Alpha")).unwrap();
    let b = h.engine.create_bot(claude_bot("Beta")).unwrap();
    let helper = h.engine.create_bot(claude_bot("Helper")).unwrap();
    let wf = h.engine.create_workflow(flow(&[&a, &b])).unwrap();
    let root = h.engine.start_workflow(&wf.id, "Do the two things.", &[]).unwrap();
    settle().await;
    let root_pane = pane(&h, &root);
    assert!(h.engine.handle_bridge(send(&root_pane, "@helper", "Check the numbers.")).await.ok);
    finish_turn(&h, &root_pane).await;
    let helper_pane = pane(&h, &only_session(&h, &helper));
    let answer = h.engine.handle_bridge(send(&helper_pane, "@alpha", "The numbers add up.")).await;
    let answer_id = answer.message_id.expect("the answer is stored");
    finish_turn(&h, &helper_pane).await;
    assert!(prompt_of(&h, &root).contains("The numbers add up."), "the step's session is at work on @helper's answer");

    let response = h.engine.handle_bridge(result(&root_pane, "Done, the numbers add up.")).await;
    assert!(response.ok, "{response:?}");
    let result_id = response.message_id.expect("the result's id");
    assert_eq!(h.engine.answered_by(&answer_id).unwrap(), Some(result_id.clone()));
    let chat = h.engine.session_messages(&root.id).unwrap();
    let result = chat.iter().find(|m| m.id == result_id).expect("the result is in the step's chat");
    assert_eq!((result.recipient.as_str(), result.in_reply_to.as_deref()), (USER, None));
    let (_, visits) = run_of(&h, &root);
    assert_eq!(visits[0].result_message.as_deref(), Some(result_id.as_str()));
    assert!(visits[0].ended_at.is_some());
    assert_eq!(only_session(&h, &b).opened_by.as_deref(), Some(root.id.as_str()));
}

fn reminders(h: &Harness, session: &Session) -> Vec<Message> {
    h.engine
        .session_messages(&session.id)
        .unwrap()
        .into_iter()
        .filter(|m| m.workflow_mark.as_ref().is_some_and(|mark| mark.role == MarkRole::Reminder))
        .collect()
}

fn review_outcomes() -> Vec<String> {
    vec!["approved".into(), "changes requested".into()]
}

async fn to_a_result_pause(h: &Harness, t: &Ticket) -> (Session, Session) {
    let (root, review) = to_review(h, t).await;
    let review_pane = pane(h, &review);
    assert!(h.engine.handle_bridge(reply(&review_pane, "working on it")).await.ok);
    finish_turn(h, &review_pane).await;
    finish_turn(h, &review_pane).await;
    (root, review)
}

#[tokio::test]
async fn a_turn_without_a_result_is_reminded_once_then_pauses() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (_, review) = to_review(&h, &t).await;
    let review_pane = pane(&h, &review);
    assert!(h.engine.handle_bridge(reply(&review_pane, "working on it")).await.ok);
    finish_turn(&h, &review_pane).await;

    let prompt = prompt_of(&h, &review);
    assert!(
        prompt.starts_with(&format!(
            "[Workflow \"Example review workflow\" · step \"Review\" · visit 1 · started by the user · thread \
             you↔user · language: English]\n{}",
            reminder_body("Review", &review_outcomes())
        )),
        "{prompt}"
    );
    assert_eq!(
        reminder_body("Review", &review_outcomes()),
        format!(
            "Step \"Review\" is still open: your turn ended without its result. {}",
            finish_line(&review_outcomes())
        )
    );
    let [reminder] = &reminders(&h, &review)[..] else { panic!("one reminder") };
    assert_eq!((reminder.sender.as_str(), reminder.status, reminder.hop), (USER, MessageStatus::Delivered, 0));
    let (run, visits) = run_of(&h, &review);
    let mark = reminder.workflow_mark.clone().unwrap();
    assert_eq!(
        mark,
        WorkflowMark {
            run_id: run.id.clone(),
            visit_id: visits[2].id.clone(),
            workflow: "Example review workflow".into(),
            step: "Review".into(),
            visit: 1,
            role: MarkRole::Reminder,
            outcome: None,
        }
    );
    assert!(visits[2].reminded);
    assert_eq!(run.status, RunStatus::Running);

    finish_turn(&h, &review_pane).await;
    let (run, _) = run_of(&h, &review);
    assert_eq!(
        (run.status, run.pause.clone()),
        (RunStatus::Paused, Some(PauseReason::NoResult { step_id: "s3".into() }))
    );
    assert_eq!(reminders(&h, &review).len(), 1);

    assert_eq!(
        refusal(h.engine.pick_outcome(&run.id, Some("maybe".into())).unwrap_err()),
        Refusal::WorkflowOutcomeUnknown("maybe".into())
    );
    h.engine.pick_outcome(&run.id, Some("approved".into())).unwrap();
    settle().await;
    let chat = h.engine.session_messages(&review.id).unwrap();
    let result = chat
        .iter()
        .find(|m| m.workflow_mark.as_ref().is_some_and(|mark| mark.role == MarkRole::Result))
        .expect("the result is in Review's chat");
    assert_eq!(result.body, "working on it");
    assert_eq!(
        (result.sender.as_str(), result.recipient.as_str(), result.sender_session.as_deref(), result.status),
        (t.review.id.as_str(), USER, Some(review.id.as_str()), MessageStatus::Delivered)
    );
    assert_eq!(result.workflow_mark.as_ref().unwrap().outcome.as_deref(), Some("approved"));
    let (run, visits) = run_of(&h, &review);
    assert_eq!((run.status, run.pause), (RunStatus::Running, None));
    assert_eq!(
        (visits[2].outcome.as_deref(), visits[2].result_message.as_deref()),
        (Some("approved"), Some(result.id.as_str()))
    );
    let prompt = prompt_of(&h, &only_session(&h, &t.testing));
    assert!(prompt.contains("· step \"Test\" · visit 1 ·"), "{prompt}");
    assert!(
        prompt.contains("--- Step \"Review\" (@review), visit 1, outcome \"approved\":\nworking on it"),
        "{prompt}"
    );
}

#[tokio::test]
async fn a_question_waits_without_pausing() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (_, review) = to_review(&h, &t).await;
    let review_pane = pane(&h, &review);
    assert!(h.engine.handle_bridge(ask(&review_pane, "Which header?")).await.ok);
    finish_turn(&h, &review_pane).await;
    assert!(reminders(&h, &review).is_empty());
    let (run, visits) = run_of(&h, &review);
    assert_eq!((run.status, visits[2].reminded), (RunStatus::Running, false));
    assert_eq!(refusal(h.engine.remind_step(&run.id).unwrap_err()), Refusal::WorkflowRunNotPaused);
    assert_eq!(
        refusal(h.engine.pick_outcome(&run.id, Some("approved".into())).unwrap_err()),
        Refusal::WorkflowRunNotPaused
    );
}

#[tokio::test]
async fn a_step_that_delegated_is_not_reminded_while_it_waits() {
    let h = harness_after(|_| {});
    let feature = h.engine.create_bot(claude_bot("Feature")).unwrap();
    let query = h.engine.create_bot(claude_bot("Query")).unwrap();
    let wf = h.engine.create_workflow(flow(&[&feature])).unwrap();
    let root = h.engine.start_workflow(&wf.id, "TASK-1 build the export", &[]).unwrap();
    settle().await;
    let step_pane = pane(&h, &root);
    assert!(h.engine.handle_bridge(send(&step_pane, "@query", "facts?")).await.ok);
    finish_turn(&h, &step_pane).await;
    assert!(reminders(&h, &root).is_empty(), "@query owes the step an answer");

    let query_pane = pane(&h, &only_session(&h, &query));
    assert!(h.engine.handle_bridge(reply(&query_pane, "Here are the facts.")).await.ok);
    finish_turn(&h, &query_pane).await;
    assert!(prompt_of(&h, &root).contains("Here are the facts."));
    finish_turn(&h, &step_pane).await;
    assert_eq!(reminders(&h, &root).len(), 1);
    assert!(prompt_of(&h, &root).contains(&reminder_body("Step 1", &[])));
}

#[tokio::test]
async fn a_stopped_turn_is_not_reminded() {
    let h = harness_after(|_| {});
    let (_, _, wf) = plan_and_feature(&h);
    let root = h.engine.start_workflow(&wf.id, "TASK-1 build the export", &[]).unwrap();
    settle().await;
    h.engine.stop_turn(&root.id).await.unwrap();
    finish_turn(&h, &pane(&h, &root)).await;
    assert!(reminders(&h, &root).is_empty());
    let (run, visits) = run_of(&h, &root);
    assert_eq!((run.status, visits[0].reminded), (RunStatus::Running, false));
}

#[tokio::test]
async fn a_step_routing_to_its_own_bot_is_not_reminded() {
    let h = harness_after(|_| {});
    let feature = h.engine.create_bot(claude_bot("Feature")).unwrap();
    let wf = h.engine.create_workflow(flow(&[&feature, &feature])).unwrap();
    let root = h.engine.start_workflow(&wf.id, "TASK-1 build the export", &[]).unwrap();
    settle().await;
    let root_pane = pane(&h, &root);
    assert!(h.engine.handle_bridge(result(&root_pane, "Step A done.")).await.ok);
    finish_turn(&h, &root_pane).await;
    assert!(reminders(&h, &root).is_empty());
    let prompt = prompt_of(&h, &root);
    assert!(prompt.starts_with("[Workflow \"Flow\" · step \"Step 2\" · visit 1 ·"), "{prompt}");
    let (run, visits) = run_of(&h, &root);
    assert_eq!((run.status, visits.len(), visits[1].reminded), (RunStatus::Running, 2, false));
}

#[tokio::test]
async fn remind_step_sends_another_reminder_and_resumes() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (_, review) = to_a_result_pause(&h, &t).await;
    let review_pane = pane(&h, &review);
    let (run, _) = run_of(&h, &review);
    assert_eq!(run.pause, Some(PauseReason::NoResult { step_id: "s3".into() }));
    assert_eq!(refusal(h.engine.remind_step("01JNORUN").unwrap_err()), Refusal::WorkflowRunNotFound("01JNORUN".into()));
    assert_eq!(refusal(h.engine.another_round(&run.id).unwrap_err()), Refusal::WorkflowRunNotPaused);
    let prompts = prompts_of(&h, &review).len();

    h.engine.remind_step(&run.id).unwrap();
    settle().await;
    let (run, visits) = run_of(&h, &review);
    assert_eq!((run.status, run.pause), (RunStatus::Running, None));
    assert_eq!(reminders(&h, &review).len(), 2);
    assert_eq!(prompts_of(&h, &review).len(), prompts + 1);
    let prompt = prompt_of(&h, &review);
    assert!(prompt.contains("· step \"Review\" · visit 1 ·"), "{prompt}");
    assert!(prompt.contains(&reminder_body("Review", &review_outcomes())), "{prompt}");
    assert!(visits[2].ended_at.is_none());

    finish_turn(&h, &review_pane).await;
    let (run, _) = run_of(&h, &review);
    assert_eq!((run.status, run.pause), (RunStatus::Paused, Some(PauseReason::NoResult { step_id: "s3".into() })));
    assert_eq!(reminders(&h, &review).len(), 2);
}

#[tokio::test]
async fn stop_run_stops_the_step_and_cancels_its_queue() {
    let h = harness_after(|_| {});
    let feature = h.engine.create_bot(claude_bot("Feature")).unwrap();
    let wf = h.engine.create_workflow(flow(&[&feature, &feature])).unwrap();
    let root = h.engine.start_workflow(&wf.id, "TASK-1 build the export", &[]).unwrap();
    settle().await;
    let root_pane = pane(&h, &root);
    assert!(h.engine.handle_bridge(result(&root_pane, "Step A done.")).await.ok);
    let (run, visits) = run_of(&h, &root);
    let step_two = visits[1].request_message.clone();
    let mut ui = h.engine.subscribe_ui();

    h.engine.stop_run(&run.id).await.unwrap();
    let (run, visits) = run_of(&h, &root);
    assert_eq!((run.status, run.pause.as_ref()), (RunStatus::Stopped, None));
    assert!(run.ended_at.is_some());
    assert!(visits.iter().all(|v| v.ended_at.is_some()), "{visits:?}");
    assert_eq!((visits[1].outcome.as_deref(), visits[1].result_message.as_deref()), (None, None));
    let chat = h.engine.session_messages(&root.id).unwrap();
    let cancelled = chat.iter().find(|m| m.id == step_two).expect("step 2's request");
    assert_eq!(cancelled.status, MessageStatus::Cancelled);
    assert!(events(&mut ui).iter().any(|e| matches!(e, UiEvent::MessageUpdated { message } if message.id == step_two)));
    assert!(h.rt.state().keys.contains(&(root_pane.clone(), vec!["esc".to_string()])), "the turn was stopped");

    finish_turn(&h, &root_pane).await;
    assert!(reminders(&h, &root).is_empty());
    assert!(prompt_of(&h, &root).contains("· step \"Step 1\" ·"));

    for refused in
        [h.engine.another_round(&run.id), h.engine.pick_outcome(&run.id, None), h.engine.remind_step(&run.id)]
    {
        assert_eq!(refusal(refused.unwrap_err()), Refusal::WorkflowRunNotPaused);
    }
    h.engine.stop_run(&run.id).await.unwrap();
    assert_eq!(run_of(&h, &root).0, run);
}

async fn wakes_up(h: &Harness, pane: &str) {
    h.rt.state().state = AgentState::Working;
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: pane.into(), state: AgentState::Working }).await;
    settle().await;
}

async fn settles(h: &Harness, pane: &str, screen: &str) {
    h.rt.state().screen = screen.into();
    h.rt.state().state = AgentState::Idle;
    h.engine.on_runtime_event(RuntimeEvent::State { pane_id: pane.into(), state: AgentState::Idle }).await;
    settle().await;
}

#[tokio::test]
async fn a_step_is_left_alone_while_its_screen_shows_background_work() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (_, review) = to_review(&h, &t).await;
    let review_pane = pane(&h, &review);

    assert!(h.engine.handle_bridge(reply(&review_pane, "Tests are running.")).await.ok);
    h.rt.state().screen = CLAUDE_SHELL_RUNNING.into();
    finish_turn(&h, &review_pane).await;
    assert!(reminders(&h, &review).is_empty());
    let (run, visits) = run_of(&h, &review);
    assert_eq!((run.status, visits[2].reminded), (RunStatus::Running, false));

    wakes_up(&h, &review_pane).await;
    assert!(h.engine.handle_bridge(reply(&review_pane, "Still running.")).await.ok);
    settles(&h, &review_pane, CLAUDE_SHELL_RUNNING).await;
    assert!(reminders(&h, &review).is_empty());
    assert_eq!(run_of(&h, &review).0.status, RunStatus::Running);

    wakes_up(&h, &review_pane).await;
    settles(&h, &review_pane, CLAUDE_NOTHING_RUNNING).await;
    assert_eq!(reminders(&h, &review).len(), 1);
    assert!(prompt_of(&h, &review).contains(&reminder_body("Review", &review_outcomes())));
    let (run, visits) = run_of(&h, &review);
    assert_eq!((run.status, visits[2].reminded), (RunStatus::Running, true));
}

const STILL_WORKING: &str = "You are still working on step";

#[tokio::test]
async fn mark_done_stops_the_run_and_restore_does_not_resume_it() {
    let h = harness_after(|_| {});
    let (_, feature, wf) = plan_and_feature(&h);
    let root = h.engine.start_workflow(&wf.id, "TASK-1 build the export", &[]).unwrap();
    settle().await;
    assert!(hand_in(&h, &root, "The plan.", None).await.ok);
    let step_two = only_session(&h, &feature);

    let archived = h.engine.archive_session(&root.id).await.unwrap();
    assert_eq!(archived.len(), 2);
    let (run, visits) = run_of(&h, &root);
    assert_eq!((run.status, run.pause.as_ref()), (RunStatus::Stopped, None));
    assert!(run.ended_at.is_some());
    assert!(visits.iter().all(|v| v.ended_at.is_some()), "{visits:?}");
    assert_eq!(
        (visits[1].session_id.as_deref(), visits[1].result_message.as_deref()),
        (Some(step_two.id.as_str()), None)
    );

    let restored = h.engine.restore_session(&root.id).await.unwrap();
    assert_eq!(restored.len(), 2);
    assert_eq!(run_of(&h, &root).0, run);
}

#[tokio::test]
async fn archiving_one_of_a_runs_sessions_stops_the_run() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (root, review) = to_review(&h, &t).await;
    let feature = only_session(&h, &t.feature);

    h.engine.archive_session(&feature.id).await.unwrap();
    let (run, visits) = run_of(&h, &root);
    assert_eq!(run.status, RunStatus::Stopped);
    assert!(visits.iter().all(|v| v.ended_at.is_some()), "{visits:?}");
    let review_pane = pane(&h, &review);
    assert!(h.rt.state().keys.contains(&(review_pane.clone(), vec!["esc".to_string()])), "Review's turn was stopped");
    assert!(only_session(&h, &t.review).archived_at.is_none(), "only Implement's session is archived");

    finish_turn(&h, &review_pane).await;
    assert!(reminders(&h, &review).is_empty());
    let response = h.engine.handle_bridge(result_with(&review_pane, "LGTM", "approved")).await;
    assert_eq!(refused(response), (ErrorCode::BadRequest, NOT_A_STEP.into()));
    let response = h.engine.handle_bridge(result(&review_pane, "LGTM")).await;
    assert_eq!(refused(response), (ErrorCode::NoOpenRequest, NO_OPENER.into()));

    h.engine.restore_session(&feature.id).await.unwrap();
    assert_eq!(run_of(&h, &root).0, run);
}

#[tokio::test]
async fn removing_a_run_session_stops_the_run() {
    let h = harness_after(|_| {});
    let (_, feature, wf) = plan_and_feature(&h);
    let root = h.engine.start_workflow(&wf.id, "TASK-1 build the export", &[]).unwrap();
    settle().await;
    assert!(hand_in(&h, &root, "The plan.", None).await.ok);
    let step_two = only_session(&h, &feature);

    h.engine.remove_session(&step_two.id).await.unwrap();
    assert!(h.engine.sessions_of(&feature.id).unwrap().is_empty());
    let (run, visits) = run_of(&h, &root);
    assert_eq!((run.status, run.pause.as_ref()), (RunStatus::Stopped, None));
    assert!(run.ended_at.is_some());
    assert_eq!(visits.len(), 2);
    assert!(visits.iter().all(|v| v.ended_at.is_some()), "{visits:?}");
    assert_eq!((visits[1].session_id.as_deref(), visits[1].result_message.as_deref()), (None, None));
    assert_eq!(run.root_session.as_deref(), Some(root.id.as_str()));
}

#[tokio::test]
async fn deleting_the_open_steps_bot_pauses_the_run() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (root, _) = to_review(&h, &t).await;

    h.engine.delete_bot(&t.review.id).await.unwrap();
    let (run, visits) = run_of(&h, &root);
    assert_eq!(
        (run.status, run.pause.clone()),
        (RunStatus::Paused, Some(PauseReason::BotMissing { step_id: "s3".into() }))
    );
    let open = &visits[2];
    assert_eq!((open.step_id.as_str(), open.session_id.as_deref(), open.ended_at.as_deref()), ("s3", None, None));

    for refused in [
        h.engine.another_round(&run.id),
        h.engine.pick_outcome(&run.id, Some("approved".into())),
        h.engine.remind_step(&run.id),
    ] {
        assert_eq!(refusal(refused.unwrap_err()), Refusal::WorkflowRunNotPaused);
    }
    h.engine.stop_run(&run.id).await.unwrap();
    let (run, visits) = run_of(&h, &root);
    assert_eq!(run.status, RunStatus::Stopped);
    assert!(visits.iter().all(|v| v.ended_at.is_some()), "{visits:?}");
}

#[tokio::test]
async fn deleting_the_bot_of_a_step_paused_for_its_result_leaves_only_stop() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (root, _) = to_a_result_pause(&h, &t).await;

    h.engine.delete_bot(&t.review.id).await.unwrap();
    let (run, _) = run_of(&h, &root);
    assert_eq!(
        (run.status, run.pause.clone()),
        (RunStatus::Paused, Some(PauseReason::BotMissing { step_id: "s3".into() }))
    );
    assert_eq!(
        refusal(h.engine.pick_outcome(&run.id, Some("approved".into())).unwrap_err()),
        Refusal::WorkflowRunNotPaused
    );
}

#[tokio::test]
async fn an_edit_during_a_run_does_not_reach_it() {
    let h = harness_after(|_| {});
    let (plan, feature, wf) = plan_and_feature(&h);
    let root = h.engine.start_workflow(&wf.id, "TASK-1 build the export", &[]).unwrap();
    settle().await;
    let mut edit = flow(&[&plan, &feature]);
    edit.def.steps[1].instructions = "Do it another way.".into();
    h.engine.update_workflow(&wf.id, edit).unwrap();

    assert!(hand_in(&h, &root, "The plan.", None).await.ok);
    let prompt = prompt_of(&h, &only_session(&h, &feature));
    assert!(prompt.contains("Your step:\nDo step 2."), "{prompt}");
    assert!(!prompt.contains("Do it another way."), "{prompt}");
}

#[tokio::test]
async fn a_run_goes_on_after_a_relaunch() {
    let h = harness_after(|_| {});
    let (_, feature, wf) = plan_and_feature(&h);
    let root = h.engine.start_workflow(&wf.id, "TASK-1 build the export", &[]).unwrap();
    settle().await;
    let request = first_message(&h, &root);

    let again = relaunch(&h);
    again.engine.recover().await.unwrap();
    assert_eq!(stored(&again, &request), MessageStatus::Interrupted);
    again.engine.retry_message(&request.id).unwrap();
    again.engine.pump_session(&root.id).await;
    let root_pane = pane(&again, &root);
    assert!(prompt_of(&again, &root).starts_with("[Workflow \"Flow\" · step \"Step 1\" · visit 1 ·"));

    let response = again.engine.handle_bridge(result(&root_pane, "The plan.")).await;
    assert_eq!(
        response.output.as_deref(),
        Some(
            "Result recorded for step \"Step 1\". The workflow moves on to step \"Step 2\" (@feature). End your turn."
        )
    );
    settle().await;
    let prompt = prompt_of(&again, &only_session(&again, &feature));
    assert!(prompt.starts_with("[Workflow \"Flow\" · step \"Step 2\" · visit 1 ·"), "{prompt}");
    assert!(prompt.contains("--- Step \"Step 1\" (@plan), visit 1:\nThe plan."), "{prompt}");
}

#[tokio::test]
async fn a_users_message_in_an_open_step_carries_the_step_line() {
    let h = harness_after(|_| {});
    let (_, _, wf) = plan_and_feature(&h);
    let root = h.engine.start_workflow(&wf.id, "TASK-1 build the export", &[]).unwrap();
    settle().await;
    let root_pane = pane(&h, &root);
    assert!(!prompt_of(&h, &root).contains(STILL_WORKING), "the step's request says it itself");

    h.engine.queue_user_message_to_session(&root.id, "Use the v2 API.", &[]).unwrap();
    finish_turn(&h, &root_pane).await;
    let prompt = prompt_of(&h, &root);
    assert!(
        prompt.starts_with("[Message from user · thread you↔user · language: English]\nUse the v2 API."),
        "{prompt}"
    );
    assert!(prompt.ends_with(&format!("\n\n{}", open_step_line("Flow", "Step 1", &[]))), "{prompt}");
    assert!(reminders(&h, &root).is_empty(), "the user's message was waiting");

    finish_turn(&h, &root_pane).await;
    assert_eq!(reminders(&h, &root).len(), 1);
    let prompt = prompt_of(&h, &root);
    assert!(prompt.contains(&reminder_body("Step 1", &[])), "{prompt}");
    assert!(!prompt.contains(STILL_WORKING), "{prompt}");
}

#[tokio::test]
async fn a_bots_message_in_an_open_step_carries_the_step_line() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let helper = h.engine.create_bot(claude_bot("Helper")).unwrap();
    let (_, review) = to_review(&h, &t).await;
    let review_pane = pane(&h, &review);
    assert!(h.engine.handle_bridge(send(&review_pane, "@helper", "Check the numbers.")).await.ok);
    finish_turn(&h, &review_pane).await;

    let helper_session = only_session(&h, &helper);
    let helper_pane = pane(&h, &helper_session);
    assert!(!prompt_of(&h, &helper_session).contains(STILL_WORKING), "@helper has no step open");
    assert!(h.engine.handle_bridge(reply(&helper_pane, "The numbers add up.")).await.ok);
    finish_turn(&h, &helper_pane).await;
    let prompt = prompt_of(&h, &review);
    assert!(prompt.starts_with("[Message from @helper · thread you↔@helper]\nThe numbers add up."), "{prompt}");
    assert!(
        prompt.ends_with(&format!("\n\n{}", open_step_line("Example review workflow", "Review", &review_outcomes()))),
        "{prompt}"
    );
}

#[tokio::test]
async fn the_opener_reaches_the_runs_session_of_a_bot() {
    let h = harness_after(|_| {});
    let (_, feature, wf) = plan_and_feature(&h);
    let root = h.engine.start_workflow(&wf.id, "TASK-1 build the export", &[]).unwrap();
    settle().await;
    assert!(hand_in(&h, &root, "The plan.", None).await.ok);
    let step_two = only_session(&h, &feature);

    let response = h.engine.handle_bridge(send(&pane(&h, &root), "@feature", "Mind the CSV export too.")).await;
    assert!(response.ok, "{response:?}");
    assert_eq!(only_session(&h, &feature).id, step_two.id, "no new session");

    finish_turn(&h, &pane(&h, &step_two)).await;
    let prompt = prompt_of(&h, &step_two);
    assert!(prompt.starts_with("[Message from @plan · thread you↔@plan]\nMind the CSV export too."), "{prompt}");
    assert!(prompt.ends_with(&open_step_line("Flow", "Step 2", &[])), "{prompt}");
}

const NO_OPENER: &str =
    "No bot is waiting for a result from this conversation. Answer with `sbot reply`, or message someone with \
     `sbot send`.";

#[tokio::test]
async fn a_result_outside_a_step_of_a_users_run_names_no_opener() {
    let h = harness_after(|_| {});
    let (_, feature, wf) = plan_and_feature(&h);
    let root = h.engine.start_workflow(&wf.id, "TASK-1 build the export", &[]).unwrap();
    settle().await;
    assert!(hand_in(&h, &root, "The plan.", None).await.ok);
    let step_two = only_session(&h, &feature);
    assert!(hand_in(&h, &step_two, "Built it.", None).await.ok);
    assert_eq!(run_of(&h, &root).0.status, RunStatus::Finished);

    let response = h.engine.handle_bridge(result(&pane(&h, &step_two), "Built it again.")).await;
    assert_eq!(refused(response), (ErrorCode::NoOpenRequest, NO_OPENER.into()));
}

#[tokio::test]
async fn a_result_outside_a_step_of_a_bots_run_names_the_bot() {
    let mut plan_id = String::new();
    let h = harness_after(|store| plan_id = master_run(store).0.id);
    h.engine.pump(&plan_id).await;
    let session = h.engine.sessions_of(&plan_id).unwrap().remove(0);
    assert!(hand_in(&h, &session, "The plan.", None).await.ok);
    assert_eq!(run_of(&h, &session).0.status, RunStatus::Finished);

    let response = h.engine.handle_bridge(result(&pane(&h, &session), "The plan again.")).await;
    assert_eq!(
        refused(response),
        (
            ErrorCode::NoOpenRequest,
            "No bot is waiting for a result from this conversation. Send your result to @master, who opened this \
             conversation, with `sbot send @master`."
                .into()
        )
    );
}

async fn activity_of(h: &Harness, session: &Session) -> SessionActivity {
    let mut all = h.engine.session_activity(std::slice::from_ref(&session.id)).await.unwrap();
    assert_eq!(all.len(), 1);
    all.remove(0)
}

#[tokio::test]
async fn a_step_sessions_activity_shows_its_run() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (root, review) = to_review(&h, &t).await;
    assert!(hand_in(&h, &review, "Fix it.", Some("changes requested")).await.ok);
    assert!(hand_in(&h, &only_session(&h, &t.feature), "Fixed.", None).await.ok);

    let (run, _) = run_of(&h, &review);
    for session in [&review, &root] {
        let glance = activity_of(&h, session).await.workflow.expect("the run's glance");
        assert_eq!(glance.run_id, run.id);
        assert_eq!(glance.name, "Example review workflow");
        assert_eq!(glance.status, RunStatus::Running);
        assert_eq!(glance.step.as_deref(), Some("Review"));
        assert_eq!(glance.handle.as_deref(), Some("review"));
        assert_eq!(glance.visit, 2);
        assert_eq!((glance.pause, glance.holder), (None, None));
    }
}

#[tokio::test]
async fn a_paused_runs_activity_says_why_and_who_holds_it() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (_, review) = to_a_result_pause(&h, &t).await;
    let glance = activity_of(&h, &review).await.workflow.expect("the run's glance");
    assert_eq!(glance.status, RunStatus::Paused);
    assert_eq!(glance.pause, Some(PauseReason::NoResult { step_id: "s3".into() }));
    assert_eq!((glance.step.as_deref(), glance.visit), (Some("Review"), 1));
    assert_eq!(glance.outcomes, review_outcomes());
    assert_eq!(glance.holder.as_deref(), Some(review.id.as_str()));

    h.engine.stop_run(&glance.run_id).await.unwrap();
    let glance = activity_of(&h, &review).await.workflow.expect("the run's glance");
    assert_eq!((glance.status, glance.pause, glance.step, glance.holder), (RunStatus::Stopped, None, None, None));
}

#[tokio::test]
async fn a_visit_cap_pause_is_held_by_the_last_visits_session() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (_, review) = to_review(&h, &t).await;
    let feature = only_session(&h, &t.feature);
    for round in 1..=2 {
        assert!(hand_in(&h, &review, &format!("Fix round {round}."), Some("changes requested")).await.ok);
        assert!(hand_in(&h, &feature, &format!("Fixed round {round}."), None).await.ok);
    }
    assert!(hand_in(&h, &review, "Still wrong.", Some("changes requested")).await.ok);
    let glance = activity_of(&h, &feature).await.workflow.expect("the run's glance");
    assert_eq!(glance.pause, Some(PauseReason::VisitCap { step_id: "s2".into() }));
    assert_eq!((glance.step.as_deref(), glance.visit), (Some("Implement"), 3));
    assert_eq!(glance.holder.as_deref(), Some(review.id.as_str()));
}

type Change = (String, RunStatus, Option<PauseReason>, Option<String>, Vec<String>);

fn changes(events: Vec<UiEvent>) -> Vec<Change> {
    events
        .into_iter()
        .filter_map(|e| match e {
            UiEvent::WorkflowRunChanged { run_id, status, pause, step, mut session_ids, .. } => {
                session_ids.sort();
                Some((run_id, status, pause, step, session_ids))
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn every_move_tells_the_ui() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let mut rx = h.engine.subscribe_ui();
    let root = h.engine.start_workflow(&t.wf.id, "TASK-1 build the export", &[]).unwrap();
    settle().await;
    let (run, _) = run_of(&h, &root);
    let started = changes(events(&mut rx));
    assert_eq!(
        started,
        vec![(run.id.clone(), RunStatus::Running, None, Some("Plan".to_string()), vec![root.id.clone()])]
    );

    assert!(hand_in(&h, &root, "The plan.", None).await.ok);
    let feature = only_session(&h, &t.feature);
    let mut both = vec![root.id.clone(), feature.id.clone()];
    both.sort();
    assert_eq!(
        changes(events(&mut rx)),
        vec![(run.id.clone(), RunStatus::Running, None, Some("Implement".to_string()), both)]
    );

    h.engine.stop_run(&run.id).await.unwrap();
    let stopped = changes(events(&mut rx));
    assert_eq!(stopped.len(), 1);
    assert_eq!((stopped[0].1, stopped[0].2.clone(), stopped[0].3.clone()), (RunStatus::Stopped, None, None));
}

#[tokio::test]
async fn pausing_and_resuming_a_run_tell_the_ui() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (_, review) = to_review(&h, &t).await;
    let mut rx = h.engine.subscribe_ui();
    let review_pane = pane(&h, &review);
    finish_turn(&h, &review_pane).await;
    finish_turn(&h, &review_pane).await;
    let seen = changes(events(&mut rx));
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(
        (seen[0].1, seen[0].2.clone()),
        (RunStatus::Paused, Some(PauseReason::NoResult { step_id: "s3".into() }))
    );
    let (run, _) = run_of(&h, &review);
    h.engine.remind_step(&run.id).unwrap();
    let seen = changes(events(&mut rx));
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(
        (seen[0].1, seen[0].2.clone(), seen[0].3.clone()),
        (RunStatus::Running, None, Some("Review".to_string()))
    );
}

#[tokio::test]
async fn get_workflow_run_lists_the_visits_in_order() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (_, review) = to_review(&h, &t).await;
    let (run, _) = run_of(&h, &review);
    let view = h.engine.get_workflow_run(&run.id).unwrap();
    assert_eq!(view.run.id, run.id);
    let steps: Vec<(&str, u32)> = view.visits.iter().map(|v| (v.step_id.as_str(), v.n)).collect();
    assert_eq!(steps, vec![("s1", 1), ("s2", 1), ("s3", 1)]);
    assert!(view.visits.windows(2).all(|w| w[0].ord < w[1].ord));
    assert_eq!(
        refusal(h.engine.get_workflow_run("01JNORUN").unwrap_err()),
        Refusal::WorkflowRunNotFound("01JNORUN".into())
    );
}

#[tokio::test]
async fn a_run_already_paused_for_a_missing_bot_is_not_announced_again() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let other = h.engine.create_bot(claude_bot("Other")).unwrap();
    let (root, _) = to_review(&h, &t).await;
    let mut rx = h.engine.subscribe_ui();
    h.engine.delete_bot(&t.review.id).await.unwrap();
    let (run, _) = run_of(&h, &root);
    let seen = changes(events(&mut rx));
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].0, run.id);
    assert_eq!(seen[0].2, Some(PauseReason::BotMissing { step_id: "s3".into() }));

    h.engine.delete_bot(&other.id).await.unwrap();
    assert_eq!(changes(events(&mut rx)), vec![]);
    let (again, _) = run_of(&h, &root);
    assert_eq!(again, run);
}

async fn bot_named(h: &Harness, handle: &str) -> Bot {
    h.engine.list_bots().await.unwrap().into_iter().find(|v| v.bot.handle == handle).unwrap().bot
}

async fn masters_turn(h: &Harness) -> (Session, String) {
    setup_team(h, None);
    let master = bot_named(h, "master").await;
    let (session, _) = h.engine.start_session_with_message(&master.id, "Ship TASK-1.", &[]).unwrap();
    h.engine.pump_session(&session.id).await;
    let pane = pane(h, &session);
    (session, pane)
}

fn drop_seeded_workflow(h: &Harness) {
    let seeded = h.engine.list_workflows().unwrap().into_iter().find(|w| w.name == "Example review workflow");
    h.engine.delete_workflow(&seeded.expect("setup brings it").id).unwrap();
}

async fn sbot(h: &Harness, pane: &str, command: BridgeCommand) -> BridgeResponse {
    h.engine.handle_bridge(request(pane, command)).await
}

fn output(response: BridgeResponse) -> String {
    response.output.unwrap_or_else(|| panic!("no output: {:?}", response.error))
}

fn workflow_run(name: &str, body: &str, files: Vec<String>) -> BridgeCommand {
    BridgeCommand::WorkflowRun { name: name.into(), body: body.into(), files }
}

fn workflow_create(definition: &str, recreate: bool) -> BridgeCommand {
    BridgeCommand::WorkflowCreate { definition: definition.into(), recreate }
}

fn workflow_show(name: &str) -> BridgeCommand {
    BridgeCommand::WorkflowShow { name: name.into() }
}

fn last_notice(h: &Harness, session: &Session, code: &str) -> serde_json::Value {
    h.engine
        .session_messages(&session.id)
        .unwrap()
        .into_iter()
        .rev()
        .find(|m| m.system_code.as_deref() == Some(code))
        .and_then(|m| m.system_params)
        .unwrap_or_else(|| panic!("no {code} notice"))
}

async fn read_text(h: &Harness, text: &str, keep: Option<&WorkflowDef>) -> NewWorkflow {
    let bots: Vec<Bot> = h.engine.list_bots().await.unwrap().into_iter().map(|v| v.bot).collect();
    parse_text(text, |handle| bots.iter().find(|b| b.handle == handle).map(|b| b.id.clone()), keep).unwrap()
}

const SHIP: &str = "Workflow: Ship\nWhen: A ticket to build.\n\n## Plan\nBot: @plan\n-> Build\n\nPlan the ticket.\n\n\
                    ## Build\nBot: @feature\n\nBuild it.\n";

const NO_SUCH_WORKFLOW: &str = "No workflow named Nope; run `sbot workflows`.";

#[tokio::test]
async fn master_lists_and_shows_workflows() {
    let h = harness();
    let (_, mp) = masters_turn(&h).await;
    assert_eq!(
        output(sbot(&h, &mp, BridgeCommand::Workflows).await),
        [
            "Example review workflow: A request to implement and review.",
            "  Plan (@plan): -> Implement",
            "  Implement (@feature): -> Review",
            "  Review (@review): approved -> Test; changes requested -> Implement",
            "  Test (@testing): passed -> Open MR; failed -> Implement",
            "  Open MR (@feature): -> Done",
        ]
        .join("\n")
    );
    let seeded = h.engine.list_workflows().unwrap().remove(0);
    let shown = output(sbot(&h, &mp, workflow_show("Example review workflow")).await);
    assert_eq!(shown, TICKET_WORKFLOW);
    let read = read_text(&h, &shown, None).await;
    assert_eq!((read.name, read.when_to_use, read.def), (seeded.name, seeded.when_to_use, seeded.def));
    h.engine.delete_workflow(&seeded.id).unwrap();
    assert_eq!(output(sbot(&h, &mp, BridgeCommand::Workflows).await), "No workflows yet.");

    let [plan, feature, review, testing] = [
        bot_named(&h, "plan").await,
        bot_named(&h, "feature").await,
        bot_named(&h, "review").await,
        bot_named(&h, "testing").await,
    ];
    let mut ticket = ticket_flow(&plan, &feature, &review, &testing);
    ticket.when_to_use = "A request to build, review and test.".into();
    h.engine.create_workflow(ticket.clone()).unwrap();
    let mut solo = flow(&[&plan]);
    solo.name = "Solo".into();
    solo.def.steps[0].arrows.clear();
    h.engine.create_workflow(solo).unwrap();

    assert_eq!(
        output(sbot(&h, &mp, BridgeCommand::Workflows).await),
        [
            "Example review workflow: A request to build, review and test.",
            "  Plan (@plan): -> Implement",
            "  Implement (@feature): -> Review",
            "  Review (@review): approved -> Test; changes requested -> Implement",
            "  Test (@testing): passed -> Done; failed -> Implement",
            "Solo",
            "  Step 1 (@plan): -> Done",
        ]
        .join("\n")
    );

    let shown = output(sbot(&h, &mp, workflow_show(" example review workflow ")).await);
    assert!(
        shown.starts_with(
            "Workflow: Example review workflow\nWhen: A request to build, review and test.\n\n## Plan\nBot: @plan\n\
             -> Implement\n"
        ),
        "{shown}"
    );
    assert_eq!(read_text(&h, &shown, None).await, ticket);

    let e = sbot(&h, &mp, workflow_show("Nope")).await.error.unwrap();
    assert_eq!((e.code, e.message.as_str()), (ErrorCode::BadRequest, NO_SUCH_WORKFLOW));
}

#[tokio::test]
async fn master_starts_a_run_inside_its_own_card() {
    let h = harness_after(|_| {});
    let (session, mp) = masters_turn(&h).await;
    let plan = bot_named(&h, "plan").await;
    let feature = bot_named(&h, "feature").await;
    h.engine.create_workflow(flow(&[&plan, &feature])).unwrap();
    let brief = h.paths.bot_dir("master").join("brief.md");
    std::fs::write(&brief, "# Brief").unwrap();

    let started =
        sbot(&h, &mp, workflow_run("flow", "TASK-1 build the export", vec![brief.to_string_lossy().into()])).await;
    assert_eq!(
        output(started),
        "Started the workflow \"Flow\". Step \"Step 1\" went to @plan. The final result arrives here as a message; \
         end your turn."
    );
    settle().await;
    let step = only_session(&h, &plan);
    assert_eq!(step.opened_by.as_deref(), Some(session.id.as_str()));
    assert_eq!(run_of(&h, &step).0.opener_session.as_deref(), Some(session.id.as_str()));
    let prompt = prompt_of(&h, &step);
    assert!(
        prompt.starts_with(
            "[Workflow \"Flow\" · step \"Step 1\" · visit 1 · started by @master · thread you↔user · language: \
             English]\nRequest:\nTASK-1 build the export\n\n"
        ),
        "{prompt}"
    );
    let visit = first_message(&h, &step);
    assert!(prompt.ends_with(&format!("\n\nAttachments:\n- `inbox/{}-brief.md`", visit.id)), "{prompt}");

    let e = sbot(&h, &mp, workflow_run("Nope", "Go.", vec![])).await.error.unwrap();
    assert_eq!((e.code, e.message.as_str()), (ErrorCode::BadRequest, NO_SUCH_WORKFLOW));
    let e = sbot(&h, &mp, workflow_run("Flow", " \n", vec![])).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::EmptyBody);
    let gone = h.paths.bot_dir("master").join("gone.md").to_string_lossy().into_owned();
    let e = sbot(&h, &mp, workflow_run("Flow", "Go.", vec![gone])).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::FileNotFound);
    assert_eq!(h.engine.sessions_of(&plan.id).unwrap().len(), 1);
}

#[tokio::test]
async fn a_master_run_reports_back_when_it_finishes() {
    let h = harness_after(|_| {});
    let (session, mp) = masters_turn(&h).await;
    let plan = bot_named(&h, "plan").await;
    let feature = bot_named(&h, "feature").await;
    let mut wf = flow(&[&plan, &feature]);
    wf.def.steps[0].arrows = vec![
        Arrow { to: "s2".into(), outcome: Some("check".into()) },
        Arrow { to: DONE.into(), outcome: Some("done".into()) },
    ];
    wf.def.steps[1].arrows = vec![arrow("s1")];
    h.engine.create_workflow(wf).unwrap();
    assert!(sbot(&h, &mp, workflow_run("Flow", "TASK-1 build the export", vec![])).await.ok);
    assert!(h.engine.handle_bridge(reply(&mp, "I started the workflow Flow.")).await.ok);
    finish_turn(&h, &mp).await;
    let told = prompts_of(&h, &session).len();

    let step_one = only_session(&h, &plan);
    let draft = h.paths.bot_dir("plan").join("draft.md");
    std::fs::write(&draft, "# Draft").unwrap();
    let first = BridgeCommand::Result {
        body: "A first plan.".into(),
        files: vec![draft.to_string_lossy().into()],
        outcome: Some("check".into()),
    };
    assert!(sbot(&h, &pane(&h, &step_one), first).await.ok);
    finish_turn(&h, &pane(&h, &step_one)).await;
    let step_two = only_session(&h, &feature);
    let notes = h.paths.bot_dir("feature").join("notes.md");
    std::fs::write(&notes, "# Notes").unwrap();
    let check = BridgeCommand::Result {
        body: "Checked: fine.".into(),
        files: vec![notes.to_string_lossy().into()],
        outcome: None,
    };
    assert!(sbot(&h, &pane(&h, &step_two), check).await.ok);
    finish_turn(&h, &pane(&h, &step_two)).await;
    assert_eq!(prompts_of(&h, &session).len(), told, "nothing reaches Master before the run finishes");
    assert!(hand_in(&h, &step_one, "The final plan.", Some("done")).await.ok);
    settle().await;

    let (run, visits) = run_of(&h, &step_one);
    assert_eq!(run.status, RunStatus::Finished);
    let finished = h
        .engine
        .session_messages(&session.id)
        .unwrap()
        .into_iter()
        .find(|m| m.workflow_mark.as_ref().is_some_and(|mark| mark.role == MarkRole::Finished))
        .expect("the finished message");
    assert_eq!((finished.sender.as_str(), finished.hop), (USER, 0));
    assert_eq!(finished.recipient_session.as_deref(), Some(session.id.as_str()));
    assert_eq!(
        finished.workflow_mark,
        Some(WorkflowMark {
            run_id: run.id.clone(),
            visit_id: visits[2].id.clone(),
            workflow: "Flow".into(),
            step: "Step 1".into(),
            visit: 2,
            role: MarkRole::Finished,
            outcome: Some("done".into()),
        })
    );
    let copy = h.paths.bot_dir("master").join("inbox").join(format!("{}-notes.md", finished.id));
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "# Notes");
    assert_eq!(finished.attachments, vec![Attachment { name: "notes.md".into(), path: copy.to_string_lossy().into() }]);

    assert_eq!(
        prompt_of(&h, &session),
        format!(
            "[Workflow \"Flow\" finished · thread you↔user · language: English]\n\
             Path:\n\
             - Step \"Step 1\" (@plan), visit 1, outcome \"check\"\n\
             - Step \"Step 2\" (@feature), visit 1\n\
             - Step \"Step 1\" (@plan), visit 2, outcome \"done\"\n\n\
             Latest results:\n\
             --- Step \"Step 2\" (@feature), visit 1:\nChecked: fine.\n\n\
             --- Step \"Step 1\" (@plan), visit 2, outcome \"done\":\nThe final plan.\n\n\
             Attachments:\n- `inbox/{}-notes.md`\n\n{}",
            finished.id,
            include_str!("../bundled/master/orchestration.md")
        )
    );
    assert!(prompt_of(&h, &session).contains("Path:\n- Step \"Step 1\" (@plan), visit 1"));
}

#[tokio::test]
async fn master_creates_a_workflow_once() {
    let h = harness();
    let (session, mp) = masters_turn(&h).await;
    drop_seeded_workflow(&h);
    let mut ui = h.engine.subscribe_ui();
    let first = output(sbot(&h, &mp, workflow_create(SHIP, false)).await);
    let saved = h.engine.list_workflows().unwrap();
    let [saved] = &saved[..] else { panic!("one workflow: {saved:?}") };
    assert_eq!((saved.name.as_str(), saved.when_to_use.as_str()), ("Ship", "A ticket to build."));
    assert_eq!(first, output(sbot(&h, &mp, workflow_show("Ship")).await));
    assert!(
        first.starts_with("Workflow: Ship\nWhen: A ticket to build.\n\n## Plan\nBot: @plan\n-> Build\n"),
        "{first}"
    );
    assert!(events(&mut ui).contains(&UiEvent::WorkflowsChanged));
    assert_eq!(last_notice(&h, &session, "master_created_workflow"), json!({"name": "Ship"}));

    assert_eq!(output(sbot(&h, &mp, workflow_create(SHIP, false)).await), format!("Already created.\n{first}"));
    assert_eq!(h.engine.list_workflows().unwrap().len(), 1);

    let other = SHIP.replace("Workflow: Ship", "Workflow: SHIP").replace("Build it.", "Build it well.");
    let e = sbot(&h, &mp, workflow_create(&other, false)).await.error.unwrap();
    assert_eq!(
        (e.code, e.message.as_str()),
        (
            ErrorCode::CreatedDifferently,
            "This conversation already created the workflow Ship with other settings. Change it with `sbot workflow \
             edit`."
        )
    );

    let deleted = output(sbot(&h, &mp, BridgeCommand::WorkflowDelete { name: "ship".into() }).await);
    assert_eq!(deleted, "Deleted the workflow Ship. Its runs' sessions stay.");
    assert_eq!(last_notice(&h, &session, "master_deleted_workflow"), json!({"name": "Ship"}));
    let e = sbot(&h, &mp, workflow_create(SHIP, false)).await.error.unwrap();
    assert_eq!(
        (e.code, e.message.as_str()),
        (
            ErrorCode::CreatedThenDeleted,
            "The workflow Ship this conversation created was deleted in the app. Ask the user; only if they want it \
             again, repeat the command with --recreate."
        )
    );
    assert!(h.engine.list_workflows().unwrap().is_empty());
    assert_eq!(output(sbot(&h, &mp, workflow_create(SHIP, true)).await), first);
    assert_eq!(h.engine.list_workflows().unwrap().len(), 1);

    for (definition, why) in [
        ("## Plan\nBot: @plan\n\nPlan.", "The definition must start with \"Workflow: <name>\"."),
        (
            "Workflow: Other\n\n## Plan\nBot: @nobody\n\nPlan.",
            "No bot named @nobody. Run `sbot bots` to see your teammates.",
        ),
        ("Workflow: Other\n\n## Plan\nBot: @plan\n", "step Plan has no instructions"),
    ] {
        let e = sbot(&h, &mp, workflow_create(definition, false)).await.error.unwrap();
        assert_eq!((e.code, e.message.as_str()), (ErrorCode::BadRequest, why), "{definition}");
    }
    assert_eq!(h.engine.list_workflows().unwrap().len(), 1);
}

#[tokio::test]
async fn master_changes_workflows_only_on_the_users_turn() {
    let h = harness();
    let (session, mp) = masters_turn(&h).await;
    drop_seeded_workflow(&h);
    output(sbot(&h, &mp, workflow_create(SHIP, false)).await);

    assert!(h.engine.handle_bridge(send(&mp, "@plan", "Plan it.")).await.ok);
    settle().await;
    let plan = bot_named(&h, "plan").await;
    let pp = pane(&h, &opened(&h, &plan, &session));
    assert!(h.engine.handle_bridge(reply(&pp, "Plan.")).await.ok);
    finish_turn(&h, &pp).await;
    finish_turn(&h, &mp).await;
    for command in [
        workflow_create(&SHIP.replace("Ship", "Other"), false),
        BridgeCommand::WorkflowEdit { name: "Ship".into(), definition: SHIP.replace("Build it.", "Build it well.") },
        BridgeCommand::WorkflowDelete { name: "Ship".into() },
    ] {
        let e = sbot(&h, &mp, command).await.error.unwrap();
        assert_eq!(
            (e.code, e.message.as_str()),
            (ErrorCode::NotAllowed, "Create, change and delete workflows only while answering the user's message.")
        );
    }
    let saved = h.engine.list_workflows().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].def.steps[1].instructions, "Build it.");

    assert!(output(sbot(&h, &mp, BridgeCommand::Workflows).await).starts_with("Ship: A ticket to build.\n"));
    assert!(output(sbot(&h, &mp, workflow_show("Ship")).await).starts_with("Workflow: Ship\n"));
    let started = output(sbot(&h, &mp, workflow_run("Ship", "TASK-2", vec![])).await);
    assert!(started.starts_with("Started the workflow \"Ship\". Step \"Plan\" went to @plan."), "{started}");
}

#[tokio::test]
async fn master_edits_keep_step_ids() {
    let h = harness();
    let (session, mp) = masters_turn(&h).await;
    drop_seeded_workflow(&h);
    let plan = bot_named(&h, "plan").await;
    let feature = bot_named(&h, "feature").await;
    let mut wf = flow(&[&plan, &feature]);
    wf.def.steps[0].pos = Some(Pos { x: 10.0, y: 20.0 });
    wf.def.steps[1].pos = Some(Pos { x: 300.0, y: 20.0 });
    wf.def.start_pos = Some(Pos { x: -100.0, y: 20.0 });
    wf.def.done_pos = Some(Pos { x: 600.0, y: 20.0 });
    let saved = h.engine.create_workflow(wf.clone()).unwrap();

    let text = "Workflow: Flow\nWhen: Two steps and a review.\n\n\
                ## Step 1\nBot: @plan\n-> Review\n\nDo step 1.\n\n\
                ## Review\nBot: @review\n-> step 2\n\nReview step 1.\n\n\
                ## step 2\nBot: @feature\n-> Done\n\nDo step 2 again.\n";
    let changed =
        output(sbot(&h, &mp, BridgeCommand::WorkflowEdit { name: "FLOW".into(), definition: text.into() }).await);
    let now = h.engine.list_workflows().unwrap().remove(0);
    assert_eq!(now.id, saved.id);
    assert_eq!(changed, output(sbot(&h, &mp, workflow_show("Flow")).await));
    assert_eq!(now.when_to_use, "Two steps and a review.");
    let steps: Vec<(&str, &str, Option<Pos>)> =
        now.def.steps.iter().map(|s| (s.id.as_str(), s.name.as_str(), s.pos.clone())).collect();
    assert_eq!(
        steps,
        vec![
            ("s1", "Step 1", Some(Pos { x: 10.0, y: 20.0 })),
            ("s3", "Review", None),
            ("s2", "step 2", Some(Pos { x: 300.0, y: 20.0 })),
        ]
    );
    assert_eq!(now.def.steps[0].arrows, vec![arrow("s3")]);
    assert_eq!(now.def.steps[1].arrows, vec![arrow("s2")]);
    assert_eq!((&now.def.start_pos, &now.def.done_pos), (&wf.def.start_pos, &wf.def.done_pos));
    assert_eq!(read_text(&h, &changed, Some(&now.def)).await.def, now.def);
    assert_eq!(last_notice(&h, &session, "master_changed_workflow"), json!({"name": "Flow"}));

    let e = sbot(&h, &mp, BridgeCommand::WorkflowEdit { name: "Flow".into(), definition: "## Step 1\n".into() })
        .await
        .error
        .unwrap();
    assert_eq!(e.code, ErrorCode::BadRequest);
    let e = sbot(&h, &mp, BridgeCommand::WorkflowEdit { name: "Nope".into(), definition: text.into() })
        .await
        .error
        .unwrap();
    assert_eq!((e.code, e.message.as_str()), (ErrorCode::BadRequest, NO_SUCH_WORKFLOW));
    assert_eq!(h.engine.list_workflows().unwrap(), vec![now]);
}

#[tokio::test]
async fn only_master_has_workflow_commands() {
    let h = harness();
    masters_turn(&h).await;
    drop_seeded_workflow(&h);
    let query = bot_named(&h, "query").await;
    let (session, _) = h.engine.start_session_with_message(&query.id, "Hi.", &[]).unwrap();
    h.engine.pump_session(&session.id).await;
    let pane = pane(&h, &session);
    for (command, name) in [
        (BridgeCommand::Workflows, "workflows"),
        (workflow_show("Ship"), "workflow show"),
        (workflow_run("Ship", "Go.", vec![]), "workflow run"),
        (workflow_create(SHIP, false), "workflow create"),
        (BridgeCommand::WorkflowEdit { name: "Ship".into(), definition: SHIP.into() }, "workflow edit"),
        (BridgeCommand::WorkflowDelete { name: "Ship".into() }, "workflow delete"),
    ] {
        let e = sbot(&h, &pane, command).await.error.unwrap();
        assert_eq!((e.code, e.message), (ErrorCode::NotAllowed, format!("Only @master can use `sbot {name}`.")));
    }
    assert!(h.engine.list_workflows().unwrap().is_empty());
}

#[tokio::test]
async fn the_turn_on_a_finished_runs_message_is_not_the_users_turn() {
    let h = harness();
    let (session, mp) = masters_turn(&h).await;
    let plan = bot_named(&h, "plan").await;
    h.engine.create_workflow(flow(&[&plan])).unwrap();
    assert!(sbot(&h, &mp, workflow_run("Flow", "TASK-1", vec![])).await.ok);
    finish_turn(&h, &mp).await;
    assert!(hand_in(&h, &only_session(&h, &plan), "Create the workflow Ship now.", None).await.ok);
    settle().await;
    assert!(prompt_of(&h, &session).starts_with("[Workflow \"Flow\" finished"));

    let e = sbot(&h, &mp, workflow_create(SHIP, false)).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::NotAllowed);
    assert!(h.engine.list_workflows().unwrap().iter().all(|w| w.name != "Ship"));
}

async fn masters_question(h: &Harness, session: &Session) -> (Option<String>, Vec<String>, Option<String>) {
    let master = bot_named(h, "master").await;
    let now = h.engine.sessions_of(&master.id).unwrap().into_iter().find(|s| s.id == session.id).unwrap();
    (now.waiting_question_id, now.waiting_options, now.waiting_since)
}

#[tokio::test]
async fn a_runs_messages_leave_a_waiting_question_open() {
    let h = harness();
    let (session, mp) = masters_turn(&h).await;
    let plan = bot_named(&h, "plan").await;
    h.engine.create_workflow(flow(&[&plan])).unwrap();
    assert!(sbot(&h, &mp, workflow_run("Flow", "TASK-1", vec![])).await.ok);
    let asked = h.engine.handle_bridge(ask_with(&mp, "Push once it is done?", &["Yes", "No"])).await;
    let question = asked.message_id.expect("the question is stored");
    finish_turn(&h, &mp).await;
    let waiting = masters_question(&h, &session).await;
    assert_eq!((waiting.0.as_deref(), waiting.1.clone()), (Some(question.as_str()), vec!["Yes".into(), "No".into()]));

    assert!(hand_in(&h, &only_session(&h, &plan), "The plan.", None).await.ok);
    settle().await;
    assert!(prompt_of(&h, &session).starts_with("[Workflow \"Flow\" finished"));
    assert_eq!(masters_question(&h, &session).await, waiting);
}

const NESTED: &str =
    "This conversation is a step of a running workflow. Finish the step with `sbot result` instead of \
                      starting another workflow.";

#[tokio::test]
async fn a_step_of_a_run_starts_no_other_run() {
    let h = harness_after(|_| {});
    setup_team(&h, None);
    drop_seeded_workflow(&h);
    let master = bot_named(&h, "master").await;
    let plan = bot_named(&h, "plan").await;
    let wf = h.engine.create_workflow(flow(&[&master])).unwrap();
    let mut other = flow(&[&plan]);
    other.name = "Other".into();
    h.engine.create_workflow(other).unwrap();
    let step = h.engine.start_workflow(&wf.id, "Route TASK-1.", &[]).unwrap();
    settle().await;

    let e = sbot(&h, &pane(&h, &step), workflow_run("Other", "TASK-1", vec![])).await.error.unwrap();
    assert_eq!((e.code, e.message.as_str()), (ErrorCode::NotAllowed, NESTED));
    assert!(h.engine.sessions_of(&plan.id).unwrap().is_empty(), "no run started");
    let (run, visits) = run_of(&h, &step);
    assert_eq!((run.status, visits.len()), (RunStatus::Running, 1));
}

#[tokio::test]
async fn a_reminder_whose_step_closed_meanwhile_is_cancelled_not_delivered() {
    let h = harness_after(|_| {});
    let (_, feature, wf) = plan_and_feature(&h);
    let root = h.engine.start_workflow(&wf.id, "TASK-1 build the export", &[]).unwrap();
    settle().await;
    let root_pane = pane(&h, &root);
    h.engine.set_paused(true).await.unwrap();
    finish_turn(&h, &root_pane).await;
    let [reminder] = &reminders(&h, &root)[..] else { panic!("one reminder") };
    assert_eq!(reminder.status, MessageStatus::Queued);
    let prompts = prompts_of(&h, &root).len();

    assert!(h.engine.handle_bridge(result(&root_pane, "The plan.")).await.ok);
    let mut ui = h.engine.subscribe_ui();
    h.engine.set_paused(false).await.unwrap();
    settle().await;
    let [reminder] = &reminders(&h, &root)[..] else { panic!("one reminder") };
    assert_eq!(reminder.status, MessageStatus::Cancelled);
    assert_eq!(prompts_of(&h, &root).len(), prompts);
    assert!(events(&mut ui).iter().any(|e| matches!(
        e,
        UiEvent::MessageUpdated { message } if message.id == reminder.id && message.status == MessageStatus::Cancelled
    )));
    assert!(prompt_of(&h, &only_session(&h, &feature)).contains("· step \"Step 2\" · visit 1 ·"));
}

#[tokio::test]
async fn a_launch_pauses_a_run_whose_open_step_lost_its_bot() {
    let h = harness_after(|_| {});
    let t = ticket(&h);
    let (root, _) = to_review(&h, &t).await;
    Store::open(&h.paths.db_path()).unwrap().delete_bot(&t.review.id).unwrap();
    assert_eq!(run_of(&h, &root).0.status, RunStatus::Running);

    let again = relaunch(&h);
    let mut ui = again.engine.subscribe_ui();
    again.engine.recover().await.unwrap();
    let (run, _) = run_of(&again, &root);
    assert_eq!(
        (run.status, run.pause.clone()),
        (RunStatus::Paused, Some(PauseReason::BotMissing { step_id: "s3".into() }))
    );
    assert!(events(&mut ui).iter().any(|e| matches!(
        e,
        UiEvent::WorkflowRunChanged { run_id, status: RunStatus::Paused, .. } if *run_id == run.id
    )));
}

#[tokio::test]
async fn a_request_file_gone_before_a_later_bots_first_visit_is_left_out_with_a_warning() {
    let (log, _guard) = capture();
    let h = harness_after(|_| {});
    let (_, feature, wf) = plan_and_feature(&h);
    let src = tempfile::tempdir().unwrap();
    let spec = src.path().join("spec.md");
    std::fs::write(&spec, "# Spec").unwrap();
    let root = h.engine.start_workflow(&wf.id, "Build it.", &[spec.to_string_lossy().into_owned()]).unwrap();
    settle().await;
    assert_eq!(first_message(&h, &root).attachments.len(), 1, "@plan got the spec");

    std::fs::remove_file(&spec).unwrap();
    assert!(hand_in(&h, &root, "The plan.", None).await.ok);
    let step_two = only_session(&h, &feature);
    let visit = first_message(&h, &step_two);
    assert!(visit.attachments.is_empty(), "{:?}", visit.attachments);
    let inbox = h.paths.bot_dir("feature").join("inbox");
    assert!(!inbox.exists() || std::fs::read_dir(&inbox).unwrap().next().is_none(), "nothing reached its inbox");
    let prompt = prompt_of(&h, &step_two);
    assert!(prompt.contains("Request:\nBuild it."), "{prompt}");
    assert!(prompt.contains("--- Step \"Step 1\" (@plan), visit 1:\nThe plan."), "{prompt}");
    assert!(!prompt.contains("Attachments:"), "{prompt}");
    assert_eq!(run_of(&h, &root).0.status, RunStatus::Running);
    let text = log.text();
    let warning = text.lines().find(|l| l.contains("left a file out of a workflow message")).expect("a warning");
    assert!(warning.contains("WARN") && warning.contains("bot=feature"), "{warning}");
    assert!(warning.contains(&format!("message_id={}", visit.id)) && warning.contains("spec.md"), "{warning}");
}

#[tokio::test]
async fn a_result_after_its_run_stopped_is_refused_and_nothing_is_stored() {
    let h = harness_after(|_| {});
    let (_, _, wf) = plan_and_feature(&h);
    let root = h.engine.start_workflow(&wf.id, "TASK-1 build the export", &[]).unwrap();
    settle().await;
    let (run, _) = run_of(&h, &root);
    h.engine.stop_run(&run.id).await.unwrap();
    let (stopped, visits) = run_of(&h, &root);
    let chat = h.engine.session_messages(&root.id).unwrap();

    let response = h.engine.handle_bridge(result_with(&pane(&h, &root), "The plan.", "approved")).await;
    assert_eq!(refused(response), (ErrorCode::BadRequest, NOT_A_STEP.into()));
    assert_eq!(h.engine.session_messages(&root.id).unwrap(), chat, "no message is stored");
    assert_eq!(run_of(&h, &root), (stopped, visits), "the run and its visits are unchanged");
}
