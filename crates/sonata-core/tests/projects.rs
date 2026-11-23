mod support;
use bridge_proto::{BridgeCommand, BridgeRequest};
use sonata_core::model::*;
use sonata_core::store::Store;
use support::*;

fn input(name: &str, directory: &std::path::Path, provider: Provider) -> ProjectInput {
    ProjectInput {
        name: name.into(),
        default_directory: Some(directory.display().to_string()),
        default_provider: provider,
    }
}

#[test]
fn project_creation_rolls_back_if_master_cannot_be_saved() {
    let h = harness_after(|_| {});
    let conn = rusqlite::Connection::open(h.paths.db_path()).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER reject_master BEFORE INSERT ON bots BEGIN SELECT RAISE(ABORT, 'simulated write failure'); END;",
    )
    .unwrap();
    let checkout = tempfile::tempdir().unwrap();
    assert!(h.engine.create_project(input("Failing", checkout.path(), Provider::Codex)).is_err());
    assert_eq!(h.engine.list_projects().unwrap().len(), 1);
    let store = Store::open(&h.paths.db_path()).unwrap();
    assert!(store.list_bots().unwrap().is_empty());
    assert_eq!(std::fs::read_dir(h.paths.bots_root.join(".projects")).unwrap().count(), 0);
}

#[tokio::test]
async fn teams_have_local_handles_workspaces_and_persistent_card_ownership() {
    let h = harness_after(|_| {});
    h.engine.finish_setup(None).unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let project = h.engine.create_project(input("Second", checkout.path(), Provider::Codex)).unwrap();
    let second = h.engine.for_project(&project.id).unwrap();
    let first_master = h.engine.master_bot().unwrap().unwrap();
    let master = second.master_bot().unwrap().unwrap();
    assert_eq!(master.handle, first_master.handle);
    assert_eq!(master.provider, Provider::Codex);
    assert_eq!(second.list_bots().await.unwrap().len(), 1);
    assert_eq!(h.engine.bot_folder(&first_master.id).unwrap(), h.paths.bots_root.join(&first_master.handle));
    assert_eq!(
        second.bot_folder(&master.id).unwrap(),
        h.paths.bots_root.join(".projects").join(&project.id).join(&master.handle)
    );
    let msg = second.queue_user_message(&master.id, "Use this project").unwrap();
    let session = second.sessions_of(&master.id).unwrap().remove(0);
    assert_eq!(session.project_id, project.id);
    assert_eq!(msg.recipient_session.as_deref(), Some(session.id.as_str()));
    h.engine.pump_session(&session.id).await;
    let starts = h.rt.state().starts.clone();
    assert!(starts.last().unwrap().cwd.starts_with(h.paths.bots_root.join(".projects").join(&project.id)));
    assert!(second.queue_user_message(&first_master.id, "wrong project").is_err());
    assert!(second
        .session_messages(
            &h.engine.sessions_of(&first_master.id).unwrap().first().map(|s| s.id.clone()).unwrap_or_default()
        )
        .is_err());
    let store = Store::open(&h.paths.db_path()).unwrap();
    assert_eq!(store.get_session(&session.id).unwrap().unwrap().project_id, project.id);
    assert!(store.scoped("default").get_bot(&master.id).unwrap().is_none());
    let mut updates = h.engine.subscribe_ui();
    let completed = second.archive_session(&session.id).await.unwrap();
    assert_eq!(completed[0].project_id, project.id);
    let restored = second.restore_session(&session.id).await.unwrap();
    assert_eq!(restored[0].project_id, project.id);
    let changed: Vec<_> = events(&mut updates)
        .into_iter()
        .filter_map(|event| match event {
            sonata_core::engine::UiEvent::SessionUpdated { session } => Some(session.session),
            _ => None,
        })
        .collect();
    assert_eq!(changed.len(), 2);
    assert!(changed.iter().all(|session| session.project_id == project.id));
}

#[tokio::test]
async fn omitted_cli_provider_uses_the_callers_project_and_delegation_is_local() {
    let h = harness();
    let checkout = tempfile::tempdir().unwrap();
    let project = h.engine.create_project(input("Second", checkout.path(), Provider::Codex)).unwrap();
    let second = h.engine.for_project(&project.id).unwrap();
    let master = second.master_bot().unwrap().unwrap();
    let other = h.engine.create_bot(claude_bot("Private helper")).unwrap();
    second.queue_user_message(&master.id, "Create the approved helper").unwrap();
    second.pump(&master.id).await;
    let session = second.sessions_of(&master.id).unwrap().remove(0);
    let pane = pane(&h, &session);
    let request = |command| BridgeRequest { v: 1, pane_id: pane.clone(), cli_session_id: None, command };
    let result = h
        .engine
        .handle_bridge(request(BridgeCommand::BotCreate {
            name: "Helper".into(),
            color: "#123456".into(),
            provider: "".into(),
            skills: vec![],
            instructions: "Help.".into(),
            recreate: false,
            model: None,
        }))
        .await;
    assert!(result.ok, "{result:?}");
    let helper = second.list_bots().await.unwrap().into_iter().find(|b| b.bot.handle == "helper").unwrap().bot;
    assert_eq!(helper.provider, Provider::Codex);
    let denied = h
        .engine
        .handle_bridge(request(BridgeCommand::Send { to: other.handle, body: "Cross project".into(), files: vec![] }))
        .await;
    assert!(!denied.ok);
    let created = second.create_bot(claude_bot("Explicit")).unwrap();
    assert_eq!(created.provider, Provider::Claude);
    h.engine.update_project(&project.id, input("Second", checkout.path(), Provider::Opencode)).unwrap();
    assert_eq!(
        second.list_bots().await.unwrap().iter().find(|b| b.bot.id == helper.id).unwrap().bot.provider,
        Provider::Codex
    );
}

#[tokio::test]
async fn archive_refuses_work_preserves_cards_and_restores_future_schedules() {
    let h = harness();
    let checkout = tempfile::tempdir().unwrap();
    let p = h.engine.create_project(input("Second", checkout.path(), Provider::Claude)).unwrap();
    let engine = h.engine.for_project(&p.id).unwrap();
    let bot = engine.master_bot().unwrap().unwrap();
    let task = engine
        .create_task(NewTask {
            name: "Daily".into(),
            bot_id: bot.id.clone(),
            prompt: "Check".into(),
            every_days: 1,
            at: "09:00".into(),
            starts_on: None,
        })
        .unwrap();
    engine.queue_user_message(&bot.id, "queued").unwrap();
    let session = engine.sessions_of(&bot.id).unwrap().remove(0);
    assert!(h.engine.archive_project(&p.id).await.is_err());
    engine.pump(&bot.id).await;
    assert!(h.engine.archive_project(&p.id).await.is_err());
    engine.stop_session(&session.id).await.unwrap();
    h.engine.archive_project(&p.id).await.unwrap();
    assert!(engine.queue_user_message(&bot.id, "refused").is_err());
    assert!(engine.run_task_now(&task.id).is_err());
    assert!(engine.sessions_of(&bot.id).unwrap().iter().all(|s| s.archived_at.is_none()));
    let now = chrono::DateTime::parse_from_rfc3339("2030-01-01T10:00:00Z").unwrap().with_timezone(&chrono::Utc);
    h.engine.set_clock(std::sync::Arc::new(move || now));
    h.engine.scheduler_tick().await;
    assert_eq!(engine.sessions_of(&bot.id).unwrap().len(), 1);
    h.engine.restore_project(&p.id).unwrap();
    let restored = engine.list_tasks().unwrap().remove(0);
    assert!(restored.enabled);
    assert!(chrono::DateTime::parse_from_rfc3339(&restored.next_run_at.unwrap()).unwrap() > now);
    h.engine.scheduler_tick().await;
    assert_eq!(engine.sessions_of(&bot.id).unwrap().len(), 1);
}

#[tokio::test]
async fn importing_and_directory_edits_are_scoped_and_preserve_custom_instructions() {
    let h = harness();
    let one = tempfile::tempdir().unwrap();
    let two = tempfile::tempdir().unwrap();
    let a = h.engine.create_project(input("A", one.path(), Provider::Claude)).unwrap();
    let b = h.engine.create_project(input("B", two.path(), Provider::Codex)).unwrap();
    let ea = h.engine.for_project(&a.id).unwrap();
    let eb = h.engine.for_project(&b.id).unwrap();
    let custom = ea.create_bot(claude_bot("Custom")).unwrap();
    let other_before = eb.master_bot().unwrap().unwrap();
    h.engine.update_project(&a.id, input("Renamed", two.path(), Provider::Codex)).unwrap();
    assert_eq!(eb.master_bot().unwrap().unwrap(), other_before);
    assert_eq!(
        ea.list_bots().await.unwrap().iter().find(|b| b.bot.id == custom.id).unwrap().bot.instructions,
        custom.instructions
    );
    let file = one.path().join("team.yml");
    ea.write_config(&file).unwrap();
    let preview_a = ea.preview_config_import(&file).unwrap();
    let preview_b = eb.preview_config_import(&file).unwrap();
    assert_ne!(preview_a.token, preview_b.token);
    let imported = eb.import_config(&file, sonata_core::engine::ImportMode::AddNew, &preview_b.token).unwrap();
    assert!(format!("{imported:?}").contains("Done"));
    assert_eq!(ea.list_bots().await.unwrap().len(), 2);
    assert!(eb.list_bots().await.unwrap().iter().all(|bot| bot.bot.project_id == b.id));
}

#[tokio::test]
async fn names_and_references_are_project_local() {
    let h = harness();
    let dir = tempfile::tempdir().unwrap();
    let a = h.engine.create_project(input("A", dir.path(), Provider::Claude)).unwrap();
    let b = h.engine.create_project(input("B", dir.path(), Provider::Claude)).unwrap();
    assert!(h.engine.create_project(input(" a ", dir.path(), Provider::Claude)).is_err());
    let ea = h.engine.for_project(&a.id).unwrap();
    let eb = h.engine.for_project(&b.id).unwrap();
    for e in [&ea, &eb] {
        e.create_task(NewTask {
            name: "Daily".into(),
            bot_id: e.master_id().unwrap().unwrap(),
            prompt: "Check".into(),
            every_days: 1,
            at: "09:00".into(),
            starts_on: None,
        })
        .unwrap();
    }
    let wrong = NewTask {
        name: "Wrong".into(),
        bot_id: b.master_id.clone().unwrap(),
        prompt: "Check".into(),
        every_days: 1,
        at: "09:00".into(),
        starts_on: None,
    };
    assert!(ea.create_task(wrong).is_err());
    assert_eq!(ea.list_tasks().unwrap().len(), 1);
    assert_eq!(eb.list_tasks().unwrap().len(), 1);
    assert!(h.engine.request_project(Some(&a.id), &[("bots", b.master_id.as_deref().unwrap())]).is_err());
}

fn flow(bot: &Bot) -> NewWorkflow {
    NewWorkflow {
        name: "Flow".into(),
        when_to_use: String::new(),
        def: WorkflowDef {
            version: WORKFLOW_FORMAT,
            start: Some("step".into()),
            start_pos: None,
            done_pos: None,
            steps: vec![Step {
                id: "step".into(),
                name: "Work".into(),
                bot_id: bot.id.clone(),
                instructions: "Work.".into(),
                arrows: vec![Arrow { to: DONE.into(), outcome: None }],
                pos: None,
            }],
        },
    }
}

#[tokio::test]
async fn workflows_and_shared_skill_import_previews_respect_project_ownership() {
    let h = harness();
    let dir = tempfile::tempdir().unwrap();
    let a = h.engine.create_project(input("A", dir.path(), Provider::Claude)).unwrap();
    let b = h.engine.create_project(input("B", dir.path(), Provider::Claude)).unwrap();
    let ea = h.engine.for_project(&a.id).unwrap();
    let eb = h.engine.for_project(&b.id).unwrap();
    let skill = ea
        .create_skill(NewSkill {
            name: "Shared".into(),
            description: "Used by both projects".into(),
            body: "Help.".into(),
        })
        .unwrap();
    let mut new = claude_bot("Helper");
    new.skill_ids.push(skill.id.clone());
    let ba = ea.create_bot(new.clone()).unwrap();
    let bb = eb.create_bot(new).unwrap();
    let wa = ea.create_workflow(flow(&ba)).unwrap();
    let wb = eb.create_workflow(flow(&bb)).unwrap();
    assert!(ea.create_workflow(flow(&bb)).is_err());
    assert!(ea.start_workflow(&wb.id, "Wrong project", &[]).is_err());
    let root = eb.start_workflow(&wb.id, "Project B request", &[]).unwrap();
    assert_eq!(root.project_id, b.id);
    assert!(h.engine.archive_project(&b.id).await.is_err());
    let file = dir.path().join("workflow.yaml");
    ea.write_workflow_config(&wa.id, &file).unwrap();
    let preview = ea
        .preview_workflow_import(
            &file,
            &sonata_core::engine::WorkflowImportRequest {
                workflow: "Flow".into(),
                workflow_choice: None,
                new_name: None,
                bots: Default::default(),
                skills: Default::default(),
            },
        )
        .unwrap();
    let usage = preview.items.iter().find(|item| item.item.name == skill.slug).unwrap();
    assert_eq!(usage.used_by_bots, vec!["helper · A", "helper · B"]);
    assert_eq!(usage.used_by_workflows, vec!["Flow · A", "Flow · B"]);
    assert_eq!(ea.list_workflows().unwrap(), vec![wa]);
    assert_eq!(eb.list_workflows().unwrap(), vec![wb]);
    let message = eb.session_messages(&root.id).unwrap().remove(0);
    assert!(ea.request_project(Some(&a.id), &[("messages", &message.id)]).is_err());
}

#[tokio::test]
async fn archive_does_not_interrupt_background_work_in_an_idle_session() {
    let h = harness();
    let dir = tempfile::tempdir().unwrap();
    let project = h.engine.create_project(input("Background", dir.path(), Provider::Claude)).unwrap();
    let engine = h.engine.for_project(&project.id).unwrap();
    let master = engine.master_bot().unwrap().unwrap();
    engine.queue_user_message(&master.id, "Run checks").unwrap();
    engine.pump(&master.id).await;
    let session = engine.sessions_of(&master.id).unwrap().remove(0);
    let pane = pane(&h, &session);
    assert!(h.engine.handle_bridge(reply(&pane, "Checks are running.")).await.ok);
    h.rt.state().screen = CLAUDE_SHELL_RUNNING.into();
    finish_turn(&h, &pane).await;
    assert!(h.engine.archive_project(&project.id).await.is_err());
    assert!(h.rt.state().stopped.is_empty());
    h.rt.state().screen = CLAUDE_NOTHING_RUNNING.into();
    h.engine.archive_project(&project.id).await.unwrap();
    assert_eq!(h.rt.state().stopped.len(), 1);
}

#[tokio::test]
async fn archive_reports_an_unreadable_screen_and_leaves_the_session_running() {
    let h = harness();
    let dir = tempfile::tempdir().unwrap();
    let project = h.engine.create_project(input("Unreadable", dir.path(), Provider::Claude)).unwrap();
    let engine = h.engine.for_project(&project.id).unwrap();
    let master = engine.master_bot().unwrap().unwrap();
    engine.queue_user_message(&master.id, "Run checks").unwrap();
    engine.pump(&master.id).await;
    let session = engine.sessions_of(&master.id).unwrap().remove(0);
    let pane = pane(&h, &session);
    assert!(h.engine.handle_bridge(reply(&pane, "Checks are running.")).await.ok);
    h.rt.state().screen = CLAUDE_NOTHING_RUNNING.into();
    finish_turn(&h, &pane).await;
    h.rt.state().snapshot_failures.push_back(sonata_core::runtime::RuntimeError::Other("unreadable".into()));
    let error = h.engine.archive_project(&project.id).await.unwrap_err();
    assert!(matches!(error, sonata_core::CoreError::Refused(sonata_core::Refusal::SnapshotFailed(_))));
    assert!(engine.current_project().unwrap().archived_at.is_none());
    assert!(h.rt.state().stopped.is_empty());
}

#[tokio::test(start_paused = true)]
async fn archive_reads_the_screen_without_holding_the_live_lock() {
    let h = harness();
    let dir = tempfile::tempdir().unwrap();
    let project = h.engine.create_project(input("Screen", dir.path(), Provider::Claude)).unwrap();
    let engine = h.engine.for_project(&project.id).unwrap();
    let master = engine.master_bot().unwrap().unwrap();
    engine.queue_user_message(&master.id, "Run checks").unwrap();
    engine.pump(&master.id).await;
    let session = engine.sessions_of(&master.id).unwrap().remove(0);
    let pane = pane(&h, &session);
    assert!(h.engine.handle_bridge(reply(&pane, "Checks are running.")).await.ok);
    h.rt.state().screen = CLAUDE_NOTHING_RUNNING.into();
    finish_turn(&h, &pane).await;
    h.rt.state().snapshot_delay = Some(std::time::Duration::from_secs(60));

    let archiving = h.engine.clone();
    let project_id = project.id.clone();
    let archive = tokio::spawn(async move { archiving.archive_project(&project_id).await });
    while h.rt.state().snapshots == 0 {
        if archive.is_finished() {
            panic!("archive finished before reading the screen: {:?}", archive.await.unwrap());
        }
        tokio::task::yield_now().await;
    }
    let listing = h.engine.clone();
    let listed = tokio::spawn(async move { listing.list_bots().await });
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert!(listed.is_finished(), "archive held the live lock while reading the screen");
    tokio::time::advance(std::time::Duration::from_secs(60)).await;
    assert!(archive.await.unwrap().is_ok());
    assert!(listed.await.unwrap().is_ok());
}
