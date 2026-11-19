mod support;

use std::collections::HashMap;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::json;
use sonata_core::bundled::{self, checkout_paragraph, NEW_ASK};
use support::*;
const CUSTOM_ASK: &str = "Ask the user before continuing.";
use sonata_core::config_file::{
    from_yaml, read_skill_folder, to_yaml, Blob, ConfigFile, FileBot, FileDef, FileEntry, FileSkill, FileStep,
    FileWorkflow,
};
use sonata_core::engine::{
    BackupPlace, ExportSummary, ImportMode, ImportOutcome, ImportPreview, ImportReport, ItemKind, ItemStatus,
    PlannedItem, WrittenItem,
};
use sonata_core::model::{Arrow, Bot, NewBot, NewSkill, NewWorkflow, Pos, Provider, Session, Skill, Workflow, DONE};
use sonata_core::{CoreError, Refusal};

fn set_up(h: &Harness) -> tempfile::TempDir {
    let checkout = tempfile::tempdir().unwrap();
    setup_team(h, Some(checkout.path()));
    checkout
}

fn exported(h: &Harness) -> (tempfile::TempDir, PathBuf, ExportSummary) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    let summary = h.engine.write_config(&path).unwrap();
    (dir, path, summary)
}

fn read(path: &Path) -> ConfigFile {
    from_yaml(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[tokio::test]
async fn export_writes_every_bot_skill_and_workflow() {
    let h = harness();
    let _checkout = set_up(&h);
    let unused_bot = h.engine.create_bot(claude_bot("Standalone")).unwrap();
    let unused_skill = h
        .engine
        .create_skill(NewSkill {
            name: "Unassigned".into(),
            description: "For later".into(),
            body: "Keep this.".into(),
        })
        .unwrap();
    let original = h.engine.list_workflows().unwrap().remove(0);
    h.engine
        .create_workflow(NewWorkflow { name: "Another workflow".into(), when_to_use: String::new(), def: original.def })
        .unwrap();
    let (_dir, path, summary) = exported(&h);
    assert_eq!(summary, ExportSummary { bots: 10, skills: 11, workflows: 2 });
    let file = read(&path);
    assert_eq!(file.format, "sonata-config");
    assert_eq!(file.version, 1);
    assert_eq!(file.app_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(file.bots[0].handle, "master");
    assert!(file.bots[0].master);
    assert_eq!(file.bots.iter().filter(|b| b.master).count(), 1);
    let plan = file.bots.iter().find(|b| b.handle == "plan").unwrap();
    assert_eq!(plan.skills, ["sonata", "sonata-plan"]);
    assert!(file.bots.iter().any(|bot| bot.handle == unused_bot.handle));
    let skill = file.skills.iter().find(|skill| skill.slug == unused_skill.slug).unwrap();
    assert!(skill
        .files
        .iter()
        .any(|entry| entry.path == "SKILL.md" && entry.text.as_ref().unwrap().contains("Keep this.")));
    assert_eq!(file.workflows.len(), 2);
    assert!(file.workflows.iter().any(|workflow| workflow.name == "Another workflow"));

    let restored = harness();
    let report = import(&restored, &path, ImportMode::AddNew);
    assert_eq!(report.error, None);
    let restored = restored.engine.export_config().unwrap();
    assert_eq!((restored.bots, restored.skills, restored.workflows), (file.bots, file.skills, file.workflows));
}

#[tokio::test]
async fn the_checkout_path_never_travels() {
    let h = harness();
    let checkout = set_up(&h);
    let own = h.engine.create_bot(claude_bot("Own")).unwrap();
    let (_dir, path, _) = exported(&h);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains(checkout.path().to_str().unwrap()));
    let file = from_yaml(&text).unwrap();
    let plan = file.bots.iter().find(|b| b.handle == "plan").unwrap();
    assert!(plan.checkout);
    assert!(plan.instructions.contains(&checkout_paragraph(None)));
    let own = file.bots.iter().find(|b| b.handle == own.handle).unwrap();
    assert!(!own.checkout);
}

#[tokio::test]
async fn skills_travel_whole() {
    let h = harness();
    let _checkout = set_up(&h);
    let folder = h.paths.skills_dir().join("sonata");
    std::fs::write(folder.join("logo.png"), [0xff, 0xfe, 0x00, 0x80]).unwrap();
    std::fs::create_dir_all(folder.join("scripts")).unwrap();
    let script = folder.join("scripts/x.sh");
    std::fs::write(&script, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    symlink("SKILL.md", folder.join("link.md")).unwrap();
    let (_dir, path, _) = exported(&h);
    let file = read(&path);
    let skill = file.skills.iter().find(|s| s.slug == "sonata").unwrap();
    let find = |p: &str| skill.files.iter().find(|f| f.path == p);
    assert!(find("logo.png").unwrap().base64.is_some());
    assert!(find("logo.png").unwrap().text.is_none());
    assert!(find("scripts/x.sh").unwrap().executable);
    assert!(find("link.md").is_none());
}

#[tokio::test]
async fn workflow_steps_name_bots_by_handle_and_keep_positions() {
    let h = harness();
    let _checkout = set_up(&h);
    let stored = h.engine.list_workflows().unwrap().remove(0);
    assert_eq!(stored.name, "Example review workflow");
    let handles: std::collections::HashMap<String, String> =
        h.engine.list_bots().await.unwrap().into_iter().map(|v| (v.bot.id, v.bot.handle)).collect();
    let (_dir, path, _) = exported(&h);
    let file = read(&path);
    let def = &file.workflows[0].definition;
    assert_eq!(def.start, stored.def.start);
    assert_eq!(def.start_pos, stored.def.start_pos);
    assert_eq!(def.done_pos, stored.def.done_pos);
    assert_eq!(def.steps.len(), stored.def.steps.len());
    for (out, step) in def.steps.iter().zip(&stored.def.steps) {
        assert_eq!(out.id, step.id);
        assert_eq!(out.pos, step.pos);
        assert_eq!(out.bot.as_deref(), Some(handles[&step.bot_id].as_str()));
    }
    let gone = stored.def.steps[0].bot_id.clone();
    h.engine.delete_bot(&gone).await.unwrap();
    let (_dir, path, _) = exported(&h);
    let file = read(&path);
    assert_eq!(file.workflows[0].definition.steps[0].bot, None);
}

#[tokio::test]
async fn an_export_that_fails_leaves_the_older_file() {
    let h = harness();
    let _checkout = set_up(&h);
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("old.yaml");
    std::fs::write(&old, "old").unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = h.engine.write_config(&old);
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(result.is_err());
    assert_eq!(std::fs::read_to_string(&old).unwrap(), "old");
    let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names, ["old.yaml"]);
}

#[tokio::test]
async fn an_export_over_256_mb_stops() {
    let h = harness();
    let _checkout = set_up(&h);
    let big = std::fs::File::create(h.paths.skills_dir().join("sonata/big.bin")).unwrap();
    big.set_len(257 * 1024 * 1024).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    match h.engine.write_config(&path) {
        Err(CoreError::Refused(Refusal::ConfigExportTooLarge { mb: 256 })) => {}
        other => panic!("{other:?}"),
    }
    assert!(!path.exists());
}

#[tokio::test]
async fn links_in_a_skill_folder_count_for_nothing() {
    let h = harness();
    let _checkout = set_up(&h);
    let folder = h.paths.skills_dir().join("sonata");
    symlink("gone.md", folder.join("dangling.md")).unwrap();
    symlink("..", folder.join("up")).unwrap();
    let big = tempfile::tempdir().unwrap();
    std::fs::File::create(big.path().join("big.bin")).unwrap().set_len(257 * 1024 * 1024).unwrap();
    symlink(big.path(), folder.join("big")).unwrap();
    let (_dir, path, _) = exported(&h);
    let file = read(&path);
    let skill = file.skills.iter().find(|s| s.slug == "sonata").unwrap();
    for link in ["dangling.md", "up", "big"] {
        let inside = format!("{link}/");
        assert!(skill.files.iter().all(|f| f.path != link && !f.path.starts_with(&inside)), "{link}");
    }
    rewrite(&path, |f| skill_mut(f, "sonata").name = "Sonata, renamed".into());
    let report = import(&h, &path, ImportMode::AddAndReplace);
    assert_eq!(report.error, None);
    assert_eq!(report.written, [written(ItemKind::Skill, "sonata", true)]);
    let backup = read(Path::new(&report.backup.unwrap().kept_at.unwrap()));
    assert_eq!(backup.skills[0].files, skill.files);
}

fn rewrite(path: &Path, change: impl FnOnce(&mut ConfigFile)) {
    let mut file = read(path);
    change(&mut file);
    std::fs::write(path, to_yaml(&file).unwrap()).unwrap();
}

fn previewed_after(h: &Harness, change: impl FnOnce(&mut ConfigFile)) -> sonata_core::Result<ImportPreview> {
    let (_dir, path, _) = exported(h);
    rewrite(&path, change);
    h.engine.preview_config_import(&path)
}

fn item<'a>(items: &'a [PlannedItem], name: &str) -> &'a PlannedItem {
    items.iter().find(|i| i.name == name).unwrap_or_else(|| panic!("no item {name} in {items:?}"))
}

fn status_of<'a>(items: &'a [PlannedItem], name: &str) -> &'a ItemStatus {
    &item(items, name).status
}

fn planned(name: &str, target: &str, status: ItemStatus) -> PlannedItem {
    PlannedItem { name: name.into(), target: target.into(), status }
}

fn invalid(code: &str, params: serde_json::Value) -> ItemStatus {
    ItemStatus::Invalid { code: code.into(), params }
}

fn refused(result: sonata_core::Result<ImportPreview>) -> Refusal {
    match result {
        Err(CoreError::Refused(r)) => r,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn text_entry(path: &str, text: &str) -> FileEntry {
    FileEntry { path: path.into(), executable: false, text: Some(text.into()), base64: None }
}

fn note_skill(slug: &str, extra: Vec<FileEntry>) -> FileSkill {
    let mut files = vec![text_entry("SKILL.md", &format!("---\nname: {slug}\ndescription: Notes\n---\nBody\n"))];
    files.extend(extra);
    FileSkill { slug: slug.into(), name: "Notes".into(), description: "Notes".into(), files }
}

fn skill_mut<'a>(f: &'a mut ConfigFile, slug: &str) -> &'a mut FileSkill {
    f.skills.iter_mut().find(|s| s.slug == slug).unwrap()
}

fn skill_named(h: &Harness, slug: &str) -> Skill {
    h.engine.list_skills().unwrap().into_iter().find(|v| v.skill.slug == slug).unwrap().skill
}

fn folder_names(dir: &Path) -> Vec<std::ffi::OsString> {
    let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name()).collect();
    names.sort();
    names
}

#[tokio::test]
async fn a_file_that_repeats_a_name_or_has_two_masters_is_refused() {
    let h = harness();
    let _checkout = set_up(&h);
    let copy_of_plan = |handle: &'static str| {
        move |f: &mut ConfigFile| {
            let mut bot = f.bots.iter().find(|b| b.handle == "plan").unwrap().clone();
            bot.handle = handle.into();
            f.bots.push(bot);
        }
    };
    assert_eq!(refused(previewed_after(&h, copy_of_plan("plan"))), Refusal::ConfigFileRepeats("@plan".into()));
    assert_eq!(refused(previewed_after(&h, copy_of_plan("Plan"))), Refusal::ConfigFileRepeats("@Plan".into()));
    let sonata_twice = |f: &mut ConfigFile| {
        let skill = f.skills.iter().find(|s| s.slug == "sonata").unwrap().clone();
        f.skills.push(skill);
    };
    assert_eq!(refused(previewed_after(&h, sonata_twice)), Refusal::ConfigFileRepeats("sonata".into()));
    for name in ["example review workflow", "  example review workflow "] {
        let workflow_twice = |f: &mut ConfigFile| {
            let mut workflow = f.workflows[0].clone();
            workflow.name = name.into();
            f.workflows.push(workflow);
        };
        assert_eq!(refused(previewed_after(&h, workflow_twice)), Refusal::ConfigFileRepeats(name.into()));
    }
    let two_masters = |f: &mut ConfigFile| f.bots.iter_mut().find(|b| b.handle == "plan").unwrap().master = true;
    assert_eq!(refused(previewed_after(&h, two_masters)), Refusal::ConfigFileTwoMasters);
}

#[tokio::test]
async fn skills_preview_as_new_unchanged_different_or_invalid() {
    let h = harness();
    let _checkout = set_up(&h);
    let library = h.engine.list_skills().unwrap();
    let folders = folder_names(&h.paths.skills_dir());
    let preview = previewed_after(&h, |f| {
        skill_mut(f, "sonata-plan").name = "Planning".into();
        let entry = skill_mut(f, "sonata").files.iter_mut().find(|e| e.path != "SKILL.md").unwrap();
        entry.executable = !entry.executable;
        skill_mut(f, "sonata-research").name = "  sonata-research  ".into();
        skill_mut(f, "sonata-query").name = String::new();
        f.skills.push(note_skill("notes", vec![]));
        f.skills.push(note_skill("bad", vec![text_entry("../x", "x")]));
    })
    .unwrap();
    let skills = &preview.skills;
    assert_eq!(item(skills, "sonata-plan"), &planned("sonata-plan", "sonata-plan", ItemStatus::Different));
    assert_eq!(item(skills, "sonata"), &planned("sonata", "sonata", ItemStatus::Different));
    assert_eq!(item(skills, "notes"), &planned("notes", "notes", ItemStatus::New));
    assert_eq!(item(skills, "bad"), &planned("bad", "", invalid("skillPathInvalid", json!({ "path": "../x" }))));
    assert_eq!(item(skills, "sonata-debug"), &planned("sonata-debug", "sonata-debug", ItemStatus::Unchanged));
    assert_eq!(skills.len(), library.len() + 2);
    for other in skills.iter().filter(|i| !["sonata", "sonata-plan", "notes", "bad"].contains(&i.name.as_str())) {
        assert_eq!(other.status, ItemStatus::Unchanged, "{}", other.name);
    }
    assert_eq!(h.engine.list_skills().unwrap(), library);
    assert_eq!(folder_names(&h.paths.skills_dir()), folders);
}

#[tokio::test]
async fn a_leftover_folder_makes_a_skill_cant_import() {
    let h = harness();
    let _checkout = set_up(&h);
    std::fs::create_dir(h.paths.skills_dir().join("orphan")).unwrap();
    let preview = previewed_after(&h, |f| f.skills.push(note_skill("orphan", vec![]))).unwrap();
    let taken = invalid("skillFolderTaken", json!({ "slug": "orphan" }));
    assert_eq!(item(&preview.skills, "orphan"), &planned("orphan", "", taken));
}

fn set_playbook(h: &Harness, playbook: &str) {
    let skill = skill_named(h, "sonata-master");
    let edit =
        NewSkill { name: skill.name.clone(), description: skill.description.clone(), body: playbook.to_string() };
    h.engine.update_skill(&skill.id, edit).unwrap();
}

#[tokio::test]
async fn a_matching_playbook_is_unchanged_and_user_edits_are_different() {
    let a = harness();
    let _a_checkout = set_up(&a);
    set_playbook(&a, &bundled::master_skill_body());
    let (_old_dir, old, _) = exported(&a);
    let current_playbook = bundled::master_skill_body();
    set_playbook(&a, &format!("{current_playbook}\nThe user's own line.\n"));
    let (_edited_dir, edited, _) = exported(&a);
    let b = harness();
    let _b_checkout = set_up(&b);
    let preview = b.engine.preview_config_import(&old).unwrap();
    assert_eq!(
        item(&preview.skills, "sonata-master"),
        &planned("sonata-master", "sonata-master", ItemStatus::Unchanged)
    );
    let preview = b.engine.preview_config_import(&edited).unwrap();
    assert_eq!(status_of(&preview.skills, "sonata-master"), &ItemStatus::Different);
}

#[tokio::test]
async fn the_token_changes_with_the_file_and_with_the_install() {
    let h = harness();
    let _checkout = set_up(&h);
    let (_dir, path, _) = exported(&h);
    let first = h.engine.preview_config_import(&path).unwrap();
    assert_eq!(h.engine.preview_config_import(&path).unwrap(), first);
    assert_eq!(first.token.len(), 64);

    let skill = skill_named(&h, "sonata-plan");
    let body = h.engine.skill_body(&skill.id).unwrap();
    h.engine
        .update_skill(&skill.id, NewSkill { name: "Planning".into(), description: skill.description, body })
        .unwrap();
    let edited = h.engine.preview_config_import(&path).unwrap();
    assert_eq!(status_of(&edited.skills, "sonata-plan"), &ItemStatus::Different);
    assert_ne!(edited.token, first.token);

    rewrite(&path, |f| {
        let first = f.exported_at.remove(0);
        f.exported_at.insert(0, if first == '1' { '2' } else { '1' });
    });
    let rewritten = h.engine.preview_config_import(&path).unwrap();
    assert_eq!(rewritten.skills, edited.skills);
    assert_ne!(rewritten.token, edited.token);
}

fn bot_mut<'a>(f: &'a mut ConfigFile, handle: &str) -> &'a mut FileBot {
    f.bots.iter_mut().find(|b| b.handle == handle).unwrap()
}

fn file_bot(handle: &str) -> FileBot {
    FileBot {
        handle: handle.into(),
        name: format!("Bot {handle}"),
        color: "#3b82f6".into(),
        provider: "claude".into(),
        model: None,
        checkout: false,
        master: false,
        skills: vec![],
        instructions: "Be helpful.".into(),
    }
}

fn one_step_workflow(name: &str, bot: Option<&str>) -> FileWorkflow {
    let step = FileStep {
        id: "s1".into(),
        name: "Write".into(),
        bot: bot.map(Into::into),
        pos: None,
        arrows: vec![Arrow { to: DONE.into(), outcome: None }],
        instructions: "Write it.".into(),
    };
    FileWorkflow {
        name: name.into(),
        when_to_use: String::new(),
        definition: FileDef {
            version: 1,
            start: Some("s1".into()),
            start_pos: None,
            done_pos: None,
            steps: vec![step],
        },
    }
}

#[tokio::test]
async fn bots_preview_with_their_handles() {
    let h = harness();
    let _checkout = set_up(&h);
    std::fs::create_dir(h.paths.bots_root.join("scout")).unwrap();
    let bots_before = h.engine.list_bots().await.unwrap();
    let folders = folder_names(&h.paths.bots_root);
    let preview = previewed_after(&h, |f| {
        bot_mut(f, "debug").color = "#000000".into();
        f.bots.push(file_bot("writer"));
        f.bots.push(file_bot("scout"));
        f.bots.push(file_bot("scout-2"));
    })
    .unwrap();
    let bots = &preview.bots;
    assert_eq!(item(bots, "plan"), &planned("plan", "plan", ItemStatus::Unchanged));
    assert_eq!(item(bots, "debug"), &planned("debug", "debug", ItemStatus::Different));
    assert_eq!(item(bots, "writer"), &planned("writer", "writer", ItemStatus::New));
    assert_eq!(item(bots, "scout"), &planned("scout", "scout-3", ItemStatus::New));
    assert_eq!(item(bots, "scout-2"), &planned("scout-2", "scout-2", ItemStatus::New));
    assert_eq!(bots.len(), bots_before.len() + 3);
    assert_eq!(h.engine.list_bots().await.unwrap(), bots_before);
    assert_eq!(folder_names(&h.paths.bots_root), folders);
}

#[tokio::test]
async fn invalid_bots_say_why() {
    let h = harness();
    let _checkout = set_up(&h);
    let preview = previewed_after(&h, |f| {
        f.skills.push(note_skill("bad", vec![text_entry("../x", "x")]));
        f.bots.push(FileBot { color: "blue".into(), ..file_bot("a") });
        f.bots.push(FileBot { provider: "gemini".into(), ..file_bot("b") });
        f.bots.push(FileBot { provider: "cursor".into(), ..file_bot("c") });
        f.bots.push(FileBot { provider: "opencode".into(), model: Some("kimi".into()), ..file_bot("d") });
        f.bots.push(FileBot { skills: vec!["sonata".into(), "nope".into()], ..file_bot("e") });
        f.bots.push(FileBot { skills: vec!["sonata".into(), "bad".into()], ..file_bot("f") });
        f.bots.push(FileBot { name: "  ".into(), ..file_bot("g") });
        bot_mut(f, "plan").color = "#12345".into();
    })
    .unwrap();
    let bots = &preview.bots;
    let cases = [
        ("a", invalid("botColorInvalid", json!({ "color": "blue" }))),
        ("b", invalid("providerUnknown", json!({ "provider": "gemini" }))),
        ("c", invalid("providerUnavailable", json!({ "provider": "cursor" }))),
        ("d", invalid("modelFormat", json!({ "model": "kimi" }))),
        ("e", invalid("skillUnknown", json!({ "slug": "nope" }))),
        ("f", invalid("importDependsOn", json!({ "name": "bad" }))),
        ("g", invalid("nameRequired", json!({}))),
        ("plan", invalid("botColorInvalid", json!({ "color": "#12345" }))),
    ];
    for (handle, status) in cases {
        assert_eq!(item(bots, handle), &planned(handle, "", status));
    }
}

#[tokio::test]
async fn the_files_master_matches_this_installs_master_whatever_the_handles() {
    let h = harness();
    let _checkout = set_up(&h);
    let preview = previewed_after(&h, |f| {
        let master = f.bots.iter_mut().find(|b| b.master).unwrap();
        master.handle = "master-2".into();
        f.bots.push(file_bot("master"));
    })
    .unwrap();
    let bots = &preview.bots;
    assert_eq!(item(bots, "master-2"), &planned("master-2", "master", ItemStatus::Unchanged));
    assert_eq!(item(bots, "master"), &planned("master", "master-2", ItemStatus::New));
}

#[tokio::test]
async fn a_master_the_user_deleted_comes_back_as_new() {
    let h = harness();
    let _checkout = set_up(&h);
    let (_dir, path, _) = exported(&h);
    let master = h.engine.master_bot().unwrap().unwrap();
    let folder = h.engine.delete_bot(&master.id).await.unwrap();
    std::fs::remove_dir_all(folder).unwrap();
    let preview = h.engine.preview_config_import(&path).unwrap();
    assert_eq!(item(&preview.bots, "master"), &planned("master", "master", ItemStatus::New));
    let own = h.engine.create_bot(claude_bot("Master")).unwrap();
    assert_eq!(own.handle, "master");
    let preview = h.engine.preview_config_import(&path).unwrap();
    assert_eq!(item(&preview.bots, "master"), &planned("master", "master-2", ItemStatus::New));
}

#[tokio::test]
async fn project_folder_is_normalized_but_custom_instructions_are_preserved() {
    let a = harness();
    let a_checkout = set_up(&a);
    let b = harness();
    let b_checkout = set_up(&b);
    assert_ne!(a_checkout.path(), b_checkout.path());
    let (_dir, path, _) = exported(&a);
    let preview = b.engine.preview_config_import(&path).unwrap();
    assert_eq!(item(&preview.bots, "plan"), &planned("plan", "plan", ItemStatus::Unchanged));
    rewrite(&path, |f| {
        let plan = bot_mut(f, "plan");
        assert!(plan.instructions.contains(NEW_ASK));
        plan.instructions = plan.instructions.replace(NEW_ASK, CUSTOM_ASK);
    });
    let preview = b.engine.preview_config_import(&path).unwrap();
    assert_eq!(item(&preview.bots, "plan"), &planned("plan", "plan", ItemStatus::Different));
}

#[tokio::test]
async fn workflows_preview_against_the_bots_the_import_leaves() {
    let h = harness();
    let _checkout = set_up(&h);
    let ticket = "Example review workflow";
    let preview = previewed_after(&h, |_| {}).unwrap();
    assert_eq!(item(&preview.workflows, ticket), &planned(ticket, ticket, ItemStatus::Unchanged));

    let moved = |f: &mut ConfigFile| f.workflows[0].definition.steps[0].pos = Some(Pos { x: 10.0, y: 20.0 });
    let preview = previewed_after(&h, moved).unwrap();
    assert_eq!(item(&preview.workflows, ticket), &planned(ticket, ticket, ItemStatus::Different));

    let renamed = "example review workflow";
    let preview = previewed_after(&h, |f| f.workflows[0].name = renamed.into()).unwrap();
    assert_eq!(item(&preview.workflows, renamed), &planned(renamed, ticket, ItemStatus::Different));

    let preview = previewed_after(&h, |f| f.bots.retain(|b| b.handle != "plan")).unwrap();
    assert_eq!(item(&preview.workflows, ticket), &planned(ticket, ticket, ItemStatus::Unchanged));

    let preview = previewed_after(&h, |f| bot_mut(f, "plan").color = "blue".into()).unwrap();
    let depends = invalid("importDependsOn", json!({ "name": "@plan" }));
    assert_eq!(item(&preview.workflows, ticket), &planned(ticket, "", depends));

    std::fs::create_dir(h.paths.bots_root.join("scout")).unwrap();
    let preview = previewed_after(&h, |f| {
        f.bots.push(file_bot("writer"));
        f.bots.push(file_bot("scout"));
        f.bots.push(FileBot { color: "blue".into(), ..file_bot("broken") });
        f.workflows.push(one_step_workflow("Write", Some("writer")));
        f.workflows.push(one_step_workflow("  Scout  ", Some("scout")));
        f.workflows.push(one_step_workflow("Broken", Some("broken")));
        f.workflows.push(one_step_workflow("Nobody", None));
        f.workflows.push(one_step_workflow("Ghost", Some("ghost")));
    })
    .unwrap();
    let workflows = &preview.workflows;
    assert_eq!(item(workflows, ticket), &planned(ticket, ticket, ItemStatus::Unchanged));
    assert_eq!(item(workflows, "Write"), &planned("Write", "Write", ItemStatus::New));
    assert_eq!(item(&preview.bots, "scout"), &planned("scout", "scout-2", ItemStatus::New));
    assert_eq!(item(workflows, "  Scout  "), &planned("  Scout  ", "Scout", ItemStatus::New));
    let depends = invalid("importDependsOn", json!({ "name": "@broken" }));
    assert_eq!(item(workflows, "Broken"), &planned("Broken", "", depends));
    let no_bot = invalid("workflowStepNoBot", json!({ "step": "Write" }));
    assert_eq!(item(workflows, "Nobody"), &planned("Nobody", "", no_bot.clone()));
    assert_eq!(item(workflows, "Ghost"), &planned("Ghost", "", no_bot));
    assert_eq!(item(&preview.bots, "writer").status, ItemStatus::New);
    let others = preview.bots.iter().filter(|i| !["writer", "scout", "broken"].contains(&i.name.as_str()));
    for other in others.chain(&preview.skills) {
        assert_eq!(other.status, ItemStatus::Unchanged, "{}", other.name);
    }
}

#[tokio::test]
async fn a_file_this_install_exported_is_all_unchanged() {
    let h = harness();
    let _checkout = set_up(&h);
    let mut ticket = h.engine.list_workflows().unwrap().remove(0);
    for (i, step) in ticket.def.steps.iter_mut().enumerate() {
        step.pos = Some(Pos { x: 0.1 + 0.2, y: (i as f64 + 1.0) / 3.0 });
    }
    let edit = NewWorkflow { name: ticket.name, when_to_use: ticket.when_to_use, def: ticket.def };
    h.engine.update_workflow(&ticket.id, edit).unwrap();
    let preview = previewed_after(&h, |_| {}).unwrap();
    assert_eq!((preview.bots.len(), preview.skills.len(), preview.workflows.len()), (9, 10, 1));
    for item in preview.skills.iter().chain(&preview.bots).chain(&preview.workflows) {
        assert_eq!(item, &planned(&item.name, &item.name, ItemStatus::Unchanged));
    }
}

fn trash_bin(h: &Harness) -> (tempfile::TempDir, Arc<Mutex<Vec<PathBuf>>>) {
    let bin = tempfile::tempdir().unwrap();
    let into = bin.path().to_path_buf();
    let moved = Arc::new(Mutex::new(Vec::new()));
    let list = moved.clone();
    h.engine.set_trash(Arc::new(move |path: &Path| {
        let to = into.join(path.file_name().unwrap());
        std::fs::rename(path, &to).map_err(|e| e.to_string())?;
        list.lock().unwrap().push(to);
        Ok(())
    }));
    (bin, moved)
}

fn import(h: &Harness, path: &Path, mode: ImportMode) -> ImportReport {
    let preview = h.engine.preview_config_import(path).unwrap();
    match h.engine.import_config(path, mode, &preview.token).unwrap() {
        ImportOutcome::Done { report } => report,
        other => panic!("expected the import done, got {other:?}"),
    }
}

fn written(kind: ItemKind, name: &str, replaced: bool) -> WrittenItem {
    WrittenItem { kind, name: name.into(), replaced }
}

async fn bot(h: &Harness, handle: &str) -> Bot {
    let bots = h.engine.list_bots().await.unwrap();
    bots.into_iter().map(|v| v.bot).find(|b| b.handle == handle).unwrap_or_else(|| panic!("no bot @{handle}"))
}

async fn handles(h: &Harness) -> Vec<String> {
    h.engine.list_bots().await.unwrap().into_iter().map(|v| v.bot.handle).collect()
}

async fn edit_bot(h: &Harness, handle: &str, change: impl FnOnce(&mut NewBot)) {
    let bot = bot(h, handle).await;
    let mut edit = NewBot {
        display_name: bot.display_name,
        color: bot.color,
        provider: bot.provider,
        model: bot.model,
        instructions: bot.instructions,
        skill_ids: bot.skill_ids,
    };
    change(&mut edit);
    h.engine.update_bot(&bot.id, edit).unwrap();
}

fn configuration(h: &Harness) -> (Vec<FileBot>, Vec<FileSkill>, Vec<FileWorkflow>) {
    let file = h.engine.export_config().unwrap();
    (file.bots, file.skills, file.workflows)
}

struct EditedB {
    plan: Bot,
    session: Session,
    plan_files: Vec<Blob>,
    workflow: Workflow,
    versions: HashMap<String, i64>,
}

async fn edit_b(b: &Harness) -> EditedB {
    edit_bot(b, "plan", |e| {
        e.display_name = "Planner".into();
        e.color = "#123456".into();
        e.provider = Provider::Codex;
    })
    .await;
    let skill = skill_named(b, "sonata-plan");
    let body = b.engine.skill_body(&skill.id).unwrap();
    b.engine.update_skill(&skill.id, NewSkill { name: "Plans".into(), description: skill.description, body }).unwrap();
    let folder = b.paths.skills_dir().join("sonata-plan");
    std::fs::write(folder.join("b-only.md"), "B's own notes.\n").unwrap();
    let mut workflow = b.engine.list_workflows().unwrap().remove(0);
    workflow.def.steps[0].pos = Some(Pos { x: 512.0, y: 640.0 });
    let edit = NewWorkflow { name: workflow.name, when_to_use: workflow.when_to_use, def: workflow.def };
    let workflow = b.engine.update_workflow(&workflow.id, edit).unwrap();
    let plan = bot(b, "plan").await;
    let session = new_session(b, &plan.id);
    let versions = b.engine.list_bots().await.unwrap().into_iter().map(|v| (v.bot.id, v.bot.config_version)).collect();
    EditedB { plan, session, plan_files: read_skill_folder(&folder).unwrap(), workflow, versions }
}

fn folders_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let entries = entries.map(|e| e.unwrap()).filter(|e| e.file_type().unwrap().is_dir());
    entries.map(|e| e.file_name().into_string().unwrap()).collect()
}

#[tokio::test]
async fn a_round_trip_into_a_fresh_install() {
    let a = harness();
    let _a_checkout = set_up(&a);
    let (_dir, path, _) = exported(&a);
    let b = harness();
    let report = import(&b, &path, ImportMode::AddNew);
    assert_eq!(report.error, None);
    assert_eq!(report.backup, None);
    assert!(report.written.iter().all(|item| !item.replaced));
    assert_eq!(handles(&b).await, handles(&a).await);
    assert_eq!(handles(&b).await[0], "master");
    let slugs = |h: &Harness| h.engine.list_skills().unwrap().into_iter().map(|v| v.skill.slug).collect::<Vec<_>>();
    assert_eq!(slugs(&b), slugs(&a));
    for slug in slugs(&a) {
        let files = |h: &Harness| read_skill_folder(&h.paths.skills_dir().join(&slug)).unwrap();
        assert_eq!(files(&b), files(&a), "{slug}");
        assert_eq!(skill_named(&b, &slug).name, skill_named(&a, &slug).name);
    }
    let names = |h: &Harness| h.engine.list_workflows().unwrap().into_iter().map(|w| w.name).collect::<Vec<_>>();
    assert_eq!(names(&b), names(&a));
    assert!(bot(&b, "plan").await.instructions.contains(&checkout_paragraph(None)));
    assert_eq!(b.engine.master_bot().unwrap().unwrap().handle, "master");
    let (_dir, again, _) = exported(&b);
    let (a_file, b_file) = (read(&path), read(&again));
    assert_eq!((b_file.bots, b_file.skills, b_file.workflows), (a_file.bots, a_file.skills, a_file.workflows));
}

#[tokio::test]
async fn add_new_writes_only_new_items() {
    let a = harness();
    let _a_checkout = set_up(&a);
    let b = harness();
    let _b_checkout = set_up(&b);
    let (_bin, moved) = trash_bin(&b);
    edit_bot(&b, "plan", |e| e.color = "#123456".into()).await;
    edit_bot(&b, "review", |e| {
        assert!(e.instructions.contains(NEW_ASK));
        e.instructions = e.instructions.replace(NEW_ASK, CUSTOM_ASK);
    })
    .await;
    let (_dir, path, _) = exported(&a);
    rewrite(&path, |f| f.bots.push(file_bot("writer")));
    let report = import(&b, &path, ImportMode::AddNew);
    assert_eq!(
        report,
        ImportReport { written: vec![written(ItemKind::Bot, "writer", false)], backup: None, error: None }
    );
    assert_eq!(bot(&b, "writer").await.display_name, "Bot writer");
    assert_eq!(bot(&b, "plan").await.color, "#123456");
    assert!(bot(&b, "review").await.instructions.contains(CUSTOM_ASK));
    assert!(moved.lock().unwrap().is_empty());
    assert!(!b.paths.previous_dir().exists() || folder_names(&b.paths.previous_dir()).is_empty());
}

#[tokio::test]
async fn add_and_replace_overwrites_and_keeps_one_backup() {
    let a = harness();
    let _a_checkout = set_up(&a);
    let b = harness_after(|_| {});
    let _b_checkout = set_up(&b);
    let before = edit_b(&b).await;
    let (bin, moved) = trash_bin(&b);
    let (_dir, path, _) = exported(&a);
    let report = import(&b, &path, ImportMode::AddAndReplace);
    assert_eq!(report.error, None);
    let ticket = "Example review workflow";
    let expected = vec![
        written(ItemKind::Skill, "sonata-plan", true),
        written(ItemKind::Bot, "plan", true),
        written(ItemKind::Workflow, ticket, true),
    ];
    assert_eq!(report.written, expected);

    let (a_plan, plan) = (bot(&a, "plan").await, bot(&b, "plan").await);
    assert_eq!(plan.id, before.plan.id);
    assert_eq!(
        (&plan.display_name, &plan.color, plan.provider),
        (&a_plan.display_name, &a_plan.color, a_plan.provider)
    );
    assert!(plan.config_version > before.plan.config_version);
    let sessions: Vec<String> = b.engine.sessions_of(&plan.id).unwrap().into_iter().map(|s| s.id).collect();
    assert_eq!(sessions, [before.session.id]);
    let skill = skill_named(&b, "sonata-plan");
    assert_eq!(skill.name, skill_named(&a, "sonata-plan").name);
    let folder = b.paths.skills_dir().join("sonata-plan");
    assert_eq!(
        read_skill_folder(&folder).unwrap(),
        read_skill_folder(&a.paths.skills_dir().join("sonata-plan")).unwrap()
    );
    let with_skill: Vec<Bot> = b
        .engine
        .list_bots()
        .await
        .unwrap()
        .into_iter()
        .map(|v| v.bot)
        .filter(|b| b.skill_ids.contains(&skill.id))
        .collect();
    assert!(!with_skill.is_empty());
    for bot in with_skill {
        assert!(bot.config_version > before.versions[&bot.id], "@{}", bot.handle);
    }
    let workflow = b.engine.list_workflows().unwrap().remove(0);
    assert_eq!(workflow.id, before.workflow.id);
    let a_workflow = a.engine.list_workflows().unwrap().remove(0);
    let positions = |w: &Workflow| w.def.steps.iter().map(|s| s.pos.clone()).collect::<Vec<_>>();
    assert_eq!(positions(&workflow), positions(&a_workflow));

    let moved = moved.lock().unwrap().clone();
    assert_eq!(moved.len(), 1);
    let name = moved[0].file_name().unwrap().to_str().unwrap().to_string();
    assert!(name.starts_with("sonata-config-before-import-") && name.ends_with(".yaml"), "{name}");
    assert_eq!(folder_names(bin.path()), [name.as_str()]);
    assert_eq!(report.backup, Some(BackupPlace { file_name: name, kept_at: None }));
    assert_eq!(folders_in(&b.paths.previous_dir()), Vec::<String>::new());
}

#[tokio::test]
async fn the_backup_puts_everything_back() {
    let a = harness();
    let _a_checkout = set_up(&a);
    let b = harness_after(|_| {});
    let _b_checkout = set_up(&b);
    let before = edit_b(&b).await;
    let edited = configuration(&b);
    let (_dir, path, _) = exported(&a);
    let report = import(&b, &path, ImportMode::AddAndReplace);
    assert_eq!(report.error, None);
    let kept = PathBuf::from(report.backup.unwrap().kept_at.unwrap());
    let backup = read(&kept);
    assert_eq!(backup.bots.iter().map(|b| b.handle.as_str()).collect::<Vec<_>>(), ["plan"]);
    assert_eq!(backup.skills.iter().map(|s| s.slug.as_str()).collect::<Vec<_>>(), ["sonata-plan"]);
    assert_eq!(backup.workflows.len(), 1);

    let restored = import(&b, &kept, ImportMode::AddAndReplace);
    assert_eq!(restored.error, None);
    let plan = bot(&b, "plan").await;
    assert_eq!(
        (plan.display_name.as_str(), plan.color.as_str(), plan.provider),
        ("Planner", "#123456", Provider::Codex)
    );
    assert_eq!(plan.instructions, before.plan.instructions);
    assert_eq!(plan.skill_ids, before.plan.skill_ids);
    assert_eq!(skill_named(&b, "sonata-plan").name, "Plans");
    assert_eq!(read_skill_folder(&b.paths.skills_dir().join("sonata-plan")).unwrap(), before.plan_files);
    let workflow = b.engine.list_workflows().unwrap().remove(0);
    assert_eq!((workflow.id, workflow.def), (before.workflow.id, before.workflow.def));
    assert_eq!(configuration(&b), edited);
}

#[tokio::test]
async fn the_backup_puts_a_replaced_master_back() {
    let a = harness();
    let _a_checkout = set_up(&a);
    let b = harness();
    let _b_checkout = set_up(&b);
    let master = b.engine.master_bot().unwrap().unwrap();
    edit_bot(&b, &master.handle, |e| e.color = "#123456".into()).await;
    let bots = handles(&b).await;
    let (_dir, path, _) = exported(&a);
    let report = import(&b, &path, ImportMode::AddAndReplace);
    assert_eq!(report.written, [written(ItemKind::Bot, "master", true)]);
    assert_eq!(b.engine.master_bot().unwrap().unwrap().color, bot(&a, "master").await.color);

    let kept = PathBuf::from(report.backup.unwrap().kept_at.unwrap());
    let restored = import(&b, &kept, ImportMode::AddAndReplace);
    assert_eq!(restored.error, None);
    assert_eq!(restored.written, [written(ItemKind::Bot, "master", true)]);
    let back = b.engine.master_bot().unwrap().expect("Master is still there");
    assert_eq!((back.id.as_str(), back.color.as_str()), (master.id.as_str(), "#123456"));
    assert_eq!(handles(&b).await, bots);
}

#[tokio::test]
async fn a_permission_or_name_only_change_is_written() {
    let h = harness();
    let _checkout = set_up(&h);
    let (_dir, path, _) = exported(&h);
    let mut changed = String::new();
    rewrite(&path, |f| {
        let entry = skill_mut(f, "sonata").files.iter_mut().find(|e| e.path != "SKILL.md" && !e.executable).unwrap();
        entry.executable = true;
        changed = entry.path.clone();
        skill_mut(f, "sonata-debug").name = "Debugging".into();
    });
    let report = import(&h, &path, ImportMode::AddAndReplace);
    assert_eq!(report.error, None);
    assert_eq!(
        report.written,
        [written(ItemKind::Skill, "sonata", true), written(ItemKind::Skill, "sonata-debug", true)]
    );
    let mode = std::fs::metadata(h.paths.skills_dir().join("sonata").join(&changed)).unwrap().permissions().mode();
    assert_ne!(mode & 0o111, 0, "{changed}");
    assert_eq!(skill_named(&h, "sonata-debug").name, "Debugging");
}

#[tokio::test]
async fn an_apply_after_a_change_writes_nothing() {
    let h = harness();
    let _checkout = set_up(&h);
    let (_dir, path, _) = exported(&h);
    rewrite(&path, |f| f.bots.push(file_bot("writer")));
    let preview = h.engine.preview_config_import(&path).unwrap();
    assert_eq!(status_of(&preview.bots, "writer"), &ItemStatus::New);
    h.engine.create_bot(claude_bot("writer")).unwrap();
    let bots = handles(&h).await;
    match h.engine.import_config(&path, ImportMode::AddAndReplace, &preview.token).unwrap() {
        ImportOutcome::Changed { preview: new } => {
            assert_ne!(new.token, preview.token);
            assert_eq!(item(&new.bots, "writer"), &planned("writer", "writer", ItemStatus::Different));
        }
        other => panic!("expected the preview changed, got {other:?}"),
    }
    assert_eq!(handles(&h).await, bots);
    assert_eq!(bot(&h, "writer").await.display_name, "writer");
    assert!(!h.paths.previous_dir().exists() || folder_names(&h.paths.previous_dir()).is_empty());

    let preview = h.engine.preview_config_import(&path).unwrap();
    rewrite(&path, |f| bot_mut(f, "plan").color = "#000000".into());
    let outcome = h.engine.import_config(&path, ImportMode::AddAndReplace, &preview.token).unwrap();
    assert!(matches!(outcome, ImportOutcome::Changed { .. }), "{outcome:?}");
    assert_eq!(bot(&h, "writer").await.display_name, "writer");
    assert_ne!(bot(&h, "plan").await.color, "#000000");
}

#[tokio::test]
async fn of_two_imports_at_once_the_second_plans_again() {
    let h = harness();
    let _checkout = set_up(&h);
    let (_dir, path, _) = exported(&h);
    rewrite(&path, |f| f.bots.push(file_bot("writer")));
    let preview = h.engine.preview_config_import(&path).unwrap();
    let start = Arc::new(std::sync::Barrier::new(2));
    let runs: Vec<_> = (0..2)
        .map(|_| {
            let (engine, path, token, start) = (h.engine.clone(), path.clone(), preview.token.clone(), start.clone());
            tokio::task::spawn_blocking(move || {
                start.wait();
                engine.import_config(&path, ImportMode::AddNew, &token).unwrap()
            })
        })
        .collect();
    let mut outcomes = Vec::new();
    for run in runs {
        outcomes.push(run.await.unwrap());
    }
    let done = outcomes.iter().filter(|o| matches!(o, ImportOutcome::Done { .. })).count();
    let changed = outcomes.iter().filter(|o| matches!(o, ImportOutcome::Changed { .. })).count();
    assert_eq!((done, changed), (1, 1), "{outcomes:?}");
    let writers: Vec<String> = handles(&h).await.into_iter().filter(|handle| handle.starts_with("writer")).collect();
    assert_eq!(writers, ["writer"]);
}

#[tokio::test]
async fn the_same_file_twice_changes_nothing_the_second_time() {
    let a = harness();
    let _a_checkout = set_up(&a);
    edit_bot(&a, "plan", |e| e.color = "#123456".into()).await;
    let body = "Write notes.".to_string();
    let notes = a.engine.create_skill(NewSkill { name: "Notes".into(), description: "Notes.".into(), body }).unwrap();
    a.engine.create_bot(NewBot { skill_ids: vec![notes.id], ..claude_bot("Writer") }).unwrap();
    let b = harness();
    let _b_checkout = set_up(&b);
    let (_dir, path, _) = exported(&a);
    let first = import(&b, &path, ImportMode::AddAndReplace);
    assert_eq!(first.error, None);
    let expected = vec![
        written(ItemKind::Skill, "notes", false),
        written(ItemKind::Bot, "plan", true),
        written(ItemKind::Bot, "writer", false),
    ];
    assert_eq!(first.written, expected);
    let preview = b.engine.preview_config_import(&path).unwrap();
    for item in preview.skills.iter().chain(&preview.bots).chain(&preview.workflows) {
        assert_eq!(item.status, ItemStatus::Unchanged, "{}", item.name);
    }
    let second = import(&b, &path, ImportMode::AddAndReplace);
    assert_eq!(second, ImportReport { written: vec![], backup: None, error: None });
}

#[tokio::test]
async fn a_failure_while_writing_stops_and_reports() {
    let h = harness();
    let _checkout = set_up(&h);
    let (_dir, path, _) = exported(&h);
    rewrite(&path, |f| {
        f.skills.push(note_skill("notes", vec![]));
        f.bots.push(FileBot { skills: vec!["notes".into()], ..file_bot("writer") });
        f.workflows.push(one_step_workflow("Write", Some("writer")));
    });
    let preview = h.engine.preview_config_import(&path).unwrap();
    let bots_root = &h.paths.bots_root;
    std::fs::set_permissions(bots_root, std::fs::Permissions::from_mode(0o555)).unwrap();
    let outcome = h.engine.import_config(&path, ImportMode::AddNew, &preview.token);
    std::fs::set_permissions(bots_root, std::fs::Permissions::from_mode(0o755)).unwrap();
    let report = match outcome.unwrap() {
        ImportOutcome::Done { report } => report,
        other => panic!("expected the import done, got {other:?}"),
    };
    assert_eq!(report.written, [written(ItemKind::Skill, "notes", false)]);
    let error = report.error.expect("the error that stopped the import");
    assert_eq!(error["code"], "internal");
    assert!(error["params"]["error"].is_string());
    assert_eq!(report.backup, None);
    assert_eq!(skill_named(&h, "notes").name, "Notes");
    assert!(!handles(&h).await.contains(&"writer".to_string()));
    assert!(h.engine.list_workflows().unwrap().iter().all(|w| w.name != "Write"));
}

#[tokio::test]
async fn a_master_the_user_deleted_comes_back_as_master() {
    let h = harness();
    let _checkout = set_up(&h);
    let (_dir, path, _) = exported(&h);
    let master = h.engine.master_bot().unwrap().unwrap();
    let folder = h.engine.delete_bot(&master.id).await.unwrap();
    std::fs::remove_dir_all(folder).unwrap();
    assert_eq!(h.engine.master_bot().unwrap(), None);
    let report = import(&h, &path, ImportMode::AddNew);
    assert_eq!(
        report,
        ImportReport { written: vec![written(ItemKind::Bot, "master", false)], backup: None, error: None }
    );
    let back = h.engine.master_bot().unwrap().expect("Master is back");
    assert_eq!(back.handle, "master");
    assert_ne!(back.id, master.id);
    assert_eq!(handles(&h).await[0], "master");
}

#[tokio::test]
async fn without_a_trash_the_backup_stays_in_previous() {
    let h = harness();
    let _checkout = set_up(&h);
    let color = bot(&h, "plan").await.color;
    let (_dir, path, _) = exported(&h);
    rewrite(&path, |f| bot_mut(f, "plan").color = "#000000".into());
    let report = import(&h, &path, ImportMode::AddAndReplace);
    assert_eq!(report.written, [written(ItemKind::Bot, "plan", true)]);
    let backup = report.backup.expect("a backup");
    let kept = PathBuf::from(backup.kept_at.expect("kept in previous/"));
    assert!(kept.is_file());
    assert_eq!(kept.parent().unwrap(), h.paths.previous_dir());
    assert_eq!(kept.file_name().unwrap().to_str().unwrap(), backup.file_name);
    let file = read(&kept);
    assert_eq!(file.bots.len(), 1);
    assert_eq!((file.bots[0].handle.as_str(), file.bots[0].color.as_str()), ("plan", color.as_str()));
    assert!(file.skills.is_empty() && file.workflows.is_empty());

    h.engine.set_trash(Arc::new(|_: &Path| Err("the Trash is full".to_string())));
    rewrite(&path, |f| bot_mut(f, "plan").color = "#111111".into());
    let report = import(&h, &path, ImportMode::AddAndReplace);
    let kept = PathBuf::from(report.backup.unwrap().kept_at.unwrap());
    assert!(kept.is_file());
    assert_eq!(read(&kept).bots[0].color, "#000000");
}

#[tokio::test]
async fn a_backup_that_cannot_be_written_writes_nothing() {
    let h = harness();
    let _checkout = set_up(&h);
    let color = bot(&h, "plan").await.color;
    let (_dir, path, _) = exported(&h);
    rewrite(&path, |f| {
        bot_mut(f, "plan").color = "#000000".into();
        f.bots.push(file_bot("writer"));
    });
    let previous = h.paths.previous_dir();
    std::fs::create_dir_all(&previous).unwrap();
    let preview = h.engine.preview_config_import(&path).unwrap();
    std::fs::set_permissions(&previous, std::fs::Permissions::from_mode(0o555)).unwrap();
    let outcome = h.engine.import_config(&path, ImportMode::AddAndReplace, &preview.token);
    std::fs::set_permissions(&previous, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(outcome.is_err(), "{outcome:?}");
    assert_eq!(bot(&h, "plan").await.color, color);
    assert!(!handles(&h).await.contains(&"writer".to_string()));
    assert!(folder_names(&previous).is_empty());
}
