mod support;
use sonata_core::bundled::{checkout_paragraph, master_instructions};
use sonata_core::model::{NewBot, NewSkill};
use sonata_core::{CoreError, Refusal};
use support::*;

#[tokio::test]
async fn setup_creates_only_master_and_its_standalone_skill() {
    let h = harness();
    h.engine.finish_setup(None).unwrap();
    let bots = h.engine.list_bots().await.unwrap();
    assert_eq!(bots.len(), 1);
    assert_eq!(bots[0].bot.handle, "master");
    assert_eq!(bots[0].bot.instructions, master_instructions(None));
    assert_eq!(bots[0].bot.skill_ids.len(), 1);
    let skills = h.engine.list_skills().unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].skill.slug, "sonata-master");
    assert!(h.engine.list_workflows().unwrap().is_empty());
    assert!(h.engine.setup_done().unwrap());
    h.engine.finish_setup(None).unwrap();
    assert!(h.engine.ensure_default_bots().unwrap().is_empty());
    assert_eq!(h.engine.list_bots().await.unwrap().len(), 1);
}

#[tokio::test]
async fn invalid_project_folder_is_rejected_before_seeding() {
    let h = harness();
    for path in [std::path::PathBuf::from("relative"), h.paths.data_dir.join("missing")] {
        assert!(matches!(h.engine.finish_setup(Some(&path)), Err(CoreError::Refused(Refusal::BadFolder(_)))));
        assert!(!h.engine.setup_done().unwrap());
        assert!(h.engine.list_bots().await.unwrap().is_empty());
        assert!(h.engine.list_skills().unwrap().is_empty());
    }
    assert!(h.engine.ensure_default_bots().unwrap().is_empty());
}

#[tokio::test]
async fn failed_setup_retries_without_duplicate_skills_or_partial_master_identity() {
    let h = harness_after(|_| {});
    let db = rusqlite::Connection::open(h.paths.db_path()).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_master BEFORE INSERT ON bots BEGIN SELECT RAISE(FAIL, 'refused'); END;")
        .unwrap();
    assert!(h.engine.finish_setup(None).is_err());
    assert!(!h.engine.setup_done().unwrap());
    assert_eq!(h.engine.master_id().unwrap(), None);
    assert_eq!(h.engine.list_skills().unwrap().len(), 1);
    db.execute_batch("DROP TRIGGER refuse_master;").unwrap();
    h.engine.finish_setup(None).unwrap();
    assert_eq!(h.engine.list_bots().await.unwrap().len(), 1);
    assert_eq!(h.engine.list_skills().unwrap().len(), 1);
}

#[tokio::test]
async fn deleting_master_does_not_recreate_it_on_relaunch() {
    let h = harness_after(|_| {});
    h.engine.finish_setup(None).unwrap();
    let master = h.engine.master_bot().unwrap().unwrap();
    h.engine.delete_bot(&master.id).await.unwrap();
    let again = relaunch(&h);
    assert!(again.engine.ensure_default_bots().unwrap().is_empty());
    assert!(again.engine.master_bot().unwrap().is_none());
    assert_eq!(again.engine.master_id().unwrap(), Some(master.id));
}

#[tokio::test]
async fn project_folder_updates_generated_instructions_and_keeps_custom_ones() {
    let h = harness();
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    h.engine.finish_setup(Some(first.path())).unwrap();
    let helper = h
        .engine
        .create_bot(NewBot {
            instructions: format!("Help.\n\n{}", checkout_paragraph(Some(first.path()))),
            ..claude_bot("Helper")
        })
        .unwrap();
    let master = h.engine.master_bot().unwrap().unwrap();
    h.engine
        .update_bot(
            &master.id,
            NewBot { instructions: "My own instructions.".into(), skill_ids: master.skill_ids, ..claude_bot("Master") },
        )
        .unwrap();
    assert_eq!(h.engine.set_project_folder(second.path()).unwrap(), ["master"]);
    assert_eq!(h.engine.project_folder().unwrap().as_deref(), Some(second.path()));
    assert_eq!(h.engine.master_bot().unwrap().unwrap().instructions, "My own instructions.");
    let bots = h.engine.list_bots().await.unwrap();
    assert!(bots
        .iter()
        .find(|v| v.bot.id == helper.id)
        .unwrap()
        .bot
        .instructions
        .contains(&checkout_paragraph(Some(second.path()))));
}

#[tokio::test]
async fn relaunch_preserves_a_user_edited_master_skill() {
    let h = harness_after(|_| {});
    h.engine.finish_setup(None).unwrap();
    let skill = h.engine.list_skills().unwrap().remove(0).skill;
    h.engine
        .update_skill(
            &skill.id,
            NewSkill { name: skill.name, description: skill.description, body: "My own playbook.".into() },
        )
        .unwrap();
    let again = relaunch(&h);
    again.engine.ensure_default_bots().unwrap();
    assert_eq!(again.engine.skill_body(&skill.id).unwrap().trim(), "My own playbook.");
}
