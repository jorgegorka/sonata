mod support;

use bridge_proto::BridgeCommand;
use sonata_core::model::*;
use sonata_core::orchestration::{Executor, Orchestration, Plan, Review, Status, TaskSpec, TaskState, Update};
use support::*;

fn spec(key: &str, executor: Executor, deps: &[&str]) -> TaskSpec {
    TaskSpec {
        key: key.into(),
        title: format!("Task {key}"),
        instructions: format!("Do {key}"),
        acceptance_criteria: vec!["Evidence supports the result".into()],
        executor,
        depends_on: deps.iter().map(|s| (*s).into()).collect(),
        files: vec![],
    }
}

fn bot_spec(key: &str, bot: &Bot, deps: &[&str]) -> TaskSpec {
    spec(key, Executor::Bot { handle: bot.handle.clone() }, deps)
}

fn workflow(h: &Harness, name: &str, bots: &[&Bot]) -> Workflow {
    let steps = bots
        .iter()
        .enumerate()
        .map(|(i, bot)| Step {
            id: format!("s{i}"),
            name: format!("Step {i}"),
            bot_id: bot.id.clone(),
            instructions: "Do the step".into(),
            pos: None,
            arrows: vec![Arrow {
                to: if i + 1 == bots.len() { DONE.into() } else { format!("s{}", i + 1) },
                outcome: None,
            }],
        })
        .collect();
    h.engine
        .create_workflow(NewWorkflow {
            name: name.into(),
            when_to_use: String::new(),
            def: WorkflowDef {
                version: WORKFLOW_FORMAT,
                start: Some("s0".into()),
                steps,
                start_pos: None,
                done_pos: None,
            },
        })
        .unwrap()
}

fn start(h: &Harness, tasks: Vec<TaskSpec>) -> Orchestration {
    h.engine.finish_setup(None).unwrap();
    let master = h.engine.master_id().unwrap().unwrap();
    let (session, message) = h.engine.start_session_with_message(&master, "Coordinate the whole process", &[]).unwrap();
    h.engine
        .start_orchestration(
            &session.id,
            &message.id,
            "process",
            Plan { title: "Process".into(), goal: message.body, tasks },
        )
        .unwrap()
}

fn current(h: &Harness, run: &Orchestration) -> Orchestration {
    h.engine.orchestration_view(&run.id).unwrap().run
}

async fn result_of(h: &Harness, run: &Orchestration, key: &str, body: &str) {
    let run = current(h, run);
    let task = run.tasks.iter().find(|t| t.spec.key == key).unwrap();
    let session = h
        .engine
        .all_sessions()
        .await
        .unwrap()
        .into_iter()
        .find(|s| Some(&s.session.id) == task.session_id.as_ref())
        .unwrap()
        .session;
    h.engine.pump_session(&session.id).await;
    let response = h.engine.handle_bridge(result(&pane(h, &session), body)).await;
    assert!(response.ok, "{response:?}");
}

fn review(h: &Harness, run: &Orchestration, key: &str, succeeded: bool) -> Orchestration {
    let revision = current(h, run).revision;
    h.engine
        .update_orchestration(
            &run.id,
            revision,
            &format!("review-{key}"),
            Update {
                reviews: vec![Review { task: key.into(), succeeded, explanation: "Checked the evidence".into() }],
                ..Update::default()
            },
            None,
        )
        .unwrap()
}

#[tokio::test]
async fn different_workflows_join_only_after_review_and_deliver_one_report() {
    let h = harness();
    let a = h.engine.create_bot(claude_bot("Research")).unwrap();
    let b = h.engine.create_bot(claude_bot("Analysis")).unwrap();
    let writer = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let wa = workflow(&h, "Research flow", &[&a]);
    let wb = workflow(&h, "Analysis flow", &[&b]);
    let run = start(
        &h,
        vec![
            spec("a", Executor::Workflow { name: wa.name }, &[]),
            spec("b", Executor::Workflow { name: wb.name }, &[]),
            bot_spec("report", &writer, &["a", "b"]),
        ],
    );
    assert_eq!(
        run.tasks.iter().map(|t| t.state).collect::<Vec<_>>(),
        [TaskState::Running, TaskState::Running, TaskState::Waiting]
    );
    assert_ne!(run.tasks[0].workflow_run_id, run.tasks[1].workflow_run_id);
    result_of(&h, &run, "a", "Research evidence").await;
    result_of(&h, &run, "b", "Analysis evidence").await;
    assert_eq!(current(&h, &run).tasks[2].state, TaskState::Waiting);
    assert!(h.engine.finish_orchestration(&run.id, current(&h, &run).revision, "Too early", &[], None).is_err());
    review(&h, &run, "a", true);
    assert_eq!(current(&h, &run).tasks[2].state, TaskState::Waiting);
    let accepted = review(&h, &run, "b", true);
    assert_eq!(accepted.tasks[2].state, TaskState::Running);
    h.engine.pump_session(accepted.tasks[2].session_id.as_deref().unwrap()).await;
    assert!(h
        .rt
        .state()
        .prompts
        .iter()
        .any(|(_, body)| body.contains("Research evidence") && body.contains("Analysis evidence")));
    result_of(&h, &run, "report", "Combined deliverable").await;
    let accepted = review(&h, &run, "report", true);
    let final_message =
        h.engine.finish_orchestration(&run.id, accepted.revision, "Whole process succeeded", &[], None).unwrap();
    let again =
        h.engine.finish_orchestration(&run.id, accepted.revision, "Whole process succeeded", &[], None).unwrap();
    assert_eq!(again.id, final_message.id);
    assert_eq!(current(&h, &run).status, Status::Completed);
    assert_eq!(
        h.engine
            .session_messages(&run.session_id)
            .unwrap()
            .iter()
            .filter(|m| m.body == "Whole process succeeded")
            .count(),
        1
    );
}

#[tokio::test]
async fn same_bot_tasks_have_fresh_sessions_and_progress_is_not_completion() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
    let run = start(&h, vec![bot_spec("a", &bot, &[]), bot_spec("b", &bot, &[])]);
    assert_ne!(run.tasks[0].session_id, run.tasks[1].session_id);
    let session = h
        .engine
        .sessions_of(&bot.id)
        .unwrap()
        .into_iter()
        .find(|s| Some(&s.id) == run.tasks[0].session_id.as_ref())
        .unwrap();
    h.engine.pump_session(&session.id).await;
    assert!(h.engine.handle_bridge(reply(&pane(&h, &session), "Working on it")).await.ok);
    assert_eq!(current(&h, &run).tasks[0].state, TaskState::Running);
    assert!(h.engine.handle_bridge(ask(&pane(&h, &session), "Which source?")).await.ok);
    assert_eq!(current(&h, &run).tasks[0].state, TaskState::Running);
    result_of(&h, &run, "a", "Final evidence").await;
    assert_eq!(current(&h, &run).tasks[0].state, TaskState::AwaitingReview);
    assert_eq!(current(&h, &run).tasks[1].state, TaskState::Running);
}

#[tokio::test]
async fn intermediate_workflow_steps_do_not_complete_a_task() {
    let h = harness();
    let a = h.engine.create_bot(claude_bot("First")).unwrap();
    let b = h.engine.create_bot(claude_bot("Second")).unwrap();
    let flow = workflow(&h, "Two steps", &[&a, &b]);
    let run = start(&h, vec![spec("flow", Executor::Workflow { name: flow.name }, &[])]);
    result_of(&h, &run, "flow", "First step done").await;
    let run = current(&h, &run);
    assert_eq!(run.tasks[0].state, TaskState::Running);
    let view = h.engine.orchestration_view(&run.id).unwrap();
    assert_eq!(view.executions[0].session_ids.len(), 2, "{view:#?}");
    assert!(h.engine.finish_orchestration(&run.id, run.revision, "Not done", &[], None).is_err());
}

#[tokio::test]
async fn atomic_followup_edit_and_review_hold_downstream_work() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
    let run = start(&h, vec![bot_spec("a", &bot, &[]), bot_spec("downstream", &bot, &["a"])]);
    result_of(&h, &run, "a", "Initial result").await;
    let revision = current(&h, &run).revision;
    let edit = Update {
        add: vec![bot_spec("followup", &bot, &["a"])],
        replace: vec![bot_spec("downstream", &bot, &["followup"])],
        reviews: vec![Review { task: "a".into(), succeeded: true, explanation: "Checked".into() }],
    };
    let updated = h.engine.update_orchestration(&run.id, revision, "extend", edit.clone(), None).unwrap();
    assert_eq!(updated.tasks[1].state, TaskState::Waiting);
    assert_eq!(updated.tasks[2].state, TaskState::Running);
    let retry = h.engine.update_orchestration(&run.id, revision, "extend", edit, None).unwrap();
    assert_eq!(retry.tasks.len(), 3);
    assert_eq!(retry.tasks[2].session_id, updated.tasks[2].session_id);
    assert!(h.engine.update_orchestration(&run.id, revision, "stale", Update::default(), None).is_err());
}

#[tokio::test]
async fn failures_block_dependents_and_independent_work_continues() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
    let run = start(
        &h,
        vec![
            bot_spec("a", &bot, &[]),
            bot_spec("b", &bot, &[]),
            bot_spec("c", &bot, &["a"]),
            bot_spec("d", &bot, &["c"]),
        ],
    );
    result_of(&h, &run, "a", "Unable to complete").await;
    let updated = review(&h, &run, "a", false);
    assert_eq!(
        updated.tasks.iter().map(|t| t.state).collect::<Vec<_>>(),
        [TaskState::Failed, TaskState::Running, TaskState::Blocked, TaskState::Blocked]
    );
    result_of(&h, &run, "b", "Independent deliverable").await;
    let done = review(&h, &run, "b", true);
    h.engine
        .finish_orchestration(
            &run.id,
            done.revision,
            "Partial result: b succeeded; a failed; c and d were blocked.",
            &[],
            None,
        )
        .unwrap();
}

#[tokio::test]
async fn rejects_cycles_unknown_dependencies_and_unauthorized_bridge_callers() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
    let run = start(&h, vec![bot_spec("a", &bot, &[]), bot_spec("b", &bot, &["a"])]);
    let edit = Update { replace: vec![bot_spec("b", &bot, &["b"])], ..Update::default() };
    assert!(h.engine.update_orchestration(&run.id, run.revision, "cycle", edit, None).is_err());
    let edit = Update { add: vec![bot_spec("c", &bot, &["missing"])], ..Update::default() };
    assert!(h.engine.update_orchestration(&run.id, run.revision, "missing", edit, None).is_err());
    let session = h.engine.sessions_of(&bot.id).unwrap().remove(0);
    h.engine.pump_session(&session.id).await;
    assert!(
        !h.engine.handle_bridge(request(&pane(&h, &session), BridgeCommand::OrchestrationShow { id: run.id })).await.ok
    );
}

#[tokio::test]
async fn cancelled_tasks_do_not_release_dependencies_or_accept_late_results() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
    let run = start(&h, vec![bot_spec("a", &bot, &[]), bot_spec("b", &bot, &["a"])]);
    h.engine.cancel_orchestration(&run.id, Some("a")).await.unwrap();
    let run = current(&h, &run);
    assert_eq!(run.tasks[0].state, TaskState::Cancelled);
    assert_eq!(run.tasks[1].state, TaskState::Blocked);
    h.engine.finish_orchestration(&run.id, run.revision, "The requested task was cancelled.", &[], None).unwrap();
}

#[tokio::test]
async fn cancelling_a_workflow_does_not_stop_master_or_another_workflow() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
    let flow = workflow(&h, "Flow", &[&bot]);
    let run = start(
        &h,
        vec![
            spec("a", Executor::Workflow { name: flow.name.clone() }, &[]),
            spec("b", Executor::Workflow { name: flow.name }, &[]),
        ],
    );
    for task in &run.tasks {
        h.engine.pump_session(task.session_id.as_deref().unwrap()).await;
    }
    h.engine.pump_session(&run.session_id).await;
    let view = h.engine.orchestration_view(&run.id).unwrap();
    assert!(view.executions.iter().all(|t| !t.session_ids.contains(&run.session_id)));
    h.engine.cancel_orchestration(&run.id, Some("a")).await.unwrap();
    let updated = current(&h, &run);
    assert_eq!(updated.tasks[0].state, TaskState::Cancelled);
    assert_eq!(updated.tasks[1].state, TaskState::Running);
    assert_eq!(
        h.engine.get_workflow_run(updated.tasks[1].workflow_run_id.as_deref().unwrap()).unwrap().run.status,
        RunStatus::Running
    );
    result_of(&h, &run, "b", "Sibling still works").await;
}

#[tokio::test]
async fn archived_history_is_restorable_and_active_work_cannot_be_marked_done() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
    let run = start(&h, vec![bot_spec("a", &bot, &[])]);
    assert!(h.engine.archive_session(&run.session_id).await.is_err());
    assert!(h.engine.archive_project("default").await.is_err());
    assert!(h.engine.archive_orchestration_task(&run.id, "a", true).await.is_err());
    result_of(&h, &run, "a", "Preserved output").await;
    review(&h, &run, "a", true);
    h.engine.archive_orchestration_task(&run.id, "a", true).await.unwrap();
    let archived = current(&h, &run);
    assert!(archived.tasks[0].archived_at.is_some());
    h.engine.archive_orchestration_task(&run.id, "a", false).await.unwrap();
    let restored = current(&h, &run);
    assert_eq!(restored.tasks[0].result.as_deref(), Some("Preserved output"));
    assert_eq!(restored.tasks[0].state, TaskState::Succeeded);
    assert!(restored.tasks[0].archived_at.is_none());
    h.engine.finish_orchestration(&run.id, restored.revision, "Finished", &[], None).unwrap();
    h.engine.archive_session(&run.session_id).await.unwrap();
    assert!(h.engine.find_session_view(&run.session_id).await.unwrap().session.archived_at.is_some());
    h.engine.archive_orchestration_task(&run.id, "a", false).await.unwrap();
    assert!(h.engine.find_session_view(&run.session_id).await.unwrap().session.archived_at.is_none());
    assert!(h
        .engine
        .find_session_view(restored.tasks[0].session_id.as_deref().unwrap())
        .await
        .unwrap()
        .session
        .archived_at
        .is_none());
}

#[tokio::test]
async fn workflow_assignment_uses_its_snapshot_even_if_template_is_deleted_before_dispatch() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
    let flow = workflow(&h, "Flow", &[&bot]);
    let run = start(&h, vec![bot_spec("a", &bot, &[]), spec("b", Executor::Workflow { name: flow.name }, &["a"])]);
    h.engine.delete_workflow(&flow.id).unwrap();
    result_of(&h, &run, "a", "Input").await;
    let run = review(&h, &run, "a", true);
    assert_eq!(run.tasks[1].state, TaskState::Running);
    assert!(h
        .engine
        .get_workflow_run(run.tasks[1].workflow_run_id.as_deref().unwrap())
        .unwrap()
        .run
        .workflow_id
        .is_none());
}

#[tokio::test]
async fn missing_required_inputs_fail_visibly_before_dispatch() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
    let artifact = h.paths.bots_root.join("artifact.txt");
    std::fs::write(&artifact, "Verified source").unwrap();
    let mut next = bot_spec("b", &bot, &["a"]);
    next.files = vec![artifact.to_string_lossy().into_owned()];
    let run = start(&h, vec![bot_spec("a", &bot, &[]), next]);
    std::fs::remove_file(&artifact).unwrap();
    result_of(&h, &run, "a", "Input").await;
    let run = review(&h, &run, "a", true);
    assert_eq!(run.tasks[1].state, TaskState::Failed);
    assert!(run.tasks[1].review.as_ref().unwrap().contains("artifact.txt"));
    assert!(run.tasks[1].session_id.is_none());
}

#[tokio::test]
async fn result_artifacts_reach_bot_and_workflow_dependents() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
    let flow = workflow(&h, "Flow", &[&bot]);
    let run = start(
        &h,
        vec![
            bot_spec("a", &bot, &[]),
            bot_spec("b", &bot, &["a"]),
            spec("c", Executor::Workflow { name: flow.name }, &["a"]),
        ],
    );
    let artifact = h.paths.bots_root.join("result.txt");
    std::fs::write(&artifact, "Verified evidence").unwrap();
    let session = h.engine.find_session_view(run.tasks[0].session_id.as_deref().unwrap()).await.unwrap().session;
    h.engine.pump_session(&session.id).await;
    let response = h
        .engine
        .handle_bridge(request(
            &pane(&h, &session),
            BridgeCommand::Result {
                body: "Evidence attached".into(),
                files: vec![artifact.to_string_lossy().into_owned()],
                outcome: None,
            },
        ))
        .await;
    assert!(response.ok, "{response:?}");
    let reviewed = review(&h, &run, "a", true);
    let mut messages = h
        .engine
        .bot_threads()
        .unwrap()
        .into_iter()
        .flat_map(|t| h.engine.thread_messages(&t.id).unwrap())
        .collect::<Vec<_>>();
    for task in &reviewed.tasks[1..] {
        messages.extend(h.engine.session_messages(task.session_id.as_deref().unwrap()).unwrap());
        let message = messages.iter().find(|m| Some(&m.id) == task.request_message_id.as_ref()).unwrap();
        assert!(message.body.contains("Evidence attached"));
        assert_eq!(message.attachments.len(), 1);
        assert_eq!(std::fs::read_to_string(&message.attachments[0].path).unwrap(), "Verified evidence");
    }
}

#[tokio::test]
async fn automated_events_and_their_retries_preserve_pending_user_questions() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
    let run = start(&h, vec![bot_spec("a", &bot, &[])]);
    h.engine.pump_session(&run.session_id).await;
    let master = h.engine.find_session_view(&run.session_id).await.unwrap().session;
    let master_pane = pane(&h, &master);
    let question = h.engine.handle_bridge(ask(&master_pane, "May I publish the result?")).await;
    assert!(question.ok);
    finish_turn(&h, &master_pane).await;
    let prompt_count = h.rt.state().prompts.iter().filter(|(pane, _)| pane == &master_pane).count();
    result_of(&h, &run, "a", "Ready").await;
    review(&h, &run, "a", true);
    let event = h
        .engine
        .session_messages(&run.session_id)
        .unwrap()
        .into_iter()
        .find(|m| m.system_code.as_deref() == Some("orchestration_event"))
        .unwrap();
    let db = rusqlite::Connection::open(h.paths.db_path()).unwrap();
    db.execute("UPDATE messages SET status = 'interrupted' WHERE id = ?1", [&event.id]).unwrap();
    let retried = h.engine.retry_message(&event.id).unwrap();
    assert_eq!(retried.system_code, event.system_code);
    assert_eq!(retried.system_params, event.system_params);
    h.engine.pump_session(&run.session_id).await;
    assert_eq!(
        h.engine.find_session_view(&run.session_id).await.unwrap().session.waiting_question_id,
        question.message_id
    );
    assert_eq!(h.rt.state().prompts.iter().filter(|(pane, _)| pane == &master_pane).count(), prompt_count);
}

#[tokio::test]
async fn cancelled_execution_cannot_be_retried_or_receive_new_work() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
    let run = start(&h, vec![bot_spec("a", &bot, &[])]);
    let session = h.engine.find_session_view(run.tasks[0].session_id.as_deref().unwrap()).await.unwrap().session;
    h.engine.pump_session(&session.id).await;
    let old_pane = pane(&h, &session);
    h.engine.cancel_orchestration(&run.id, Some("a")).await.unwrap();
    assert!(!h.engine.handle_bridge(result(&old_pane, "Late result")).await.ok);
    let request = run.tasks[0].request_message_id.as_deref().unwrap();
    let db = rusqlite::Connection::open(h.paths.db_path()).unwrap();
    db.execute("UPDATE messages SET status = 'interrupted' WHERE id = ?1", [request]).unwrap();
    assert!(h.engine.retry_message(request).is_err());
    assert!(h.engine.queue_user_message_to_session(&session.id, "Try again", &[]).is_err());
    h.engine.pump_session(&session.id).await;
    assert_eq!(current(&h, &run).tasks[0].state, TaskState::Cancelled);
}

#[tokio::test]
async fn restart_preserves_results_and_idempotency_without_relaunching_completed_work() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
    let run = start(&h, vec![bot_spec("a", &bot, &[]), bot_spec("b", &bot, &["a"])]);
    result_of(&h, &run, "a", "Saved before restart").await;
    settle().await;
    let next = relaunch(&h);
    next.engine.recover().await.unwrap();
    let restored = current(&next, &run);
    assert_eq!(restored.tasks[0].state, TaskState::AwaitingReview);
    assert_eq!(restored.tasks[0].result.as_deref(), Some("Saved before restart"));
    assert_eq!(restored.tasks[0].session_id, run.tasks[0].session_id);
    let reviewed = review(&next, &run, "a", true);
    assert_eq!(reviewed.tasks[1].state, TaskState::Running);
    assert_eq!(next.engine.sessions_of(&bot.id).unwrap().len(), 2);
}

#[tokio::test]
async fn restart_finishes_cancellation_committed_before_runtime_cleanup() {
    let h = harness_after(|_| {});
    h.engine.set_paused(true).await.unwrap();
    let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
    let run = start(&h, vec![bot_spec("a", &bot, &[])]);
    settle().await;
    let store = sonata_core::store::Store::open(&h.paths.db_path()).unwrap();
    store.cancel_orchestration(&run.id, None).unwrap();
    let next = relaunch(&h);
    next.engine.recover().await.unwrap();
    let request = store.get_message(run.tasks[0].request_message_id.as_deref().unwrap()).unwrap().unwrap();
    assert_eq!(request.status, MessageStatus::Cancelled);
    next.engine.set_paused(false).await.unwrap();
    next.engine.pump_session(run.tasks[0].session_id.as_deref().unwrap()).await;
    assert!(next.rt.state().prompts.iter().all(|(_, body)| !body.contains("Task: Task a")));
    assert_eq!(current(&next, &run).status, Status::Cancelled);
}

#[tokio::test]
async fn review_notifications_do_not_grant_permission_to_create_resources() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
    let run = start(&h, vec![bot_spec("a", &bot, &[])]);
    h.engine.pump_session(&run.session_id).await;
    let master = h
        .engine
        .sessions_of(&h.engine.master_id().unwrap().unwrap())
        .unwrap()
        .into_iter()
        .find(|s| s.id == run.session_id)
        .unwrap();
    let master_pane = pane(&h, &master);
    h.engine.handle_bridge(reply(&master_pane, "Tasks started")).await;
    finish_turn(&h, &master_pane).await;
    result_of(&h, &run, "a", "Ready for review").await;
    settle().await;
    h.engine.pump_session(&run.session_id).await;
    let response = h
        .engine
        .handle_bridge(request(
            &master_pane,
            BridgeCommand::OrchestrationStart { key: "unapproved".into(), body: "{}".into() },
        ))
        .await;
    assert!(!response.ok);
    assert_eq!(response.error.unwrap().code, bridge_proto::ErrorCode::NotAllowed);
}

#[tokio::test]
async fn project_ownership_is_enforced_for_reads_and_cancellation() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
    let run = start(&h, vec![bot_spec("a", &bot, &[])]);
    let project = h
        .engine
        .create_project(ProjectInput {
            name: "Other".into(),
            default_directory: Some(h.paths.bots_root.to_string_lossy().into_owned()),
            default_provider: Provider::Claude,
        })
        .unwrap();
    let other = h.engine.for_project(&project.id).unwrap();
    assert!(other.list_orchestrations().unwrap().is_empty());
    assert!(other.orchestration_view(&run.id).is_err());
    assert!(other.cancel_orchestration(&run.id, None).await.is_err());
    assert_eq!(current(&h, &run).tasks[0].state, TaskState::Running);
}

#[tokio::test]
async fn final_report_and_completion_roll_back_together_on_storage_failure() {
    let h = harness_after(|_| {});
    let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
    let run = start(&h, vec![bot_spec("a", &bot, &[])]);
    result_of(&h, &run, "a", "Evidence").await;
    let reviewed = review(&h, &run, "a", true);
    let db = rusqlite::Connection::open(h.paths.db_path()).unwrap();
    db.execute_batch("CREATE TRIGGER reject_completion BEFORE UPDATE ON orchestrations WHEN NEW.status = 'completed' BEGIN SELECT RAISE(ABORT, 'simulated failure'); END;").unwrap();
    assert!(h.engine.finish_orchestration(&run.id, reviewed.revision, "Final report", &[], None).is_err());
    assert_eq!(current(&h, &run).status, Status::Active);
    assert!(!h.engine.session_messages(&run.session_id).unwrap().iter().any(|m| m.body == "Final report"));
    db.execute_batch("DROP TRIGGER reject_completion;").unwrap();
    h.engine.finish_orchestration(&run.id, reviewed.revision, "Final report", &[], None).unwrap();
    assert_eq!(
        h.engine.session_messages(&run.session_id).unwrap().iter().filter(|m| m.body == "Final report").count(),
        1
    );
}

#[tokio::test]
async fn task_results_and_workflow_completion_roll_back_if_review_state_cannot_be_saved() {
    for workflow_task in [false, true] {
        let h = harness_after(|_| {});
        let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
        let task = if workflow_task {
            let flow = workflow(&h, "Flow", &[&bot]);
            spec("a", Executor::Workflow { name: flow.name }, &[])
        } else {
            bot_spec("a", &bot, &[])
        };
        let run = start(&h, vec![task]);
        let session = h.engine.sessions_of(&bot.id).unwrap().remove(0);
        h.engine.pump_session(&session.id).await;
        let db = rusqlite::Connection::open(h.paths.db_path()).unwrap();
        db.execute_batch("CREATE TRIGGER reject_review BEFORE UPDATE ON orchestrations WHEN json_extract(NEW.data, '$.tasks[0].state') = 'awaiting_review' BEGIN SELECT RAISE(ABORT, 'simulated failure'); END;").unwrap();
        assert!(!h.engine.handle_bridge(result(&pane(&h, &session), "Complete")).await.ok);
        let after = current(&h, &run);
        assert_eq!(after.tasks[0].state, TaskState::Running);
        assert!(after.tasks[0].result.is_none());
        if let Some(id) = after.tasks[0].workflow_run_id.as_deref() {
            assert_eq!(h.engine.get_workflow_run(id).unwrap().run.status, RunStatus::Running);
        }
        db.execute_batch("DROP TRIGGER reject_review;").unwrap();
        result_of(&h, &run, "a", "Complete").await;
        assert_eq!(current(&h, &run).tasks[0].state, TaskState::AwaitingReview);
    }
}

#[tokio::test]
async fn start_retries_and_competing_updates_do_not_duplicate_execution() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Worker")).unwrap();
    let run = start(&h, vec![bot_spec("a", &bot, &[])]);
    let again = h
        .engine
        .start_orchestration(
            &run.session_id,
            &run.request_message_id,
            "process",
            Plan {
                title: run.title.clone(),
                goal: run.goal.clone(),
                tasks: run.tasks.iter().map(|t| t.spec.clone()).collect(),
            },
        )
        .unwrap();
    assert_eq!(again.id, run.id);
    assert_eq!(h.engine.sessions_of(&bot.id).unwrap().len(), 1);
    result_of(&h, &run, "a", "Evidence").await;
    let revision = current(&h, &run).revision;
    let edit = Update {
        reviews: vec![Review { task: "a".into(), succeeded: true, explanation: "Checked".into() }],
        ..Update::default()
    };
    let (left, right) =
        tokio::join!(async { h.engine.update_orchestration(&run.id, revision, "review", edit.clone(), None) }, async {
            h.engine.update_orchestration(&run.id, revision, "review", edit.clone(), None)
        });
    assert_eq!(left.unwrap(), right.unwrap());
    assert_eq!(h.engine.sessions_of(&bot.id).unwrap().len(), 1);
}
