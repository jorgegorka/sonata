mod support;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bridge_proto::{BridgeCommand, BridgeResponse, ErrorCode};
use serde_json::json;
use sonata_core::bundled::checkout_paragraph;
use sonata_core::engine::UiEvent;
use sonata_core::model::*;
use support::*;

async fn bot_named(h: &Harness, handle: &str) -> Bot {
    h.engine.list_bots().await.unwrap().into_iter().find(|v| v.bot.handle == handle).unwrap().bot
}

fn skill_named(h: &Harness, slug: &str) -> Skill {
    h.engine.list_skills().unwrap().into_iter().find(|v| v.skill.slug == slug).expect("the skill exists").skill
}

async fn user_session(h: &Harness, handle: &str, body: &str) -> (Session, String) {
    let bot = bot_named(h, handle).await;
    let (session, _) = h.engine.start_session_with_message(&bot.id, body, &[]).unwrap();
    h.engine.pump_session(&session.id).await;
    let pane = pane(h, &session);
    (session, pane)
}

async fn masters_turn(h: &Harness, checkout: Option<&Path>) -> (Session, String) {
    setup_team(h, checkout);
    user_session(h, "master", "Apply the review.").await
}

async fn run(h: &Harness, pane: &str, command: BridgeCommand) -> BridgeResponse {
    h.engine.handle_bridge(request(pane, command)).await
}

fn output(response: BridgeResponse) -> String {
    response.output.unwrap_or_else(|| panic!("no output: {:?}", response.error))
}

fn update(slug: &str, folder: &Path) -> BridgeCommand {
    BridgeCommand::SkillUpdate { slug: slug.into(), folder: folder.to_string_lossy().into_owned() }
}

fn edit(handle: &str, instructions: Option<&str>, add: &[&str], remove: &[&str]) -> BridgeCommand {
    BridgeCommand::BotEdit {
        handle: handle.into(),
        instructions: instructions.map(str::to_string),
        add_skills: add.iter().map(|s| s.to_string()).collect(),
        remove_skills: remove.iter().map(|s| s.to_string()).collect(),
        model: None,
    }
}

fn edit_model(handle: &str, model: Option<&str>) -> BridgeCommand {
    BridgeCommand::BotEdit {
        handle: handle.into(),
        instructions: None,
        add_skills: vec![],
        remove_skills: vec![],
        model: model.map(String::from),
    }
}

async fn copy(h: &Harness, pane: &str, slug: &str) -> PathBuf {
    let response = run(h, pane, BridgeCommand::SkillCopy { slug: slug.into() }).await;
    PathBuf::from(response.draft.unwrap_or_else(|| panic!("no draft: {:?}", response.error)).path)
}

fn trash(h: &Harness) -> Arc<Mutex<Vec<PathBuf>>> {
    let bin = h.paths.data_dir.parent().unwrap().join("trash");
    std::fs::create_dir_all(&bin).unwrap();
    let moved = Arc::new(Mutex::new(Vec::new()));
    let list = moved.clone();
    h.engine.set_trash(Arc::new(move |path: &Path| {
        let to = bin.join(path.file_name().unwrap());
        std::fs::rename(path, &to).map_err(|e| e.to_string())?;
        list.lock().unwrap().push(to);
        Ok(())
    }));
    moved
}

fn trashed(moved: &Arc<Mutex<Vec<PathBuf>>>) -> Vec<PathBuf> {
    moved.lock().unwrap().clone()
}

fn last_notice(h: &Harness, session: &Session, code: &str) -> serde_json::Value {
    notices(h, session, code).pop().unwrap_or_else(|| panic!("no {code} notice"))
}

fn notices(h: &Harness, session: &Session, code: &str) -> Vec<serde_json::Value> {
    h.engine
        .session_messages(&session.id)
        .unwrap()
        .into_iter()
        .filter(|m| m.system_code.as_deref() == Some(code))
        .filter_map(|m| m.system_params)
        .collect()
}

async fn guide(h: &Harness) -> Skill {
    let source = h.paths.data_dir.parent().unwrap().join("import/guide");
    std::fs::create_dir_all(source.join("reference")).unwrap();
    std::fs::write(source.join("SKILL.md"), "---\nname: guide\ndescription: Guides the work.\n---\n\n# Guide\n")
        .unwrap();
    std::fs::write(source.join("reference/a.md"), "old a\n").unwrap();
    let skill = h.engine.import_skill(&source).unwrap();
    for handle in ["plan", "feature"] {
        let bot = bot_named(h, handle).await;
        let mut skill_ids = bot.skill_ids.clone();
        skill_ids.push(skill.id.clone());
        h.engine.update_bot(&bot.id, NewBot { skill_ids, ..settings_of(&bot) }).unwrap();
    }
    skill
}

fn settings_of(bot: &Bot) -> NewBot {
    NewBot {
        display_name: bot.display_name.clone(),
        color: bot.color.clone(),
        provider: bot.provider,
        model: bot.model.clone(),
        instructions: bot.instructions.clone(),
        skill_ids: bot.skill_ids.clone(),
    }
}

fn files(dir: &Path) -> Vec<(String, String)> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
            if entry.file_type().unwrap().is_dir() {
                walk(&entry.path(), &format!("{name}/"), out);
            } else {
                out.push((name, std::fs::read_to_string(entry.path()).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, "", &mut out);
    out.sort();
    out
}

fn previous(h: &Harness) -> Vec<String> {
    match std::fs::read_dir(h.paths.previous_dir()) {
        Ok(entries) => entries.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect(),
        Err(_) => Vec::new(),
    }
}

async fn version_of(h: &Harness, handle: &str) -> i64 {
    bot_named(h, handle).await.config_version
}

#[tokio::test]
async fn only_master_on_the_users_turn_edits() {
    let h = harness();
    let (master_session, mp) = masters_turn(&h, None).await;
    let query_before = bot_named(&h, "query").await;
    let draft = copy(&h, &mp, "sonata-plan").await;
    let (_, plan_pane) = user_session(&h, "plan", "Hi.").await;
    for (command, name) in
        [(update("sonata-plan", &draft), "skill update"), (edit("@query", Some("Queries."), &[], &[]), "bot edit")]
    {
        let e = run(&h, &plan_pane, command).await.error.unwrap();
        assert_eq!((e.code, e.message), (ErrorCode::NotAllowed, format!("Only @master can use `sbot {name}`.")));
    }

    assert!(h.engine.handle_bridge(send(&mp, "@plan", "Plan it.")).await.ok);
    settle().await;
    let plan = bot_named(&h, "plan").await;
    let pp = pane(&h, &opened(&h, &plan, &master_session));
    assert!(h.engine.handle_bridge(reply(&pp, "Plan.")).await.ok);
    finish_turn(&h, &pp).await;
    finish_turn(&h, &mp).await;
    let denied = |r: BridgeResponse| {
        let e = r.error.expect("refused");
        assert_eq!(e.code, ErrorCode::NotAllowed);
        assert_eq!(
            e.message,
            "Change skills and bots only while answering the user's message, after they approved the change."
        );
    };
    denied(run(&h, &mp, update("sonata-plan", &draft)).await);
    denied(run(&h, &mp, edit("@query", Some("Queries."), &[], &[])).await);

    let master = bot_named(&h, "master").await;
    let task = NewTask {
        name: "Review".into(),
        bot_id: master.id.clone(),
        prompt: "Review the week.".into(),
        every_days: 7,
        at: "09:00".into(),
        starts_on: None,
    };
    let task = h.engine.create_task(task).unwrap();
    let run_session = h.engine.run_task_now(&task.id).unwrap();
    settle().await;
    let rp = pane(&h, &run_session);
    let run_draft = copy(&h, &rp, "sonata-plan").await;
    denied(run(&h, &rp, update("sonata-plan", &run_draft)).await);
    denied(run(&h, &rp, edit("@query", Some("Queries."), &[], &[])).await);

    assert_eq!(bot_named(&h, "query").await, query_before);
    assert!(previous(&h).is_empty());
}

#[tokio::test]
async fn skill_update_replaces_the_whole_folder() {
    let h = harness();
    let (session, mp) = masters_turn(&h, None).await;
    let skill = guide(&h).await;
    let moved = trash(&h);
    let (plan_before, feature_before, query_before) =
        (version_of(&h, "plan").await, version_of(&h, "feature").await, version_of(&h, "query").await);
    let draft = copy(&h, &mp, "guide").await;
    std::fs::write(draft.join("reference/a.md"), "new a\n").unwrap();
    std::fs::write(draft.join("reference/b.md"), "b\n").unwrap();
    let text = std::fs::read_to_string(draft.join("SKILL.md")).unwrap();
    std::fs::write(draft.join("SKILL.md"), text.replace("name: guide", "name: something-else")).unwrap();
    let mut ui = h.engine.subscribe_ui();

    let text = output(run(&h, &mp, update("guide", &draft)).await);
    assert_eq!(
        text,
        "Updated the skill guide. Bots that use it: @plan, @feature (each starts with the new version at its next \
         message). The previous version is in the Trash."
    );
    let library = h.engine.skills_dir().join("guide");
    let now = files(&library);
    assert_eq!(
        now.iter().map(|(path, _)| path.as_str()).collect::<Vec<_>>(),
        ["SKILL.md", "reference/a.md", "reference/b.md"]
    );
    assert_eq!(now[1].1, "new a\n");
    assert!(now[0].1.starts_with("---\nname: guide\n"), "{}", now[0].1);
    let after = skill_named(&h, "guide");
    assert_eq!((after.id.as_str(), after.slug.as_str()), (skill.id.as_str(), "guide"));
    assert_eq!(after.name, "something-else");
    assert_eq!(h.engine.list_skills().unwrap().iter().filter(|v| v.skill.slug.starts_with("guide")).count(), 1);
    assert_eq!(version_of(&h, "plan").await, plan_before + 1);
    assert_eq!(version_of(&h, "feature").await, feature_before + 1);
    assert_eq!(version_of(&h, "query").await, query_before);

    let moved = trashed(&moved);
    assert_eq!(moved.len(), 1);
    assert!(moved[0].file_name().unwrap().to_string_lossy().ends_with("-skill-guide"), "{}", moved[0].display());
    assert_eq!(std::fs::read_to_string(moved[0].join("reference/a.md")).unwrap(), "old a\n");
    assert!(!moved[0].join("reference/b.md").exists());
    assert!(previous(&h).is_empty(), "{:?}", previous(&h));
    assert!(draft.join("reference/b.md").is_file(), "the draft stays");

    assert_eq!(last_notice(&h, &session, "master_changed_skill"), json!({"slug": "guide", "bots": "@plan, @feature"}));
    assert!(events(&mut ui).contains(&UiEvent::SkillsChanged));
}

#[tokio::test]
async fn a_draft_named_after_its_slug_keeps_the_skills_name() {
    let h = harness();
    let (session, mp) = masters_turn(&h, None).await;
    let written = h.engine.create_skill(skill("Weekly Report", "Write it.")).unwrap();
    assert_eq!(written.slug, "weekly-report");
    let draft = copy(&h, &mp, "weekly-report").await;
    let text = std::fs::read_to_string(draft.join("SKILL.md")).unwrap();
    std::fs::write(draft.join("SKILL.md"), text.replace("Write it.", "Write it on Fridays.")).unwrap();
    trash(&h);

    let text = output(run(&h, &mp, update("weekly-report", &draft)).await);
    assert!(text.starts_with("Updated the skill weekly-report. No bot uses it. "), "{text}");
    assert_eq!(skill_named(&h, "weekly-report").name, "Weekly Report");
    assert!(h.engine.skill_body(&written.id).unwrap().contains("Write it on Fridays."));
    assert_eq!(last_notice(&h, &session, "master_changed_unused_skill"), json!({"slug": "weekly-report"}));
    assert!(notices(&h, &session, "master_changed_skill").is_empty(), "no \"used by —\"");
}

#[tokio::test]
async fn skill_update_refuses() {
    let h = harness();
    let (_, mp) = masters_turn(&h, None).await;
    let _ = guide(&h).await;
    let moved = trash(&h);
    let library = h.engine.skills_dir().join("guide");
    let before = files(&library);
    let master_before = files(&h.engine.skills_dir().join("sonata-master"));
    let drafts = h.paths.bots_root.join("master/skill-drafts/session-1");

    let mine = copy(&h, &mp, "sonata-master").await;
    std::fs::write(mine.join("NOTE.md"), "more rights").unwrap();
    let e = run(&h, &mp, update("sonata-master", &mine)).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::NotAllowed);
    assert_eq!(
        e.message,
        "The skill sonata-master is your own playbook: only the user changes it, in the app. Ask them to make this \
         change there."
    );

    let draft = copy(&h, &mp, "guide").await;
    std::fs::write(draft.join("reference/a.md"), "new a\n").unwrap();
    let e = run(&h, &mp, update("nope", &draft)).await.error.unwrap();
    assert_eq!(
        (e.code, e.message.as_str()),
        (ErrorCode::BadRequest, "No skill named nope. Run `sbot skills` to see the library.")
    );

    let elsewhere = tempfile::tempdir().unwrap();
    let outside = skill_folder(elsewhere.path(), "guide");
    let (_, other_pane) = user_session(&h, "master", "Something else.").await;
    let theirs = copy(&h, &other_pane, "guide").await;
    for folder in [&outside, &theirs, &drafts] {
        let e = run(&h, &mp, update("guide", folder)).await.error.unwrap();
        assert_eq!(e.code, ErrorCode::BadRequest, "{}", folder.display());
        assert_eq!(
            e.message,
            format!("Update skills only from folders in this conversation's drafts: {}/<name>.", drafts.display())
        );
    }

    let secret = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(secret.path(), draft.join("reference/keys")).unwrap();
    let e = run(&h, &mp, update("guide", &draft)).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::BadRequest);
    assert!(e.message.starts_with("The draft holds a link, which Sonata does not copy: "), "{}", e.message);
    assert!(e.message.ends_with("reference/keys. Replace it with the file itself."), "{}", e.message);
    std::fs::remove_file(draft.join("reference/keys")).unwrap();

    std::fs::remove_file(draft.join("SKILL.md")).unwrap();
    let e = run(&h, &mp, update("guide", &draft)).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::BadRequest);
    assert!(e.message.starts_with("The draft cannot replace the skill: "), "{}", e.message);
    assert!(e.message.ends_with("guide has no SKILL.md."), "{}", e.message);

    assert_eq!(files(&library), before);
    assert_eq!(files(&h.engine.skills_dir().join("sonata-master")), master_before);
    assert!(trashed(&moved).is_empty());
    assert!(previous(&h).is_empty(), "{:?}", previous(&h));
}

#[tokio::test]
async fn an_unchanged_draft_changes_nothing() {
    let h = harness();
    let (session, mp) = masters_turn(&h, None).await;
    let _ = guide(&h).await;
    let moved = trash(&h);
    let before = version_of(&h, "plan").await;
    let draft = copy(&h, &mp, "guide").await;
    assert_eq!(output(run(&h, &mp, update("guide", &draft)).await), "No change: guide already matches the draft.");
    assert_eq!(version_of(&h, "plan").await, before);

    std::fs::write(draft.join("reference/a.md"), "new a\n").unwrap();
    assert!(output(run(&h, &mp, update("guide", &draft)).await).starts_with("Updated the skill guide."));
    assert_eq!(version_of(&h, "plan").await, before + 1);
    let mut ui = h.engine.subscribe_ui();
    assert_eq!(output(run(&h, &mp, update("guide", &draft)).await), "No change: guide already matches the draft.");
    assert_eq!(version_of(&h, "plan").await, before + 1);
    assert_eq!(trashed(&moved).len(), 1);
    assert!(previous(&h).is_empty(), "{:?}", previous(&h));
    assert_eq!(notices(&h, &session, "master_changed_skill").len(), 1);
    assert!(!events(&mut ui).contains(&UiEvent::SkillsChanged));
}

#[tokio::test]
async fn a_failed_update_leaves_the_library_as_it_was() {
    let h = harness();
    let (session, mp) = masters_turn(&h, None).await;
    let _ = guide(&h).await;
    let moved = trash(&h);
    let library = h.engine.skills_dir().join("guide");
    let before = files(&library);
    let version = version_of(&h, "plan").await;
    let draft = copy(&h, &mp, "guide").await;
    std::fs::write(draft.join("reference/a.md"), "new a\n").unwrap();

    let skills_dir = h.engine.skills_dir();
    let mode = |m: u32| std::os::unix::fs::PermissionsExt::from_mode(m);
    std::fs::set_permissions(&skills_dir, mode(0o555)).unwrap();
    let e = run(&h, &mp, update("guide", &draft)).await.error.unwrap();
    std::fs::set_permissions(&skills_dir, mode(0o755)).unwrap();
    assert_eq!(e.code, ErrorCode::Internal);
    assert!(e.message.starts_with("Could not update the skill: "), "{}", e.message);
    assert_eq!(files(&library), before);
    assert!(previous(&h).is_empty(), "the copy is removed: {:?}", previous(&h));

    std::fs::remove_dir(h.paths.previous_dir()).unwrap();
    std::fs::write(h.paths.previous_dir(), "in the way").unwrap();
    let e = run(&h, &mp, update("guide", &draft)).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::Internal);
    assert_eq!(files(&library), before);

    assert!(trashed(&moved).is_empty());
    assert!(notices(&h, &session, "master_changed_skill").is_empty());
    assert_eq!(version_of(&h, "plan").await, version);
}

#[tokio::test]
async fn bot_edit_changes_instructions_and_skills() {
    let h = harness();
    let (session, mp) = masters_turn(&h, None).await;
    h.engine.create_skill(skill("sonata-seo", "Rank.")).unwrap();
    let moved = trash(&h);
    let review = bot_named(&h, "review").await;
    let old_instructions = review.instructions.clone();
    let mut ui = h.engine.subscribe_ui();

    let command = edit("@review", Some("Reviews for SEO.\n"), &["sonata-seo"], &["sonata-review"]);
    assert_eq!(
        output(run(&h, &mp, command).await),
        "Updated @review. Its sessions start with the new version at their next message. The previous version is in \
         the Trash."
    );
    let after = bot_named(&h, "review").await;
    assert_eq!(after.instructions, format!("Reviews for SEO.\n\n{}", checkout_paragraph(None)));
    let skills: Vec<String> = h
        .engine
        .list_skills()
        .unwrap()
        .into_iter()
        .filter(|v| after.skill_ids.contains(&v.skill.id))
        .map(|v| v.skill.slug)
        .collect();
    assert_eq!(sorted(skills), ["sonata", "sonata-seo"]);
    assert_eq!(after.config_version, review.config_version + 1);
    assert_eq!(
        (after.display_name.as_str(), after.color.as_str()),
        (review.display_name.as_str(), review.color.as_str())
    );
    let updated = events(&mut ui).into_iter().find_map(|e| match e {
        UiEvent::BotUpdated { bot } => Some(bot),
        _ => None,
    });
    assert_eq!(updated.expect("a bot_updated event").bot, after);

    let moved = trashed(&moved);
    assert_eq!(moved.len(), 1);
    assert!(moved[0].file_name().unwrap().to_string_lossy().ends_with("-bot-review.md"), "{}", moved[0].display());
    let old = std::fs::read_to_string(&moved[0]).unwrap();
    assert!(old.starts_with("# @review before "), "{old}");
    assert!(old.contains("\n\nSkills: sonata, sonata-review\n\n"), "{old}");
    assert!(old.ends_with(&format!("{}\n", old_instructions.trim_end())), "{old}");
    assert!(previous(&h).is_empty(), "{:?}", previous(&h));
    assert_eq!(last_notice(&h, &session, "master_changed_bot"), json!({"handle": "review"}));

    let command = edit("review", Some("Reviews for SEO."), &["sonata-seo"], &["sonata-review"]);
    assert_eq!(output(run(&h, &mp, command).await), "No change: @review already has these instructions and skills.");
    assert_eq!(bot_named(&h, "review").await.config_version, after.config_version);
    assert_eq!(notices(&h, &session, "master_changed_bot").len(), 1);

    let command = edit("@review", None, &["sonata-review"], &[]);
    assert!(output(run(&h, &mp, command).await).starts_with("Updated @review."));
    let again = bot_named(&h, "review").await;
    assert_eq!(again.instructions, after.instructions);
    assert_eq!(again.skill_ids.len(), 3);
}

#[tokio::test]
async fn bot_edit_sets_keeps_and_clears_the_model() {
    let h = harness();
    let (_, mp) = masters_turn(&h, None).await;
    assert!(output(run(&h, &mp, edit_model("@review", Some(" openai/gpt-6 "))).await).starts_with("Updated @review."));
    assert_eq!(bot_named(&h, "review").await.model.as_deref(), Some("openai/gpt-6"));
    run(&h, &mp, edit("@review", Some("Reviews more."), &[], &[])).await;
    assert_eq!(bot_named(&h, "review").await.model.as_deref(), Some("openai/gpt-6"), "no --model keeps it");
    assert!(output(run(&h, &mp, edit_model("@review", Some("  "))).await).starts_with("Updated @review."));
    assert_eq!(bot_named(&h, "review").await.model, None);
}

#[tokio::test]
async fn the_previous_version_names_the_model_it_had() {
    let h = harness();
    let (_, mp) = masters_turn(&h, None).await;
    run(&h, &mp, edit_model("@review", Some("openai/gpt-6"))).await;
    let moved = trash(&h);
    run(&h, &mp, edit_model("@review", Some("openai/gpt-7"))).await;
    let moved = trashed(&moved);
    let old = std::fs::read_to_string(&moved[0]).unwrap();
    assert!(old.contains("\nSkills: sonata, sonata-review\nModel: openai/gpt-6\n\n"), "{old}");
}

#[tokio::test]
async fn an_edit_of_only_the_model_is_a_change() {
    let h = harness();
    let (_, mp) = masters_turn(&h, None).await;
    let before = version_of(&h, "review").await;
    let reply = output(run(&h, &mp, edit_model("@review", Some("sonnet"))).await);
    assert!(reply.starts_with("Updated @review."), "{reply}");
    assert_eq!(version_of(&h, "review").await, before + 1);
    let same = output(run(&h, &mp, edit_model("@review", Some("sonnet"))).await);
    assert!(same.starts_with("No change"), "{same}");
}

#[tokio::test]
async fn bot_edit_keeps_the_checkout_paragraph_current() {
    let h = harness();
    let checkout = tempfile::tempdir().unwrap();
    let (_, mp) = masters_turn(&h, Some(checkout.path())).await;
    trash(&h);
    let current = checkout_paragraph(Some(checkout.path()));
    let paragraphs = |text: &str| text.matches("The project folder is at").count();

    assert!(output(run(&h, &mp, edit("@query", Some("Queries the database."), &[], &[])).await).starts_with("Updated"));
    let query = bot_named(&h, "query").await;
    assert_eq!(query.instructions, format!("Queries the database.\n\n{current}"));

    let old = checkout_paragraph(Some(Path::new("/old/sonata")));
    let text = format!("Runs queries.\n\n{old}\n\nAsk before writing.\n");
    assert!(output(run(&h, &mp, edit("@query", Some(&text), &[], &[])).await).starts_with("Updated"));
    let query = bot_named(&h, "query").await;
    assert_eq!(query.instructions, format!("Runs queries.\n\nAsk before writing.\n\n{current}"));
    assert_eq!(paragraphs(&query.instructions), 1);

    let other = tempfile::tempdir().unwrap();
    h.engine.set_project_folder(other.path()).unwrap();
    let query = bot_named(&h, "query").await;
    assert_eq!(
        query.instructions,
        format!("Runs queries.\n\nAsk before writing.\n\n{}", checkout_paragraph(Some(other.path())))
    );

    let writer = h.engine.create_bot(claude_bot("Writer")).unwrap();
    let text = format!("Writes.\n\n{old}\n");
    assert!(output(run(&h, &mp, edit("@writer", Some(&text), &[], &[])).await).starts_with("Updated"));
    assert_eq!(bot_named(&h, &writer.handle).await.instructions, format!("Writes.\n\n{old}"));
}

#[tokio::test]
async fn bot_edit_refuses_a_checkout_line_the_app_cannot_replace() {
    for checkout in [true, false] {
        let h = harness();
        let folder = tempfile::tempdir().unwrap();
        let (session, mp) = masters_turn(&h, checkout.then_some(folder.path())).await;
        let moved = trash(&h);
        let query = bot_named(&h, "query").await;
        let old = checkout_paragraph(Some(Path::new("/old/sonata")));
        for text in [
            "Runs queries. The project folder is at `/Users/me/sonata`, use it.\n\nAsk first.".to_string(),
            format!("Runs queries.\n\nThe project folder is at `/Users/me/sonata`.\n\n{old}"),
        ] {
            let e = run(&h, &mp, edit("@query", Some(&text), &[], &[])).await.error.unwrap();
            assert_eq!(e.code, ErrorCode::BadRequest, "{text}");
            assert_eq!(
                e.message,
                "The instructions hold a line about the project folder that is not the app's paragraph. Leave the \
                 project folder paragraph out of the instructions; the app adds the current one."
            );
        }
        assert_eq!(bot_named(&h, "query").await, query);
        assert!(trashed(&moved).is_empty());
        assert!(previous(&h).is_empty(), "{:?}", previous(&h));
        assert!(notices(&h, &session, "master_changed_bot").is_empty());

        let text = format!("Runs queries.\n\n{old}\n\nAsk first.");
        assert!(output(run(&h, &mp, edit("@query", Some(&text), &[], &[])).await).starts_with("Updated @query."));
        let current = checkout_paragraph(checkout.then_some(folder.path()));
        assert_eq!(bot_named(&h, "query").await.instructions, format!("Runs queries.\n\nAsk first.\n\n{current}"));
        let other = tempfile::tempdir().unwrap();
        h.engine.set_project_folder(other.path()).unwrap();
        assert_eq!(
            bot_named(&h, "query").await.instructions,
            format!("Runs queries.\n\nAsk first.\n\n{}", checkout_paragraph(Some(other.path())))
        );
    }
}

#[tokio::test]
async fn bot_edit_refuses_master_and_unknown_skills() {
    let h = harness();
    let (_, mp) = masters_turn(&h, None).await;
    let moved = trash(&h);
    let master = bot_named(&h, "master").await;
    let review = bot_named(&h, "review").await;

    let e = run(&h, &mp, edit("@master", Some("No limits."), &[], &["sonata-master"])).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::NotAllowed);
    assert_eq!(
        e.message,
        "@master is you: only the user changes your instructions and skills, in the app. Ask them to make this \
         change there."
    );
    for (command, message) in [
        (edit("@review", Some("New."), &["nope"], &[]), "No skill named nope. Run `sbot skills` to see the library."),
        (edit("@review", Some("New."), &[], &["gone"]), "No skill named gone. Run `sbot skills` to see the library."),
    ] {
        let e = run(&h, &mp, command).await.error.unwrap();
        assert_eq!((e.code, e.message.as_str()), (ErrorCode::BadRequest, message));
    }
    let e = run(&h, &mp, edit("@nobody", Some("New."), &[], &[])).await.error.unwrap();
    assert_eq!(
        (e.code, e.message.as_str()),
        (ErrorCode::UnknownRecipient, "No bot named @nobody. Run `sbot bots` to see your teammates.")
    );

    assert_eq!(bot_named(&h, "master").await, master);
    assert_eq!(bot_named(&h, "review").await, review);
    assert!(trashed(&moved).is_empty());
    assert!(previous(&h).is_empty(), "{:?}", previous(&h));
}

#[tokio::test]
async fn without_a_trash_the_previous_version_is_kept() {
    let h = harness();
    let (_, mp) = masters_turn(&h, None).await;
    let _ = guide(&h).await;
    let draft = copy(&h, &mp, "guide").await;
    std::fs::write(draft.join("reference/a.md"), "new a\n").unwrap();

    let kept_at = |text: &str| -> PathBuf {
        let (_, path) = text.split_once(" The previous version is kept at ").unwrap_or_else(|| panic!("{text}"));
        PathBuf::from(path.strip_suffix('.').unwrap())
    };
    let text = output(run(&h, &mp, update("guide", &draft)).await);
    let kept = kept_at(&text);
    assert!(kept.starts_with(h.paths.previous_dir()), "{}", kept.display());
    assert_eq!(std::fs::read_to_string(kept.join("reference/a.md")).unwrap(), "old a\n");

    let text = output(run(&h, &mp, edit("@query", Some("Queries."), &[], &[])).await);
    let kept = kept_at(&text);
    assert!(std::fs::read_to_string(&kept).unwrap().starts_with("# @query before "));

    h.engine.set_trash(Arc::new(|_: &Path| Err("the Trash is full".to_string())));
    let text = output(run(&h, &mp, edit("@query", Some("Runs queries."), &[], &[])).await);
    assert!(kept_at(&text).is_file());
}
