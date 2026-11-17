mod support;
use sonata_core::config_file::{self, ConfigFile};
use sonata_core::engine::{
    DependencyChoice as Choice, ImportMode, ImportOutcome, ItemKind, WorkflowImportAction as Action,
    WorkflowImportOutcome as Outcome, WorkflowImportPreview, WorkflowImportRequest,
};
use sonata_core::model::{Arrow, Bot, NewBot, NewSkill, NewWorkflow, Pos, Skill, Step, Workflow, WorkflowDef};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use support::*;

fn skill(h: &Harness, name: &str) -> Skill {
    h.engine
        .create_skill(NewSkill {
            name: name.into(),
            description: format!("{name} description"),
            body: format!("{name} instructions"),
        })
        .unwrap()
}
fn bot(h: &Harness, name: &str, skills: &[Skill]) -> Bot {
    h.engine
        .create_bot(NewBot { skill_ids: skills.iter().map(|s| s.id.clone()).collect(), ..claude_bot(name) })
        .unwrap()
}
fn workflow(h: &Harness, name: &str, bots: &[Bot]) -> Workflow {
    h.engine
        .create_workflow(NewWorkflow {
            name: name.into(),
            when_to_use: "For tickets".into(),
            def: WorkflowDef {
                version: 1,
                start: Some("s0".into()),
                start_pos: Some(Pos { x: 10.0, y: 20.0 }),
                done_pos: None,
                steps: bots
                    .iter()
                    .enumerate()
                    .map(|(i, b)| Step {
                        id: format!("s{i}"),
                        name: format!("Step {i}"),
                        bot_id: b.id.clone(),
                        instructions: "Do the task".into(),
                        pos: None,
                        arrows: if i + 1 < bots.len() {
                            vec![Arrow { to: format!("s{}", i + 1), outcome: None }]
                        } else {
                            vec![]
                        },
                    })
                    .collect(),
            },
        })
        .unwrap()
}
fn package(h: &Harness, workflow: &Workflow) -> PathBuf {
    let path = h.paths.data_dir.join("workflow.yaml");
    h.engine.write_workflow_config(&workflow.id, &path).unwrap();
    path
}
fn request(name: &str) -> WorkflowImportRequest {
    WorkflowImportRequest { workflow: name.into(), ..Default::default() }
}
fn read(path: &Path) -> ConfigFile {
    config_file::read_file(path).unwrap().0
}
fn rewrite(path: &Path, change: impl FnOnce(&mut ConfigFile)) {
    let mut f = read(path);
    change(&mut f);
    std::fs::write(path, config_file::to_yaml(&f).unwrap()).unwrap();
}
fn preview(h: &Harness, path: &Path, r: &WorkflowImportRequest) -> WorkflowImportPreview {
    h.engine.preview_workflow_import(path, r).unwrap()
}
fn apply(h: &Harness, path: &Path, r: &WorkflowImportRequest) -> sonata_core::engine::ImportReport {
    let p = preview(h, path, r);
    assert!(p.can_import, "{p:#?}");
    match h.engine.import_workflow_config(path, r, &p.token).unwrap() {
        Outcome::Done { report } => {
            assert!(report.error.is_none(), "{report:?}");
            report
        }
        o => panic!("{o:?}"),
    }
}
fn item<'a>(p: &'a WorkflowImportPreview, kind: ItemKind, name: &str) -> &'a sonata_core::engine::WorkflowImportItem {
    p.items.iter().find(|i| i.kind == kind && i.item.name == name).unwrap()
}
fn edit_bot(h: &Harness, b: &Bot, skills: &[Skill], instructions: &str) {
    h.engine
        .update_bot(
            &b.id,
            NewBot {
                display_name: b.display_name.clone(),
                color: b.color.clone(),
                provider: b.provider,
                model: b.model.clone(),
                skill_ids: skills.iter().map(|s| s.id.clone()).collect(),
                instructions: instructions.into(),
            },
        )
        .unwrap();
}

#[tokio::test]
async fn export_is_the_exact_closure_and_round_trips_files_and_graph() {
    let a = harness();
    let shared = skill(&a, "shared");
    let unused = skill(&a, "unused");
    let b = bot(&a, "Review", std::slice::from_ref(&shared));
    let _other = bot(&a, "Other", std::slice::from_ref(&unused));
    let w = workflow(&a, "Review workflow", &[b.clone(), b]);
    workflow(&a, "Unrelated", &[_other]);
    std::fs::remove_dir_all(a.paths.skills_dir().join(&unused.slug)).unwrap();
    let dir = a.paths.skills_dir().join(&shared.slug);
    std::fs::write(dir.join("binary"), [0xff, 0, 0xfe]).unwrap();
    std::fs::write(dir.join("run.sh"), "#!/bin/sh\ntrue\n").unwrap();
    std::fs::set_permissions(dir.join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = package(&a, &w);
    let f = read(&path);
    assert_eq!((f.workflows.len(), f.bots.len(), f.skills.len()), (1, 1, 1));
    let b = harness();
    let r = request(&w.name);
    let result = apply(&b, &path, &r);
    assert_eq!(result.written.len(), 3);
    assert!(result.backup.is_none());
    let imported = b.engine.list_workflows().unwrap().remove(0);
    let out = b.engine.export_workflow_config(&imported.id).unwrap();
    assert_eq!(out.workflows, f.workflows);
    assert_eq!(out.skills, f.skills);
    assert_eq!(out.bots, f.bots);
    assert!(apply(&b, &path, &r).written.is_empty());
}

#[tokio::test]
async fn export_refuses_missing_bot_and_keeps_an_existing_destination() {
    let h = harness();
    let b = bot(&h, "Review", &[]);
    let w = workflow(&h, "Review", std::slice::from_ref(&b));
    let path = package(&h, &w);
    let bytes = std::fs::read(&path).unwrap();
    h.engine.delete_bot(&b.id).await.unwrap();
    assert!(h.engine.write_workflow_config(&w.id, &path).is_err());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[tokio::test]
async fn a_full_export_selects_only_one_workflow_and_its_dependencies() {
    let a = harness();
    let s = skill(&a, "shared");
    let b = bot(&a, "Review", &[s]);
    let w = workflow(&a, "Selected", &[b]);
    let s2 = skill(&a, "unused");
    let b2 = bot(&a, "Other", &[s2]);
    workflow(&a, "Unrelated", &[b2]);
    let path = a.paths.data_dir.join("all.yaml");
    a.engine.write_config(&path).unwrap();
    rewrite(&path, |f| f.skills.iter_mut().find(|s| s.slug == "unused").unwrap().files.clear());
    let b = harness();
    assert_eq!(b.engine.workflows_in_config(&path).unwrap().len(), 2);
    assert_eq!(apply(&b, &path, &request(&w.name)).written.len(), 3);
    assert_eq!(b.engine.list_workflows().unwrap().len(), 1);
}

#[tokio::test]
async fn keeping_a_local_bot_prunes_incoming_skills_and_does_not_pull_other_payloads() {
    let a = harness();
    let incoming = skill(&a, "incoming");
    let b = bot(&a, "Review", &[incoming]);
    let w = workflow(&a, "Selected", &[b]);
    let extra = skill(&a, "local-only");
    let other = bot(&a, "Other", &[extra]);
    workflow(&a, "Other", &[other]);
    let path = a.paths.data_dir.join("all.yaml");
    a.engine.write_config(&path).unwrap();
    let h = harness();
    rewrite(&path, |f| f.skills.iter_mut().find(|s| s.slug == "local-only").unwrap().files.clear());
    let local = skill(&h, "local-only");
    let b = bot(&h, "Review", std::slice::from_ref(&local));
    edit_bot(&h, &b, std::slice::from_ref(&local), "Local behavior");
    let mut r = request(&w.name);
    let p = preview(&h, &path, &r);
    assert!(!p.can_import);
    r.bots.insert("review".into(), Choice::KeepLocal);
    let p = preview(&h, &path, &r);
    assert!(p.can_import);
    assert!(!p.items.iter().any(|i| i.item.name == "incoming"));
    assert_eq!(item(&p, ItemKind::Skill, "local-only").action, Action::Reuse);
    let result = apply(&h, &path, &r);
    assert_eq!(result.written.len(), 1);
    assert_eq!(result.written[0].kind, ItemKind::Workflow);
}

#[tokio::test]
async fn identical_bots_still_require_skill_choices_and_show_shared_usage() {
    let a = harness();
    let s = skill(&a, "shared");
    let b = bot(&a, "Review", &[s]);
    let w = workflow(&a, "Selected", &[b]);
    let path = package(&a, &w);
    let h = harness();
    apply(&h, &path, &request(&w.name));
    let b = h.engine.list_bots().await.unwrap()[0].bot.clone();
    workflow(&h, "Other workflow", &[b]);
    rewrite(&path, |f| f.skills[0].files[0].text.as_mut().unwrap().push_str("\nChanged"));
    let mut r = request(&w.name);
    let p = preview(&h, &path, &r);
    assert_eq!(item(&p, ItemKind::Bot, "review").action, Action::Reuse);
    assert_eq!(item(&p, ItemKind::Skill, "shared").action, Action::Choose);
    assert!(item(&p, ItemKind::Skill, "shared").used_by_workflows.contains(&"Other workflow".into()));
    assert_eq!(item(&p, ItemKind::Skill, "shared").changed_files, ["SKILL.md"]);
    r.skills.insert("shared".into(), Choice::Replace);
    let report = apply(&h, &path, &r);
    assert_eq!(report.written.len(), 1);
    assert!(report.backup.is_some());
}

#[tokio::test]
async fn invalid_required_items_block_every_write_but_explicit_local_reuse_can_resolve_them() {
    let a = harness();
    let s = skill(&a, "shared");
    let b = bot(&a, "Review", &[s]);
    let w = workflow(&a, "Selected", &[b]);
    let path = package(&a, &w);
    rewrite(&path, |f| f.skills[0].files.clear());
    let h = harness();
    let r = request(&w.name);
    let p = preview(&h, &path, &r);
    assert!(!p.can_import);
    assert!(matches!(h.engine.import_workflow_config(&path, &r, &p.token).unwrap(), Outcome::Blocked { .. }));
    assert!(h.engine.list_bots().await.unwrap().is_empty());
    assert!(h.engine.list_workflows().unwrap().is_empty());
    skill(&h, "shared");
    let mut r = r;
    r.skills.insert("shared".into(), Choice::KeepLocal);
    assert_eq!(apply(&h, &path, &r).written.len(), 2);
}

#[tokio::test]
async fn invalid_incoming_bot_can_be_explicitly_kept_without_its_dependencies() {
    let a = harness();
    let s = skill(&a, "shared");
    let b = bot(&a, "Review", &[s]);
    let w = workflow(&a, "Selected", &[b]);
    let path = package(&a, &w);
    rewrite(&path, |f| {
        f.bots[0].provider = "future-provider".into();
        f.skills[0].files.clear();
    });
    let h = harness();
    bot(&h, "Review", &[]);
    let mut r = request(&w.name);
    assert!(!preview(&h, &path, &r).can_import);
    r.bots.insert("review".into(), Choice::KeepLocal);
    let p = preview(&h, &path, &r);
    assert!(p.can_import);
    assert!(!p.items.iter().any(|i| i.kind == ItemKind::Skill));
    assert_eq!(apply(&h, &path, &r).written.len(), 1);
}

#[tokio::test]
async fn rename_creates_only_a_workflow_and_rejects_case_insensitive_collisions() {
    let h = harness();
    let b = bot(&h, "Review", &[]);
    let w = workflow(&h, "Selected", &[b]);
    let path = package(&h, &w);
    let mut r = request(&w.name);
    r.new_name = Some("sELECTED".into());
    assert!(!preview(&h, &path, &r).can_import);
    r.new_name = Some("Alternative".into());
    let result = apply(&h, &path, &r);
    assert_eq!(result.written.len(), 1);
    assert!(!result.written[0].replaced);
    assert_eq!(h.engine.list_workflows().unwrap().len(), 2);
}

#[tokio::test]
async fn keeping_a_workflow_uses_its_local_graph() {
    let a = harness();
    let b = bot(&a, "Incoming", &[]);
    let w = workflow(&a, "Selected", &[b]);
    let path = package(&a, &w);
    let h = harness();
    let b = bot(&h, "Local", &[]);
    workflow(&h, &w.name, &[b]);
    let mut r = request(&w.name);
    r.workflow_choice = Some(Choice::KeepLocal);
    let p = preview(&h, &path, &r);
    assert!(p.can_import);
    assert!(!p.items.iter().any(|i| i.item.name == "incoming"));
    assert!(apply(&h, &path, &r).written.is_empty());
}

#[tokio::test]
async fn changing_an_already_different_bot_or_its_shared_usage_invalidates_the_token() {
    let a = harness();
    let b = bot(&a, "Review", &[]);
    let w = workflow(&a, "Selected", &[b]);
    let path = package(&a, &w);
    let h = harness();
    let b = bot(&h, "Review", &[]);
    edit_bot(&h, &b, &[], "Local one");
    let mut r = request(&w.name);
    r.bots.insert("review".into(), Choice::Replace);
    let p = preview(&h, &path, &r);
    edit_bot(&h, &b, &[], "Local two");
    assert!(matches!(h.engine.import_workflow_config(&path, &r, &p.token).unwrap(), Outcome::Changed { .. }));
    assert!(h.engine.list_workflows().unwrap().is_empty());
    let p = preview(&h, &path, &r);
    workflow(&h, "New usage", &[b]);
    assert!(matches!(h.engine.import_workflow_config(&path, &r, &p.token).unwrap(), Outcome::Changed { .. }));
}

#[tokio::test]
async fn changing_skill_bytes_modes_file_or_choices_invalidates_the_token() {
    let h = harness();
    let s = skill(&h, "shared");
    let b = bot(&h, "Review", std::slice::from_ref(&s));
    let w = workflow(&h, "Selected", &[b]);
    let path = package(&h, &w);
    rewrite(&path, |f| f.skills[0].files[0].text.as_mut().unwrap().push_str("\nIncoming"));
    let mut r = request(&w.name);
    r.skills.insert("shared".into(), Choice::Replace);
    let p = preview(&h, &path, &r);
    let skill_path = h.paths.skills_dir().join(&s.slug).join("SKILL.md");
    std::fs::set_permissions(&skill_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(h.engine.import_workflow_config(&path, &r, &p.token).unwrap(), Outcome::Changed { .. }));
    let p = preview(&h, &path, &r);
    r.skills.insert("shared".into(), Choice::KeepLocal);
    assert!(matches!(h.engine.import_workflow_config(&path, &r, &p.token).unwrap(), Outcome::Changed { .. }));
    let p = preview(&h, &path, &r);
    rewrite(&path, |f| f.exported_at = "changed".into());
    assert!(matches!(h.engine.import_workflow_config(&path, &r, &p.token).unwrap(), Outcome::Changed { .. }));
}

#[tokio::test]
async fn replacements_back_up_only_written_items_and_restore_with_existing_import() {
    let h = harness();
    let s = skill(&h, "shared");
    let b = bot(&h, "Review", &[s]);
    let w = workflow(&h, "Selected", &[b]);
    let path = package(&h, &w);
    let before = h.engine.export_config().unwrap();
    rewrite(&path, |f| {
        f.bots[0].instructions = "New instructions".into();
        f.workflows[0].when_to_use = "New use".into();
    });
    let mut r = request(&w.name);
    r.bots.insert("review".into(), Choice::Replace);
    r.workflow_choice = Some(Choice::Replace);
    let result = apply(&h, &path, &r);
    assert_eq!(result.written.len(), 2);
    let backup = PathBuf::from(result.backup.unwrap().kept_at.unwrap());
    let saved = read(&backup);
    assert!(saved.skills.is_empty());
    assert_eq!(saved.bots.len(), 1);
    assert_eq!(saved.workflows.len(), 1);
    let p = h.engine.preview_config_import(&backup).unwrap();
    assert!(matches!(
        h.engine.import_config(&backup, ImportMode::AddAndReplace, &p.token).unwrap(),
        ImportOutcome::Done { .. }
    ));
    let after = h.engine.export_config().unwrap();
    assert_eq!(before.bots, after.bots);
    assert_eq!(before.workflows, after.workflows);
}

#[tokio::test]
async fn master_is_exported_only_when_referenced_and_maps_to_the_local_master() {
    let a = harness();
    a.engine.finish_setup(None).unwrap();
    let master = a.engine.master_bot().unwrap().unwrap();
    let w = workflow(&a, "Master workflow", &[master]);
    let path = package(&a, &w);
    assert!(read(&path).bots[0].master);
    rewrite(&path, |f| {
        f.bots[0].handle = "remote-master".into();
        f.workflows[0].definition.steps[0].bot = Some("remote-master".into());
    });
    let h = harness();
    h.engine.finish_setup(None).unwrap();
    let r = request(&w.name);
    let p = preview(&h, &path, &r);
    assert_eq!(item(&p, ItemKind::Bot, "remote-master").item.target, "master");
    assert_eq!(apply(&h, &path, &r).written.len(), 1);
    let master = h.engine.master_bot().unwrap().unwrap();
    assert_eq!(
        h.engine.list_workflows().unwrap().iter().find(|w| w.name == "Master workflow").unwrap().def.steps[0].bot_id,
        master.id
    );
    assert!(apply(&h, &path, &r).written.is_empty());
    let helper = bot(&a, "Helper", &[]);
    let seeded = workflow(&a, "User-created workflow", &[helper]);
    assert!(!a.engine.export_workflow_config(&seeded.id).unwrap().bots.iter().any(|b| b.master));
}

#[tokio::test]
async fn missing_required_skill_blocks_export_and_bot_folder_collisions_resolve_in_the_graph() {
    let a = harness();
    let s = skill(&a, "shared");
    let b = bot(&a, "Review", std::slice::from_ref(&s));
    let w = workflow(&a, "Selected", &[b]);
    let path = package(&a, &w);
    let h = harness();
    std::fs::create_dir_all(h.paths.bots_root.join("review")).unwrap();
    let r = request(&w.name);
    let p = preview(&h, &path, &r);
    assert_eq!(item(&p, ItemKind::Bot, "review").item.target, "review-2");
    apply(&h, &path, &r);
    let bot = h.engine.list_bots().await.unwrap().remove(0).bot;
    assert_eq!(h.engine.list_workflows().unwrap()[0].def.steps[0].bot_id, bot.id);
    std::fs::remove_dir_all(a.paths.skills_dir().join(s.slug)).unwrap();
    assert!(a.engine.export_workflow_config(&w.id).is_err());
}

#[tokio::test]
async fn a_non_master_collision_with_master_changes_the_workflows_bot_and_requires_a_choice() {
    let h = harness();
    h.engine.finish_setup(None).unwrap();
    let master = h.engine.master_bot().unwrap().unwrap();
    let w = workflow(&h, "Selected", &[master]);
    let path = package(&h, &w);
    rewrite(&path, |f| f.bots[0].master = false);
    let p = preview(&h, &path, &request(&w.name));
    assert_eq!(item(&p, ItemKind::Bot, "master").item.target, "master-2");
    assert_eq!(item(&p, ItemKind::Workflow, &w.name).action, Action::Choose);
}

#[tokio::test]
async fn backup_failure_writes_nothing_and_write_failure_reports_completed_items() {
    let a = harness();
    let s = skill(&a, "shared");
    let b = bot(&a, "Review", &[s]);
    let w = workflow(&a, "Selected", &[b]);
    let path = package(&a, &w);
    let h = harness();
    let r = request(&w.name);
    let p = preview(&h, &path, &r);
    std::fs::set_permissions(&h.paths.bots_root, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = h.engine.import_workflow_config(&path, &r, &p.token);
    std::fs::set_permissions(&h.paths.bots_root, std::fs::Permissions::from_mode(0o755)).unwrap();
    let Outcome::Done { report } = result.unwrap() else { panic!("expected partial report") };
    assert_eq!(report.written.len(), 1);
    assert_eq!(report.written[0].kind, ItemKind::Skill);
    assert!(report.error.is_some());
    assert!(h.engine.list_bots().await.unwrap().is_empty());
    assert!(h.engine.list_workflows().unwrap().is_empty());
    apply(&h, &path, &r);
    rewrite(&path, |f| f.bots[0].instructions = "Replacement".into());
    let mut r = r;
    r.bots.insert("review".into(), Choice::Replace);
    let p = preview(&h, &path, &r);
    let previous = h.paths.previous_dir();
    std::fs::create_dir_all(&previous).unwrap();
    std::fs::set_permissions(&previous, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = h.engine.import_workflow_config(&path, &r, &p.token);
    std::fs::set_permissions(&previous, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(result.is_err());
    assert_eq!(h.engine.list_bots().await.unwrap()[0].bot.instructions, "Be helpful.");
}

#[tokio::test]
async fn an_oversized_required_skill_is_refused_without_replacing_the_export() {
    let h = harness();
    let s = skill(&h, "shared");
    let b = bot(&h, "Review", std::slice::from_ref(&s));
    let w = workflow(&h, "Selected", &[b]);
    let path = package(&h, &w);
    let before = std::fs::read(&path).unwrap();
    let big = h.paths.skills_dir().join(&s.slug).join("large.bin");
    std::fs::File::create(big).unwrap().set_len(sonata_core::skills::MAX_SKILL_BYTES + 1).unwrap();
    assert!(matches!(
        h.engine.write_workflow_config(&w.id, &path),
        Err(sonata_core::CoreError::Refused(sonata_core::Refusal::SkillTooLarge { .. }))
    ));
    assert_eq!(std::fs::read(path).unwrap(), before);
}
