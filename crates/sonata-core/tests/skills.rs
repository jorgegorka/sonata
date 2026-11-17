mod support;

use sonata_core::model::*;
use sonata_core::{CoreError, Refusal};
use support::*;

#[tokio::test]
async fn a_written_skill_gets_a_folder_a_file_and_a_unique_slug() {
    let h = harness();
    let first = h.engine.create_skill(skill("Weekly report", "# Steps\n")).unwrap();
    assert_eq!(
        (first.slug.as_str(), first.name.as_str(), first.source),
        ("weekly-report", "Weekly report", SkillSource::Written)
    );
    let text = std::fs::read_to_string(h.paths.skills_dir().join("weekly-report/SKILL.md")).unwrap();
    assert_eq!(text, "---\nname: weekly-report\ndescription: \"Use for Weekly report.\"\n---\n# Steps\n");
    let second = h.engine.create_skill(skill("Weekly report", "other")).unwrap();
    assert_eq!(second.slug, "weekly-report-2");
    assert_eq!(h.engine.skill_body(&second.id).unwrap(), "other");
    let listed: Vec<String> = h.engine.list_skills().unwrap().into_iter().map(|v| v.skill.slug).collect();
    assert_eq!(listed, vec!["weekly-report".to_string(), "weekly-report-2".to_string()]);
    let err = h.engine.create_skill(NewSkill { name: "  ".into(), ..skill("x", "") }).unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::SkillNameRequired)), "{err}");
    let err = h.engine.create_skill(NewSkill { description: " ".into(), ..skill("x", "") }).unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::SkillDescriptionRequired)), "{err}");
    let long = NewSkill { description: "d".repeat(1025), ..skill("x", "") };
    let err = h.engine.create_skill(long).unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::SkillBadFrontmatter(_))), "{err}");
}

#[tokio::test]
async fn importing_the_same_skill_twice_gets_a_second_slug_and_a_matching_name() {
    let h = harness();
    let folder = skill_folder(h.paths.data_dir.parent().unwrap(), "PDF Tools");
    let first = h.engine.import_skill(&folder).unwrap();
    assert_eq!(
        (first.slug.as_str(), first.name.as_str(), first.source),
        ("pdf-tools", "PDF Tools", SkillSource::Imported)
    );
    let copy = h.paths.skills_dir().join("pdf-tools");
    assert!(copy.join("scripts/run.sh").is_file(), "extra files are copied");
    assert!(std::fs::read_to_string(copy.join("SKILL.md")).unwrap().starts_with("---\nname: pdf-tools\n"));
    let original = std::fs::read_to_string(folder.join("SKILL.md")).unwrap();
    assert!(original.starts_with("---\nname: PDF Tools\n"), "the original is untouched");
    assert_eq!(h.engine.skill_body(&first.id).unwrap(), "\n# PDF Tools");
    let second = h.engine.import_skill(&folder).unwrap();
    assert_eq!(second.slug, "pdf-tools-2");
    let second_file = std::fs::read_to_string(h.paths.skills_dir().join("pdf-tools-2/SKILL.md")).unwrap();
    assert!(second_file.starts_with("---\nname: pdf-tools-2\n"));
    let err = h.engine.import_skill(&h.paths.bots_root).unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::SkillNoFile(_))), "{err}");
}

#[tokio::test]
async fn editing_a_skill_rewrites_its_file_and_restarts_only_the_bots_that_have_it() {
    let h = harness();
    let audit = h.engine.create_skill(skill("Audit", "Check everything.")).unwrap();
    let with = h.engine.create_bot(NewBot { skill_ids: vec![audit.id.clone()], ..claude_bot("Researcher") }).unwrap();
    let without = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let edit = NewSkill { name: "Audit".into(), description: "Use for audits.".into(), body: "Check twice.".into() };
    let edited = h.engine.update_skill(&audit.id, edit.clone()).unwrap();
    assert_eq!(edited.description, "Use for audits.");
    assert_eq!(h.engine.skill_body(&audit.id).unwrap(), "Check twice.");
    let version = |bots: &[BotView], id: &str| bots.iter().find(|b| b.bot.id == id).unwrap().bot.config_version;
    let bots = h.engine.list_bots().await.unwrap();
    assert_eq!((version(&bots, &with.id), version(&bots, &without.id)), (2, 1));
    h.engine.update_skill(&audit.id, edit).unwrap();
    let bots = h.engine.list_bots().await.unwrap();
    assert_eq!(version(&bots, &with.id), 2, "the same text again is no change");
    let err = h.engine.update_skill("missing", skill("x", "")).unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::SkillNotFound(_))), "{err}");
}

#[tokio::test]
async fn a_skill_a_bot_has_cannot_be_deleted_and_a_free_one_returns_its_folder() {
    let h = harness();
    let audit = h.engine.create_skill(skill("Audit", "")).unwrap();
    let bot = h.engine.create_bot(NewBot { skill_ids: vec![audit.id.clone()], ..claude_bot("Researcher") }).unwrap();
    let err = h.engine.delete_skill(&audit.id).unwrap_err();
    let researcher = vec!["researcher".to_string()];
    assert!(matches!(&err, CoreError::Refused(Refusal::SkillInUse(bots)) if bots == &researcher), "{err}");
    h.engine.update_bot(&bot.id, claude_bot("Researcher")).unwrap();
    assert_eq!(h.engine.delete_skill(&audit.id).unwrap(), h.paths.skills_dir().join("audit"));
    assert!(h.engine.list_skills().unwrap().is_empty());
    let err = h.engine.delete_skill(&audit.id).unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::SkillNotFound(_))), "{err}");
    let err = h.engine.create_bot(NewBot { skill_ids: vec!["missing".into()], ..claude_bot("Writer") }).unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::SkillNotFound(_))), "{err}");
    let err = h.engine.update_bot(&bot.id, NewBot { skill_ids: vec!["missing".into()], ..claude_bot("Researcher") });
    assert!(matches!(err.unwrap_err(), CoreError::Refused(Refusal::SkillNotFound(_))));
}

#[tokio::test]
async fn saving_an_unchanged_body_that_ends_in_a_newline_restarts_no_bot() {
    let h = harness();
    let audit = h.engine.create_skill(skill("Audit", "Check.\n")).unwrap();
    let bot = h.engine.create_bot(NewBot { skill_ids: vec![audit.id.clone()], ..claude_bot("Researcher") }).unwrap();
    let path = h.paths.skills_dir().join("audit/SKILL.md");
    let before = std::fs::read_to_string(&path).unwrap();
    for _ in 0..2 {
        h.engine.update_skill(&audit.id, skill("Audit", "Check.\n")).unwrap();
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "the file is identical");
    let version = |bots: Vec<BotView>| bots.into_iter().find(|b| b.bot.id == bot.id).unwrap().bot.config_version;
    assert_eq!(version(h.engine.list_bots().await.unwrap()), 1, "an unchanged body is no change");
    h.engine.update_skill(&audit.id, skill("Audit", "Check twice.\n")).unwrap();
    assert_eq!(version(h.engine.list_bots().await.unwrap()), 2, "a real change restarts the bot");
}

fn library(h: &Harness) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(h.paths.skills_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn orphan(h: &Harness, slug: &str) -> std::path::PathBuf {
    let dir = h.paths.skills_dir().join(slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SKILL.md"), "left by hand\n").unwrap();
    dir
}

#[tokio::test]
async fn an_orphan_folder_in_the_library_is_never_adopted() {
    let h = harness();
    let left = orphan(&h, "weekly-report");
    let created = h.engine.create_skill(skill("Weekly report", "# Steps\n")).unwrap();
    assert_eq!(created.slug, "weekly-report-2");
    assert_eq!(std::fs::read_to_string(left.join("SKILL.md")).unwrap(), "left by hand\n");
}

#[tokio::test]
async fn a_library_that_cannot_be_listed_takes_no_new_skill() {
    use std::os::unix::fs::PermissionsExt;
    let h = harness();
    let left = orphan(&h, "audit");
    std::fs::set_permissions(h.paths.skills_dir(), std::fs::Permissions::from_mode(0o300)).unwrap();
    let result = h.engine.create_skill(skill("Audit", "Check."));
    std::fs::set_permissions(h.paths.skills_dir(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(result, Err(CoreError::Io(_))), "{result:?}");
    assert_eq!(std::fs::read_to_string(left.join("SKILL.md")).unwrap(), "left by hand\n");
    assert_eq!(library(&h), vec!["audit".to_string()]);
}

#[tokio::test]
async fn a_failed_create_removes_only_the_folder_it_made() {
    let h = harness_after(|_| {});
    let left = orphan(&h, "audit");
    rusqlite::Connection::open(h.paths.db_path())
        .unwrap()
        .execute_batch("CREATE TRIGGER no_skills BEFORE INSERT ON skills BEGIN SELECT RAISE(FAIL, 'refused'); END;")
        .unwrap();
    let err = h.engine.create_skill(skill("Audit", "Check.")).unwrap_err();
    assert!(matches!(err, CoreError::Db(_)), "{err}");
    assert_eq!(library(&h), vec!["audit".to_string()]);
    assert_eq!(std::fs::read_to_string(left.join("SKILL.md")).unwrap(), "left by hand\n");
}

#[tokio::test]
async fn a_failed_import_removes_only_the_folder_it_made() {
    use std::os::unix::fs::PermissionsExt;
    let h = harness();
    let left = orphan(&h, "pdf-tools");
    let folder = skill_folder(h.paths.data_dir.parent().unwrap(), "PDF Tools");
    let locked = folder.join("scripts/run.sh");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let err = h.engine.import_skill(&folder).unwrap_err();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(err, CoreError::Io(_)), "{err}");
    assert_eq!(library(&h), vec!["pdf-tools".to_string()]);
    assert_eq!(std::fs::read_to_string(left.join("SKILL.md")).unwrap(), "left by hand\n");
    assert!(h.engine.list_skills().unwrap().is_empty());
}

#[tokio::test]
async fn skills_created_at_once_with_one_name_each_get_their_own_folder() {
    let h = harness();
    let start = std::sync::Barrier::new(4);
    let mut slugs: Vec<String> = std::thread::scope(|s| {
        let calls: Vec<_> = (0..4)
            .map(|_| {
                s.spawn(|| {
                    start.wait();
                    h.engine.create_skill(skill("Audit", "Check."))
                })
            })
            .collect();
        calls.into_iter().map(|call| call.join().unwrap().unwrap().slug).collect()
    });
    slugs.sort();
    assert_eq!(slugs, ["audit", "audit-2", "audit-3", "audit-4"]);
    for slug in &slugs {
        let text = std::fs::read_to_string(h.paths.skills_dir().join(slug).join("SKILL.md")).unwrap();
        assert!(text.starts_with(&format!("---\nname: {slug}\n")), "{slug}: {text}");
    }
}

#[tokio::test]
async fn an_attached_skill_reaches_the_bot_with_its_next_message_and_a_detached_one_goes() {
    let h = harness();
    let audit = h.engine.create_skill(skill("Audit", "Check everything.")).unwrap();
    let (bot, _) = bot_with_delivered(&h, "first").await;
    let dir = h.paths.bot_dir("researcher");
    assert!(!dir.join(".claude/skills").exists());
    h.engine.handle_bridge(reply("w1:p1", "done")).await;
    finish_turn(&h, "w1:p1").await;

    h.engine.update_bot(&bot.id, NewBot { skill_ids: vec![audit.id.clone()], ..claude_bot("Researcher") }).unwrap();
    h.engine.queue_user_message(&bot.id, "second").unwrap();
    h.engine.pump(&bot.id).await;
    assert_eq!(h.rt.state().starts.len(), 2, "the bot restarted to read its new skill");
    let copy = dir.join(".claude/skills/audit/SKILL.md");
    assert!(std::fs::read_to_string(&copy).unwrap().contains("Check everything."));
    let agents = std::fs::read_to_string(dir.join("AGENTS.md")).unwrap();
    assert!(agents.contains("- **Audit**: Use for Audit. — `.claude/skills/audit/SKILL.md`"), "{agents}");
    h.engine.handle_bridge(reply("w1:p2", "done")).await;
    finish_turn(&h, "w1:p2").await;

    let edit = NewSkill { name: "Audit".into(), description: "Use for Audit.".into(), body: "Check twice.".into() };
    h.engine.update_skill(&audit.id, edit).unwrap();
    h.engine.queue_user_message(&bot.id, "third").unwrap();
    h.engine.pump(&bot.id).await;
    assert_eq!(h.rt.state().starts.len(), 3, "an edited skill restarts the bot too");
    assert!(std::fs::read_to_string(&copy).unwrap().contains("Check twice."));
    h.engine.handle_bridge(reply("w1:p3", "done")).await;
    finish_turn(&h, "w1:p3").await;

    h.engine.update_bot(&bot.id, claude_bot("Researcher")).unwrap();
    h.engine.queue_user_message(&bot.id, "fourth").unwrap();
    h.engine.pump(&bot.id).await;
    assert!(!dir.join(".claude/skills/audit").exists(), "the detached skill's copy is gone");
    assert!(!std::fs::read_to_string(dir.join("AGENTS.md")).unwrap().contains("Your skills"));
}

#[tokio::test]
async fn a_skill_whose_library_folder_is_gone_is_left_out_and_the_bot_still_starts() {
    let h = harness();
    let audit = h.engine.create_skill(skill("Audit", "Check everything.")).unwrap();
    let brief = h.engine.create_skill(skill("Brief", "Keep it short.")).unwrap();
    let ids = vec![audit.id.clone(), brief.id.clone()];
    let bot = h.engine.create_bot(NewBot { skill_ids: ids, ..claude_bot("Researcher") }).unwrap();
    let library_folder = h.paths.skills_dir().join("audit");
    std::fs::remove_dir_all(&library_folder).unwrap();

    h.engine.queue_user_message(&bot.id, "hello").unwrap();
    h.engine.pump(&bot.id).await;

    assert_eq!(h.rt.state().starts.len(), 1, "the bot started");
    assert_eq!(h.rt.state().prompts.len(), 1, "the message was delivered");
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::Working);
    let notices: Vec<Message> =
        h.engine.user_thread(&bot.id).unwrap().into_iter().filter(|m| m.system_code.is_some()).collect();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert_eq!(notices[0].system_code.as_deref(), Some("skill_folder_missing"));
    let params = notices[0].system_params.clone().unwrap();
    assert_eq!(params["name"], "Audit");
    assert_eq!(params["path"], library_folder.to_string_lossy().as_ref());
    let dir = h.paths.bot_dir("researcher");
    assert!(!dir.join(".claude/skills/audit").exists(), "no copy of the missing skill");
    assert!(dir.join(".claude/skills/brief/SKILL.md").is_file(), "the other skill is copied");
    let agents = std::fs::read_to_string(dir.join("AGENTS.md")).unwrap();
    assert!(!agents.contains("**Audit**") && agents.contains("**Brief**"), "{agents}");
    let err = h.engine.skill_body(&audit.id).unwrap_err();
    assert!(matches!(&err, CoreError::Refused(Refusal::SkillNoFile(path)) if path.contains("audit")), "{err}");
}
