mod support;
use bridge_proto::{BridgeCommand, BridgeRequest, ErrorCode};
use serde_json::{json, Value};
use sonata_core::engine::{OnboardingProfile, OnboardingProgress, OnboardingState};
use sonata_core::model::*;
use sonata_core::store::Store;
use support::*;

fn profile(h: &Harness) {
    h.engine
        .save_onboarding(OnboardingProgress {
            profile: OnboardingProfile { job: "Researcher".into(), tasks: "Compare papers and prepare reports".into() },
            stage: "suggestions".into(),
            ..Default::default()
        })
        .unwrap();
}
fn draft() -> Value {
    json!({"workflows":[
        {"name":"Weekly report","when_to_use":"Summarize supplied papers","expected_output":"A sourced report","requirements":["Papers"],"definition":{"version":1,"start":"write","steps":[{"id":"write","name":"Write","bot":"writer","instructions":"Compare the supplied papers and write the report.","arrows":[{"to":"done"}]}]}},
        {"name":"Short summary","when_to_use":"Summarize one paper","expected_output":"A concise summary","definition":{"version":1,"start":"summarize","steps":[{"id":"summarize","name":"Summarize","bot":"writer","instructions":"Summarize the supplied paper.","arrows":[]}]}},
        {"name":"Check a document","when_to_use":"Check a draft","expected_output":"Corrections","definition":{"version":1,"start":"check","steps":[{"id":"check","name":"Check","bot":"checker","instructions":"Check the supplied document.","arrows":[]}]}}
    ],"bots":[
        {"handle":"writer","name":"Writer","color":"#4f46e5","provider":"claude","instructions":"Write sourced reports.","skills":["report-writing"]},
        {"handle":"checker","name":"Checker","color":"#4f46e5","provider":"claude","instructions":"Check documents.","skills":[]}
    ],"skills":[{"slug":"report-writing","name":"Report writing","description":"Write reports.","files":[{"path":"SKILL.md","text":"---\nname: report-writing\ndescription: Write reports.\n---\nCite the supplied evidence."}]}]})
}
fn start(h: &Harness) -> OnboardingState {
    h.engine.finish_setup(None).unwrap();
    profile(h);
    h.engine.start_onboarding().unwrap()
}
fn propose(h: &Harness) -> OnboardingState {
    let state = start(h);
    h.engine
        .accept_onboarding_proposal(
            state.session_id.as_deref().unwrap(),
            state.generation.as_deref().unwrap(),
            &draft().to_string(),
        )
        .unwrap()
}
fn choose(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

#[tokio::test]
async fn first_run_profile_and_skip_do_not_create_optional_resources_or_start_sessions() {
    let h = harness_after(|_| {});
    assert!(h.engine.onboarding_pending().unwrap());
    profile(&h);
    assert!(h.engine.finish_onboarding().await.is_err());
    h.engine.finish_setup(None).unwrap();
    assert!(h.engine.onboarding_pending().unwrap(), "basic setup does not finish onboarding");
    h.engine.finish_onboarding().await.unwrap();
    let reopened = relaunch(&h);
    assert!(!reopened.engine.onboarding_pending().unwrap());
    assert_eq!(reopened.engine.onboarding_state().unwrap().progress.profile.job, "Researcher");
    assert_eq!(reopened.engine.list_bots().await.unwrap().len(), 1);
    assert!(reopened.engine.list_workflows().unwrap().is_empty());
    assert!(h.rt.state().starts.is_empty());
}

#[tokio::test]
async fn only_selected_workflows_and_their_shared_dependencies_are_created() {
    let h = harness();
    let state = propose(&h);
    assert_eq!(state.status, "ready");
    assert_eq!(h.engine.list_bots().await.unwrap().len(), 1, "proposal creates nothing");
    let selected = choose(&["Weekly report", "Short summary"]);
    let preview = h.engine.preview_onboarding(selected.clone()).unwrap();
    assert_eq!((preview.config.workflows.len(), preview.config.bots.len(), preview.config.skills.len()), (2, 1, 1));
    let result = h.engine.apply_onboarding(selected.clone(), &preview.token).unwrap();
    assert_eq!(result.state.status, "complete");
    assert_eq!(result.created.len(), 4);
    assert_eq!(h.engine.list_bots().await.unwrap().len(), 2);
    assert_eq!(h.engine.list_skills().unwrap().len(), 2); // includes Master's playbook
    assert!(h.rt.state().starts.is_empty(), "creating does not run anything");
    assert!(h.engine.list_tasks().unwrap().is_empty());
    assert!(h.engine.apply_onboarding(selected.clone(), &preview.token).unwrap().changed);
    let preview = h.engine.preview_onboarding(selected.clone()).unwrap();
    h.engine.apply_onboarding(selected, &preview.token).unwrap();
    assert_eq!(h.engine.list_workflows().unwrap().len(), 2);
}

#[tokio::test]
async fn partial_creation_survives_restart_and_retries_without_duplicates() {
    let h = harness_after(|_| {});
    propose(&h);
    let conn = rusqlite::Connection::open(h.paths.db_path()).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER fail_workflow BEFORE INSERT ON workflows BEGIN SELECT RAISE(ABORT, 'disk failure'); END;",
    )
    .unwrap();
    let selected = choose(&["Weekly report"]);
    let preview = h.engine.preview_onboarding(selected.clone()).unwrap();
    let result = h.engine.apply_onboarding(selected.clone(), &preview.token).unwrap();
    assert_eq!(result.state.status, "partial");
    assert_eq!(result.created.len(), 2);
    conn.execute_batch("DROP TRIGGER fail_workflow;").unwrap();
    let restarted = relaunch(&h);
    let preview = restarted.engine.preview_onboarding(selected.clone()).unwrap();
    assert_eq!(preview.created.len(), 2);
    let result = restarted.engine.apply_onboarding(selected, &preview.token).unwrap();
    assert_eq!(result.state.status, "complete");
    assert_eq!(result.created.len(), 3);
    assert_eq!(restarted.engine.list_bots().await.unwrap().len(), 2);
    assert_eq!(restarted.engine.list_workflows().unwrap().len(), 1);
}

#[tokio::test]
async fn changed_or_removed_created_resources_are_not_overwritten_or_recreated() {
    let h = harness();
    propose(&h);
    let selected = choose(&["Weekly report"]);
    let p = h.engine.preview_onboarding(selected.clone()).unwrap();
    h.engine.apply_onboarding(selected.clone(), &p.token).unwrap();
    let bot = h.engine.list_bots().await.unwrap().into_iter().find(|b| b.bot.handle == "writer").unwrap().bot;
    h.engine
        .update_bot(
            &bot.id,
            NewBot { instructions: "User edited this".into(), skill_ids: bot.skill_ids, ..claude_bot("Writer") },
        )
        .unwrap();
    assert!(h.engine.preview_onboarding(selected.clone()).is_err());
    let workflow = h.engine.list_workflows().unwrap().remove(0);
    h.engine.delete_workflow(&workflow.id).unwrap();
    assert!(h.engine.apply_onboarding(selected, &p.token).is_err());
    assert!(h.engine.list_workflows().unwrap().is_empty());
}

#[tokio::test]
async fn generation_is_bound_to_its_session_and_cancellation_ignores_late_results() {
    let h = harness();
    let state = start(&h);
    let session = state.session_id.as_deref().unwrap();
    let generation = state.generation.as_deref().unwrap();
    assert!(h.engine.accept_onboarding_proposal("other", generation, &draft().to_string()).is_err());
    assert!(h.engine.accept_onboarding_proposal(session, "old", &draft().to_string()).is_err());
    assert_eq!(
        h.engine.start_onboarding().unwrap().generation,
        state.generation,
        "double click does not enqueue another request"
    );
    h.engine.cancel_onboarding().await.unwrap();
    assert!(h.engine.accept_onboarding_proposal(session, generation, &draft().to_string()).is_err());
    assert!(h.engine.list_workflows().unwrap().is_empty());
}

#[test]
fn invalid_proposals_are_rejected_and_can_be_corrected() {
    let h = harness();
    let state = start(&h);
    for body in [
        "not json".to_string(),
        json!({"workflows":[]}).to_string(),
        {
            let mut d = draft();
            d["skills"][0]["files"][0]["executable"] = json!(true);
            d.to_string()
        },
        {
            let mut d = draft();
            d["workflows"][0]["definition"]["steps"][0]["arrows"][0]["to"] = json!("missing");
            d.to_string()
        },
    ] {
        assert!(h
            .engine
            .accept_onboarding_proposal(
                state.session_id.as_deref().unwrap(),
                state.generation.as_deref().unwrap(),
                &body
            )
            .is_err());
        assert_eq!(h.engine.onboarding_state().unwrap().status, "generating");
    }
    h.engine
        .accept_onboarding_proposal(
            state.session_id.as_deref().unwrap(),
            state.generation.as_deref().unwrap(),
            &draft().to_string(),
        )
        .unwrap();
}

#[tokio::test]
async fn names_are_resolved_before_review_and_existing_resources_are_preserved() {
    let h = harness();
    h.engine.create_bot(claude_bot("Writer")).unwrap();
    h.engine
        .create_skill(NewSkill {
            name: "report-writing".into(),
            description: "Existing".into(),
            body: "Keep this".into(),
        })
        .unwrap();
    let state = propose(&h);
    let proposal = state.proposal.unwrap();
    assert_eq!(proposal.bots[0].handle, "writer-2");
    assert_eq!(proposal.skills[0].slug, "report-writing-2");
    assert_eq!(proposal.workflows[0].workflow.definition.steps[0].bot.as_deref(), Some("writer-2"));
    let p = h.engine.preview_onboarding(choose(&["Weekly report"])).unwrap();
    h.engine.apply_onboarding(p.selected, &p.token).unwrap();
    let old = h.engine.list_skills().unwrap().into_iter().find(|s| s.skill.slug == "report-writing").unwrap();
    assert_eq!(h.engine.skill_body(&old.skill.id).unwrap(), "Keep this");
}

#[tokio::test]
async fn bridge_generation_cannot_create_resources_or_run_workflows() {
    let h = harness();
    let state = start(&h);
    let id = state.session_id.as_deref().unwrap();
    h.engine.pump_session(id).await;
    let session = h.engine.sessions_of(&h.engine.master_id().unwrap().unwrap()).unwrap().remove(0);
    let pane = pane(&h, &session);
    for command in [
        BridgeCommand::WorkflowRun { name: "anything".into(), body: "run".into(), files: vec![] },
        BridgeCommand::SkillAdd { folder: "/tmp/skill".into(), recreate: false },
        BridgeCommand::Send { to: "user".into(), body: "delegate".into(), files: vec![] },
    ] {
        let response =
            h.engine.handle_bridge(BridgeRequest { v: 1, pane_id: pane.clone(), cli_session_id: None, command }).await;
        assert_eq!(response.error.unwrap().code, ErrorCode::NotAllowed);
    }
    let response = h
        .engine
        .handle_bridge(BridgeRequest {
            v: 1,
            pane_id: pane,
            cli_session_id: None,
            command: BridgeCommand::OnboardingPropose {
                generation: state.generation.unwrap(),
                body: draft().to_string(),
            },
        })
        .await;
    assert!(response.error.is_none(), "{response:?}");
    assert_eq!(h.engine.onboarding_state().unwrap().status, "ready");
}

#[test]
fn restart_and_an_ended_turn_without_a_proposal_are_retryable() {
    let h = harness_after(|_| {});
    let state = start(&h);
    Store::open(&h.paths.db_path()).unwrap().mark_turn_ended(state.message_id.as_deref().unwrap()).unwrap();
    assert_eq!(h.engine.onboarding_state().unwrap().status, "failed");
    h.engine.start_onboarding().unwrap();
    let restarted = relaunch(&h);
    assert_eq!(restarted.engine.onboarding_state().unwrap().status, "failed");
    assert_eq!(restarted.engine.onboarding_state().unwrap().progress.profile.job, "Researcher");
    assert_eq!(restarted.engine.start_onboarding().unwrap().status, "generating");
}

#[tokio::test]
async fn projects_are_isolated_and_new_bots_inherit_the_provider() {
    let h = harness();
    h.engine.finish_setup(None).unwrap();
    let folder = tempfile::tempdir().unwrap();
    let project = h
        .engine
        .create_project(ProjectInput {
            name: "Other".into(),
            default_directory: Some(folder.path().display().to_string()),
            default_provider: Provider::Codex,
        })
        .unwrap();
    let engine = h.engine.for_project(&project.id).unwrap();
    engine
        .save_onboarding(OnboardingProgress {
            profile: OnboardingProfile { job: "Editor".into(), tasks: "Review drafts".into() },
            stage: "suggestions".into(),
            ..Default::default()
        })
        .unwrap();
    let state = engine.start_onboarding().unwrap();
    assert!(h
        .engine
        .accept_onboarding_proposal(
            state.session_id.as_deref().unwrap(),
            state.generation.as_deref().unwrap(),
            &draft().to_string()
        )
        .is_err());
    engine
        .accept_onboarding_proposal(
            state.session_id.as_deref().unwrap(),
            state.generation.as_deref().unwrap(),
            &draft().to_string(),
        )
        .unwrap();
    let p = engine.preview_onboarding(choose(&["Weekly report"])).unwrap();
    engine.apply_onboarding(p.selected, &p.token).unwrap();
    assert!(h.engine.for_project("default").unwrap().list_workflows().unwrap().is_empty());
    assert!(engine.list_bots().await.unwrap().iter().all(|b| b.bot.provider == Provider::Codex));
}

#[tokio::test]
async fn reuse_preserves_master_and_checks_versions_even_when_content_is_unchanged() {
    let h = harness();
    let state = start(&h);
    let master = h.engine.master_bot().unwrap().unwrap();
    let mut d = draft();
    d["workflows"].as_array_mut().unwrap().truncate(1);
    d["workflows"][0]["definition"]["steps"][0]["bot"] = json!(master.handle);
    d["bots"] = json!([]);
    d["skills"] = json!([]);
    h.engine
        .accept_onboarding_proposal(
            state.session_id.as_deref().unwrap(),
            state.generation.as_deref().unwrap(),
            &d.to_string(),
        )
        .unwrap();
    let p = h.engine.preview_onboarding(choose(&["Weekly report"])).unwrap();
    h.engine
        .update_bot(
            &master.id,
            NewBot {
                display_name: master.display_name,
                color: master.color,
                provider: master.provider,
                model: master.model,
                instructions: master.instructions,
                skill_ids: master.skill_ids,
            },
        )
        .unwrap();
    assert!(h.engine.apply_onboarding(p.selected.clone(), &p.token).unwrap().changed);
    assert!(h.engine.list_workflows().unwrap().is_empty());
    let p = h.engine.preview_onboarding(p.selected).unwrap();
    let result = h.engine.apply_onboarding(p.selected, &p.token).unwrap();
    assert_eq!(result.created.len(), 1);
    assert_eq!(h.engine.list_bots().await.unwrap().len(), 1);
}

#[tokio::test]
async fn an_owned_skill_folder_left_before_the_database_write_is_recovered() {
    let h = harness_after(|_| {});
    let mut state = propose(&h);
    state.status = "applying".into();
    let skill = &state.bundle.as_ref().unwrap().skills[0];
    let dir = h.paths.skills_dir().join(&skill.slug);
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join(".onboarding-owner"), state.generation.as_deref().unwrap()).unwrap();
    std::fs::write(dir.join("SKILL.md"), skill.files[0].text.as_deref().unwrap()).unwrap();
    Store::open(&h.paths.db_path())
        .unwrap()
        .set_setting("onboarding.project.default", &serde_json::to_string(&state).unwrap())
        .unwrap();
    let reopened = relaunch(&h);
    assert_eq!(reopened.engine.onboarding_state().unwrap().status, "partial");
    let p = reopened.engine.preview_onboarding(choose(&["Weekly report"])).unwrap();
    assert_eq!(reopened.engine.apply_onboarding(p.selected, &p.token).unwrap().state.status, "complete");
    assert!(!dir.join(".onboarding-owner").exists());
    assert_eq!(reopened.engine.list_skills().unwrap().len(), 2);
}

#[test]
fn returning_to_the_questions_discards_the_old_draft_but_keeps_answers() {
    let h = harness();
    let state = propose(&h);
    let next = h.engine.save_onboarding(OnboardingProgress { stage: "profile".into(), ..state.progress }).unwrap();
    assert!(next.proposal.is_none());
    assert_eq!(next.progress.profile.job, "Researcher");
    assert!(next.progress.selected.is_empty());
}

#[tokio::test]
async fn finishing_archives_only_onboarding_conversations_and_keeps_the_proposal_resumable() {
    let h = harness();
    let state = propose(&h);
    let master = h.engine.master_bot().unwrap().unwrap();
    let (other, _) = h.engine.start_session_with_message(&master.id, "An unrelated task", &[]).unwrap();
    h.engine.finish_onboarding().await.unwrap();
    let sessions = h.engine.sessions_of(&master.id).unwrap();
    assert!(sessions.iter().find(|s| Some(&s.id) == state.session_id.as_ref()).unwrap().archived_at.is_some());
    assert!(sessions.iter().find(|s| s.id == other.id).unwrap().archived_at.is_none());
    let p = h.engine.preview_onboarding(choose(&["Weekly report"])).unwrap();
    assert_eq!(h.engine.apply_onboarding(p.selected, &p.token).unwrap().state.status, "complete");
}
