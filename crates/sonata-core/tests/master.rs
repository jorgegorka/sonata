mod support;

use std::path::{Path, PathBuf};

use bridge_proto::{BridgeCommand, BridgeResponse, Created, Draft, ErrorCode};
use serde_json::json;
use sonata_core::bundled::{self, checkout_paragraph, master_instructions, NEW_ASK};
use sonata_core::engine::UiEvent;
use sonata_core::envelope::WORKFLOW_REQUEST_NOTE;
use sonata_core::model::*;
use sonata_core::runtime::RuntimeEvent;
use sonata_core::store::Store;
use sonata_core::{CoreError, Refusal};
use support::*;

async fn master(h: &Harness) -> Bot {
    let id = h.engine.master_id().unwrap().expect("Master was created");
    h.engine.list_bots().await.unwrap().into_iter().find(|v| v.bot.id == id).expect("Master exists").bot
}

async fn bot(h: &Harness, handle: &str) -> Bot {
    h.engine.list_bots().await.unwrap().into_iter().find(|v| v.bot.handle == handle).unwrap().bot
}

async fn handles(h: &Harness) -> Vec<String> {
    h.engine.list_bots().await.unwrap().into_iter().map(|v| v.bot.handle).collect()
}

fn skill_slugs_of(h: &Harness, bot: &Bot) -> Vec<String> {
    let skills = h.engine.list_skills().unwrap();
    bot.skill_ids.iter().map(|id| skills.iter().find(|v| &v.skill.id == id).unwrap().skill.slug.clone()).collect()
}

async fn master_session(h: &Harness, body: &str) -> (Session, String) {
    let master = master(h).await;
    let (session, _) = h.engine.start_session_with_message(&master.id, body, &[]).unwrap();
    h.engine.pump_session(&session.id).await;
    let pane = pane(h, &session);
    (session, pane)
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

async fn forget_master(h: &Harness) {
    let master = master(h).await;
    let store = Store::open(&h.paths.db_path()).unwrap();
    store.delete_bot(&master.id).unwrap();
    store.delete_setting("sonata_master").unwrap();
    std::fs::remove_dir(h.paths.bots_root.join(&master.handle)).unwrap();
    let skill = h.engine.list_skills().unwrap().into_iter().find(|v| v.skill.slug == "sonata-master").unwrap().skill;
    std::fs::remove_dir_all(h.engine.delete_skill(&skill.id).unwrap()).unwrap();
}

fn edit_of(bot: &Bot, instructions: &str) -> NewBot {
    NewBot {
        display_name: bot.display_name.clone(),
        color: bot.color.clone(),
        provider: bot.provider,
        model: bot.model.clone(),
        instructions: instructions.to_string(),
        skill_ids: bot.skill_ids.clone(),
    }
}

#[tokio::test]
async fn master_is_listed_first_with_a_user_created_team() {
    let h = harness();
    let checkout = tempfile::tempdir().unwrap();
    setup_team(&h, Some(checkout.path()));
    let bots = h.engine.list_bots().await.unwrap();
    assert_eq!(
        bots.iter().map(|v| v.bot.handle.as_str()).collect::<Vec<_>>(),
        ["master", "research", "plan", "feature", "debug", "query", "interview", "review", "testing"]
    );
    let master = &bots[0].bot;
    assert_eq!(h.engine.master_id().unwrap().as_deref(), Some(master.id.as_str()));
    assert_eq!(skill_slugs_of(&h, master), ["sonata-master"]);
    assert_eq!((master.display_name.as_str(), master.color.as_str()), ("Master", "#4f46e5"));
    assert_eq!((master.provider, master.model.as_deref()), (Provider::Claude, None));
    assert_eq!(master.instructions, master_instructions(Some(checkout.path())));
    assert!(master.instructions.contains(&checkout_paragraph(Some(checkout.path()))));
    let research = &bots[1].bot;
    assert!(master.created_at <= research.created_at);
}

#[tokio::test]
async fn masters_session_gets_its_playbook_and_instructions() {
    let h = harness();
    setup_team(&h, None);
    master_session(&h, "Build TASK-1.").await;
    let dir = h.paths.bot_dir("master");
    let playbook = std::fs::read_to_string(dir.join(".claude/skills/sonata-master/SKILL.md")).unwrap();
    assert!(playbook.contains("name: sonata-master\n"), "{playbook}");
    assert!(playbook.contains("without the user's authorization"), "{playbook}");
    assert!(!dir.join(".claude/skills/sonata/SKILL.md").exists());
    let agents = std::fs::read_to_string(dir.join("AGENTS.md")).unwrap();
    assert!(agents.contains("You are the orchestrator of the Sonata bots."), "{agents}");
    assert!(agents.contains("`.claude/skills/sonata-master/SKILL.md`"), "{agents}");
}

#[tokio::test]
async fn masters_playbook_discovers_workflows_and_requires_approval_for_new_helpers() {
    let h = harness();
    h.engine.finish_setup(None).unwrap();
    let body = std::fs::read_to_string(h.paths.skills_dir().join("sonata-master/SKILL.md")).unwrap();
    for text in ["sbot workflows", "sbot workflow run", "after the user approves", "No team, project integration"] {
        assert!(body.contains(text), "{text}");
    }
}

#[tokio::test]
async fn an_older_install_gets_master_once_at_launch() {
    let h = harness_after(|_| {});
    let checkout = tempfile::tempdir().unwrap();
    setup_team(&h, Some(checkout.path()));
    forget_master(&h).await;
    assert_eq!(h.engine.master_id().unwrap(), None);
    let research = bot(&h, "research").await;

    let again = relaunch(&h);
    let added = again.engine.ensure_default_bots().unwrap();
    assert_eq!(added.iter().map(|b| b.handle.as_str()).collect::<Vec<_>>(), ["master"]);
    let master = master(&again).await;
    assert_eq!(master.id, added[0].id);
    assert_eq!(handles(&again).await[0], "master");
    assert_eq!(skill_slugs_of(&again, &master), ["sonata-master"]);
    assert_eq!(master.instructions, master_instructions(Some(checkout.path())));
    assert_eq!(bot(&again, "research").await, research);

    assert!(again.engine.ensure_default_bots().unwrap().is_empty());
    assert_eq!(handles(&again).await.iter().filter(|handle| handle.starts_with("master")).count(), 1);
}

#[tokio::test]
async fn a_deleted_master_does_not_come_back() {
    let h = harness_after(|_| {});
    setup_team(&h, None);
    let master = master(&h).await;
    h.engine.delete_bot(&master.id).await.unwrap();

    assert!(h.engine.ensure_default_bots().unwrap().is_empty());
    assert!(h.engine.master_bot().unwrap().is_none());
    let again = relaunch(&h);
    assert!(again.engine.ensure_default_bots().unwrap().is_empty());
    assert!(!handles(&again).await.iter().any(|handle| handle.starts_with("master")));
    assert!(again.engine.master_bot().unwrap().is_none());
}

#[tokio::test]
async fn master_leaves_a_users_own_master_alone() {
    let h = harness();
    let mine = h.engine.create_bot(claude_bot("Master")).unwrap();
    assert_eq!(mine.handle, "master");
    setup_team(&h, None);
    let id = h.engine.master_id().unwrap().unwrap();
    assert_ne!(id, mine.id);
    let bots = h.engine.list_bots().await.unwrap();
    assert_eq!((bots[0].bot.id.as_str(), bots[0].bot.handle.as_str()), (id.as_str(), "master-2"));
    assert_eq!(bots.iter().find(|v| v.bot.id == mine.id).unwrap().bot, mine);
}

#[tokio::test]
async fn a_master_with_another_handle_has_the_rights_and_every_generated_line_names_it() {
    let h = harness_after(|_| {});
    let mine = h.engine.create_bot(claude_bot("Master")).unwrap();
    setup_team(&h, None);
    assert_eq!((mine.handle.as_str(), master(&h).await.handle.as_str()), ("master", "master-2"));
    let (own, _) = h.engine.start_session_with_message(&mine.id, "hi", &[]).unwrap();
    h.engine.pump_session(&own.id).await;
    let refused = h.engine.handle_bridge(request(&pane(&h, &own), BridgeCommand::Skills)).await.error.unwrap();
    assert_eq!(
        (refused.code, refused.message.as_str()),
        (ErrorCode::NotAllowed, "Only @master-2 can use `sbot skills`.")
    );
    let (m, m_pane) = master_session(&h, "Build TASK-1.").await;
    assert!(h.engine.handle_bridge(request(&m_pane, BridgeCommand::Skills)).await.skills.is_some());
    assert!(h.engine.handle_bridge(send(&m_pane, "@feature", "Build TASK-1, plan first.")).await.ok);
    assert!(h.engine.handle_bridge(reply(&m_pane, "Started @feature.")).await.ok);
    finish_turn(&h, &m_pane).await;
    let f = opened(&h, &bot(&h, "feature").await, &m);
    let f_pane = pane(&h, &f);
    assert!(h.engine.handle_bridge(ask(&f_pane, "Plan: steps 1 to 3. Approve?")).await.ok);
    assert_eq!(last_notice(&h, &m, "session_waiting")["handle"], "feature");
    finish_turn(&h, &f_pane).await;
    user_says(&h, &f, "Approved.").await;
    let prompt = h.rt.state().prompts.iter().rev().find(|(p, _)| *p == f_pane).unwrap().1.clone();
    assert!(
        prompt.ends_with(
            "You are still working on @master-2's request. When it is done, send the final result with `sbot result`."
        ),
        "{prompt}"
    );
    let done = h.engine.handle_bridge(result(&f_pane, "Branch TASK-1 at abc123.")).await;
    assert_eq!(done.to.as_deref(), Some("@master-2"), "{done:?}");
}

#[tokio::test]
async fn an_empty_master_folder_left_by_a_crash_is_reused() {
    let h = harness();
    std::fs::create_dir_all(h.paths.bots_root.join("master")).unwrap();
    setup_team(&h, None);
    assert_eq!(master(&h).await.handle, "master");
    assert!(!h.paths.bots_root.join("master-2").exists());
}

#[tokio::test]
async fn a_master_folder_with_files_is_not_reused() {
    let h = harness();
    std::fs::create_dir_all(h.paths.bots_root.join("master")).unwrap();
    std::fs::write(h.paths.bots_root.join("master/MEMORY.md"), "# Memory\n").unwrap();
    setup_team(&h, None);
    assert_eq!(master(&h).await.handle, "master-2");
    assert_eq!(std::fs::read_to_string(h.paths.bots_root.join("master/MEMORY.md")).unwrap(), "# Memory\n");
}

#[tokio::test]
async fn a_checkout_change_reaches_master_and_any_bot_with_the_paragraph() {
    let h = harness();
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    setup_team(&h, Some(a.path()));
    let seo = format!("Manages SEO for the Sonata site.\n\n{}", checkout_paragraph(Some(a.path())));
    let seo = h.engine.create_bot(NewBot { instructions: seo, ..claude_bot("SEO") }).unwrap();
    let plain = h.engine.create_bot(claude_bot("Plain")).unwrap();

    let kept = h.engine.set_project_folder(b.path()).unwrap();
    assert!(kept.is_empty(), "{kept:?}");
    assert_eq!(master(&h).await.instructions, master_instructions(Some(b.path())));
    assert_eq!(
        bot(&h, &seo.handle).await.instructions,
        format!("Manages SEO for the Sonata site.\n\n{}", checkout_paragraph(Some(b.path())))
    );
    assert_eq!(bot(&h, &plain.handle).await, plain);
}

#[tokio::test]
async fn a_checkout_change_reports_a_master_whose_paragraph_the_user_edited() {
    let h = harness();
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    setup_team(&h, Some(a.path()));
    let master = master(&h).await;
    let master = h.engine.update_bot(&master.id, edit_of(&master, "Orchestrate the Sonata bots.")).unwrap();
    assert_eq!(h.engine.set_project_folder(b.path()).unwrap(), ["master"]);
    assert_eq!(bot(&h, "master").await, master);
}

async fn master_plan_exchange(h: &Harness, hops: i64) -> (Session, String, String) {
    setup_team(h, None);
    let (master_session, mp) = master_session(h, "Get the plan.").await;
    assert!(h.engine.handle_bridge(send(&mp, "@plan", "Plan it.")).await.ok);
    settle().await;
    let plan = bot(h, "plan").await;
    let pp = pane(h, &opened(h, &plan, &master_session));
    let mut hop = 1;
    while hop < hops {
        assert!(h.engine.handle_bridge(reply(&pp, "Plan.")).await.ok);
        finish_turn(h, &pp).await;
        finish_turn(h, &mp).await;
        hop += 1;
        if hop < hops {
            assert!(h.engine.handle_bridge(send(&mp, "@plan", "More.")).await.ok);
            settle().await;
            hop += 1;
        }
    }
    (master_session, mp, pp)
}

#[tokio::test]
async fn a_chain_master_takes_part_in_gets_five_times_the_limit() {
    let h = harness_after(|s| s.set_setting("hop_limit", "1").unwrap());
    let (session, mp, _) = master_plan_exchange(&h, 6).await;
    let refused = h.engine.handle_bridge(send(&mp, "@plan", "Even more.")).await.error.unwrap();
    assert_eq!(refused.code, ErrorCode::ChainLimitReached);
    assert!(refused.message.ends_with("send your result to the user with `sbot send user`."), "{}", refused.message);
    assert_eq!(last_notice(&h, &session, "chain_limit_reached")["limit"], "5");
}

#[tokio::test]
async fn past_the_limit_a_bot_in_masters_chain_is_told_to_report_to_master() {
    let h = harness_after(|s| s.set_setting("hop_limit", "1").unwrap());
    let (_, _, pp) = master_plan_exchange(&h, 5).await;
    let refused = h.engine.handle_bridge(send(&pp, "@feature", "Build it.")).await.error.unwrap();
    assert_eq!(refused.code, ErrorCode::ChainLimitReached);
    assert_eq!(
        refused.message,
        "This exchange between bots has reached its limit of 5 messages. Do not message other bots about it \
         again, except @master: send it your result with `sbot send @master`."
    );
}

fn plan_message_at(h: &Harness, hop: i64) -> Message {
    let threads = h.engine.bot_threads().unwrap();
    assert_eq!(threads.len(), 1, "{threads:?}");
    h.engine.thread_messages(&threads[0].id).unwrap().into_iter().find(|m| m.hop == hop).expect("a message at that hop")
}

#[tokio::test]
async fn messages_to_master_get_past_the_limit() {
    let h = harness_after(|s| s.set_setting("hop_limit", "1").unwrap());
    let (_, mp, pp) = master_plan_exchange(&h, 5).await;
    assert!(h.engine.handle_bridge(reply(&pp, "Plan.")).await.ok);
    let message = plan_message_at(&h, 6);
    assert_eq!(message.kind, MessageKind::Text);
    assert_ne!(message.status, MessageStatus::Failed);
    finish_turn(&h, &pp).await;
    finish_turn(&h, &mp).await;
    assert_eq!(plan_message_at(&h, 6).status, MessageStatus::Delivered);
}

#[tokio::test]
async fn a_fallback_to_master_past_the_limit_is_delivered() {
    let h = harness_after(|s| s.set_setting("hop_limit", "1").unwrap());
    setup_team(&h, None);
    let (m1, m1_pane) = master_session(&h, "Get it done.").await;
    let mut asker = m1;
    let mut asker_pane = m1_pane;
    for handle in ["plan", "feature", "debug"] {
        let next = bot(&h, handle).await;
        assert!(h.engine.handle_bridge(send(&asker_pane, &format!("@{handle}"), "Go on.")).await.ok);
        settle().await;
        asker = opened(&h, &next, &asker);
        asker_pane = pane(&h, &asker);
    }
    assert!(h.engine.handle_bridge(send(&asker_pane, "@master", "Back to you.")).await.ok);
    settle().await;
    let master = master(&h).await;
    let m2 = h
        .engine
        .sessions_of(&master.id)
        .unwrap()
        .into_iter()
        .find(|s| s.opened_by.as_deref() == Some(asker.id.as_str()))
        .unwrap();
    let m2_pane = pane(&h, &m2);
    let plan = bot(&h, "plan").await;
    assert!(h.engine.handle_bridge(send(&m2_pane, "@plan", "Once more.")).await.ok);
    settle().await;
    let p2 = opened(&h, &plan, &m2);
    finish_turn(&h, &pane(&h, &p2)).await;
    let fallback = h
        .engine
        .bot_threads()
        .unwrap()
        .iter()
        .flat_map(|t| h.engine.thread_messages(&t.id).unwrap())
        .find(|m| m.kind == MessageKind::RawFallback)
        .expect("a fallback answer");
    assert_eq!(fallback.hop, 6);
    assert_ne!(fallback.status, MessageStatus::Failed);
}

async fn plan_pane(h: &Harness) -> String {
    let plan = bot(h, "plan").await;
    let (session, _) = h.engine.start_session_with_message(&plan.id, "hi", &[]).unwrap();
    h.engine.pump_session(&session.id).await;
    pane(h, &session)
}

#[tokio::test]
async fn only_master_may_read_bots_and_skills() {
    let h = harness();
    setup_team(&h, Some(tempfile::tempdir().unwrap().path()));
    let plan_pane = plan_pane(&h).await;
    let refused = h.engine.handle_bridge(request(&plan_pane, BridgeCommand::Skills)).await.error.unwrap();
    assert_eq!(refused.code, ErrorCode::NotAllowed);
    assert_eq!(refused.message, "Only @master can use `sbot skills`.");
    let refused = h.engine.handle_bridge(request(&plan_pane, BridgeCommand::BotShow { handle: "@plan".into() })).await;
    assert_eq!(refused.error.unwrap().message, "Only @master can use `sbot bot show`.");
    let refused = h.engine.handle_bridge(request("w9:p9", BridgeCommand::Skills)).await.error.unwrap();
    assert_eq!(refused.code, ErrorCode::NotABot);
}

#[tokio::test]
async fn bot_show_gives_master_the_whole_bot() {
    let h = harness();
    setup_team(&h, Some(tempfile::tempdir().unwrap().path()));
    let (_, master_pane) = master_session(&h, "hi").await;
    let detail = h
        .engine
        .handle_bridge(request(&master_pane, BridgeCommand::BotShow { handle: "@plan".into() }))
        .await
        .bot
        .unwrap();
    assert_eq!(detail.role, "Example plan role.");
    assert_eq!(detail.skills.iter().map(|s| s.slug.as_str()).collect::<Vec<_>>(), ["sonata", "sonata-plan"]);
    assert!(detail.instructions.contains(NEW_ASK));
    let unknown = h
        .engine
        .handle_bridge(request(&master_pane, BridgeCommand::BotShow { handle: "@nobody".into() }))
        .await
        .error
        .unwrap();
    assert_eq!(unknown.code, ErrorCode::UnknownRecipient);
    assert_eq!(unknown.message, "No bot named @nobody. Run `sbot bots` to see your teammates.");
}

#[tokio::test]
async fn skills_lists_the_library_with_its_users() {
    let h = harness();
    setup_team(&h, Some(tempfile::tempdir().unwrap().path()));
    let (_, master_pane) = master_session(&h, "hi").await;
    let response = h.engine.handle_bridge(request(&master_pane, BridgeCommand::Skills)).await;
    let skills = response.skills.unwrap();
    let sonata = skills.iter().find(|s| s.slug == "sonata").unwrap();
    assert_eq!(sonata.used_by.len(), 8);
    let drafts = h.paths.bots_root.join("master/skill-drafts/session-1");
    assert_eq!(response.drafts_dir, Some(drafts.to_string_lossy().into_owned()));
    assert!(!drafts.exists());
}

async fn copy(h: &Harness, pane: &str, slug: &str) -> Draft {
    let response = h.engine.handle_bridge(request(pane, BridgeCommand::SkillCopy { slug: slug.into() })).await;
    response.draft.unwrap_or_else(|| panic!("no draft: {:?}", response.error))
}

async fn add(h: &Harness, pane: &str, folder: &Path, recreate: bool) -> BridgeResponse {
    let folder = folder.to_string_lossy().into_owned();
    h.engine.handle_bridge(request(pane, BridgeCommand::SkillAdd { folder, recreate })).await
}

async fn drafting(h: &Harness) -> (Session, String, PathBuf) {
    setup_team(h, None);
    let (session, pane) = master_session(h, "Make an SEO bot.").await;
    (session, pane, h.paths.bots_root.join("master/skill-drafts/session-1"))
}

fn has_skill(h: &Harness, slug: &str) -> bool {
    h.engine.list_skills().unwrap().iter().any(|v| v.skill.slug == slug)
}

#[tokio::test]
async fn skill_copy_puts_a_draft_in_the_sessions_own_folder_and_resumes_it() {
    let h = harness();
    let (_, master_pane, _) = drafting(&h).await;
    let d = copy(&h, &master_pane, "sonata-plan").await;
    assert_eq!(PathBuf::from(&d.path), h.paths.bots_root.join("master/skill-drafts/session-1/sonata-plan"));
    assert!(!d.resumed);
    let library = h.engine.skills_dir().join("sonata-plan/SKILL.md");
    let draft = PathBuf::from(&d.path);
    assert_eq!(std::fs::read_to_string(draft.join("SKILL.md")).unwrap(), std::fs::read_to_string(library).unwrap());
    std::fs::write(draft.join("NOTE.md"), "mine").unwrap();
    let again = copy(&h, &master_pane, "sonata-plan").await;
    assert!(again.resumed);
    assert_eq!(again.path, d.path);
    assert_eq!(std::fs::read_to_string(PathBuf::from(&again.path).join("NOTE.md")).unwrap(), "mine");
}

#[tokio::test]
async fn skill_copy_refuses_a_slug_the_library_does_not_have() {
    let h = harness();
    let (_, master_pane, drafts) = drafting(&h).await;
    for slug in ["nope", "../sonata-plan"] {
        let response =
            h.engine.handle_bridge(request(&master_pane, BridgeCommand::SkillCopy { slug: slug.into() })).await;
        let e = response.error.unwrap();
        assert_eq!(e.code, ErrorCode::BadRequest);
        assert_eq!(e.message, format!("No skill named {slug}. Run `sbot skills` to see the library."));
    }
    assert!(!drafts.exists());
}

#[tokio::test]
async fn only_master_may_copy_and_add_skills() {
    let h = harness();
    setup_team(&h, None);
    let plan_pane = plan_pane(&h).await;
    let response =
        h.engine.handle_bridge(request(&plan_pane, BridgeCommand::SkillCopy { slug: "sonata-plan".into() })).await;
    assert_eq!(response.error.unwrap().message, "Only @master can use `sbot skill copy`.");
    let elsewhere = tempfile::tempdir().unwrap();
    let refused = add(&h, &plan_pane, &skill_folder(elsewhere.path(), "sonata-seo"), false).await.error.unwrap();
    assert_eq!(
        (refused.code, refused.message.as_str()),
        (ErrorCode::NotAllowed, "Only @master can use `sbot skill add`.")
    );
    assert!(!h.paths.bots_root.join("plan/skill-drafts").exists());
}

#[tokio::test]
async fn two_master_sessions_get_separate_drafts() {
    let h = harness();
    let (_, p1, _) = drafting(&h).await;
    let (_, p2) = master_session(&h, "Make a docs bot.").await;
    let one = copy(&h, &p1, "sonata-plan").await;
    let two = copy(&h, &p2, "sonata-plan").await;
    assert_eq!(PathBuf::from(&two.path), h.paths.bots_root.join("master/skill-drafts/session-2/sonata-plan"));
    assert!(!two.resumed, "session 2 gets its own copy");
    assert_ne!(one.path, two.path);
}

#[tokio::test]
async fn skill_add_takes_only_this_sessions_drafts() {
    let h = harness();
    let (_, p1, drafts) = drafting(&h).await;
    let (_, p2) = master_session(&h, "Make a docs bot.").await;
    let elsewhere = tempfile::tempdir().unwrap();
    let outside = skill_folder(elsewhere.path(), "sonata-seo");
    let theirs = PathBuf::from(copy(&h, &p2, "sonata-plan").await.path);
    std::fs::create_dir_all(&drafts).unwrap();
    let escape = drafts.join("../session-2/sonata-plan");
    for folder in [&outside, &theirs, &escape, &drafts] {
        let e = add(&h, &p1, folder, false).await.error.unwrap();
        assert_eq!(e.code, ErrorCode::BadRequest, "{}", folder.display());
        assert_eq!(
            e.message,
            format!("Add only folders from this conversation's drafts: {}/<name>.", drafts.display())
        );
    }
    assert!(!has_skill(&h, "sonata-seo") && !has_skill(&h, "sonata-plan-2"));
}

#[tokio::test]
async fn skill_add_refuses_a_draft_with_a_link() {
    let h = harness();
    let (_, master_pane, drafts) = drafting(&h).await;
    let draft = skill_folder(&drafts, "sonata-seo");
    let secret = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(secret.path(), draft.join("scripts/keys")).unwrap();
    let e = add(&h, &master_pane, &draft, false).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::BadRequest);
    assert!(e.message.starts_with("The draft holds a link, which Sonata does not copy: "), "{}", e.message);
    assert!(e.message.ends_with("scripts/keys. Replace it with the file itself."), "{}", e.message);
    assert!(!has_skill(&h, "sonata-seo"));
}

#[tokio::test]
async fn skill_copy_writes_through_no_link() {
    let h = harness();
    let (_, master_pane, _) = drafting(&h).await;
    let mine = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(mine.path(), h.paths.bots_root.join("master/skill-drafts")).unwrap();
    let response =
        h.engine.handle_bridge(request(&master_pane, BridgeCommand::SkillCopy { slug: "sonata-plan".into() })).await;
    let e = response.error.unwrap();
    assert_eq!(e.code, ErrorCode::Internal);
    assert!(e.message.starts_with("Could not copy the skill: "), "{}", e.message);
    assert!(e.message.contains("only writes skills into real folders"), "{}", e.message);
    assert_eq!(std::fs::read_dir(mine.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn skill_add_says_what_a_draft_lacks() {
    let h = harness();
    let (_, master_pane, drafts) = drafting(&h).await;
    std::fs::create_dir_all(drafts.join("sonata-seo")).unwrap();
    let e = add(&h, &master_pane, &drafts.join("sonata-seo"), false).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::BadRequest);
    assert!(e.message.starts_with("The draft cannot be added: "), "{}", e.message);
    assert!(e.message.ends_with("sonata-seo has no SKILL.md."), "{}", e.message);
}

#[tokio::test]
async fn skill_add_never_replaces_a_library_skill() {
    let h = harness();
    let (_, master_pane, _) = drafting(&h).await;
    let library = h.engine.skills_dir().join("sonata-plan");
    let before = std::fs::read_to_string(library.join("SKILL.md")).unwrap();
    let draft = PathBuf::from(copy(&h, &master_pane, "sonata-plan").await.path);
    std::fs::write(draft.join("EXTRA.md"), "more").unwrap();
    let created = add(&h, &master_pane, &draft, false).await.created.unwrap();
    assert_eq!(created, Created { name: "sonata-plan-2".into(), already: false });
    assert_eq!(std::fs::read_to_string(library.join("SKILL.md")).unwrap(), before);
    assert!(!library.join("EXTRA.md").exists());
    assert!(h.engine.skills_dir().join("sonata-plan-2/EXTRA.md").is_file());
}

#[tokio::test]
async fn skill_add_waits_for_a_turn_on_the_users_message() {
    let h = harness();
    let (master_session, mp, drafts) = drafting(&h).await;
    let draft = skill_folder(&drafts, "sonata-seo");
    assert!(h.engine.handle_bridge(send(&mp, "@plan", "Plan it.")).await.ok);
    settle().await;
    let plan = bot(&h, "plan").await;
    let pp = pane(&h, &opened(&h, &plan, &master_session));
    assert!(h.engine.handle_bridge(reply(&pp, "Plan.")).await.ok);
    finish_turn(&h, &pp).await;
    finish_turn(&h, &mp).await;
    let refused = "Create skills and bots only while answering the user's message, after they approved the draft.";
    let e = add(&h, &mp, &draft, false).await.error.unwrap();
    assert_eq!((e.code, e.message.as_str()), (ErrorCode::NotAllowed, refused));
    assert!(!copy(&h, &mp, "sonata-plan").await.resumed);
    finish_turn(&h, &mp).await;
    assert_eq!(add(&h, &mp, &draft, false).await.error.unwrap().code, ErrorCode::NotAllowed);
    assert!(!has_skill(&h, "sonata-seo"));
}

#[tokio::test]
async fn adding_again_returns_the_skill_an_edit_is_refused_a_deletion_needs_recreate() {
    let h = harness();
    let (_, master_pane, drafts) = drafting(&h).await;
    let draft = skill_folder(&drafts, "sonata-seo");
    let first = add(&h, &master_pane, &draft, false).await.created.unwrap();
    assert_eq!(first, Created { name: "sonata-seo".into(), already: false });
    let again = add(&h, &master_pane, &draft, false).await.created.unwrap();
    assert_eq!(again, Created { name: first.name.clone(), already: true });
    assert_eq!(h.engine.list_skills().unwrap().iter().filter(|v| v.skill.slug.starts_with("sonata-seo")).count(), 1);

    let original = std::fs::read_to_string(draft.join("SKILL.md")).unwrap();
    std::fs::write(draft.join("SKILL.md"), original.replace("# sonata-seo", "# SEO for Sonata")).unwrap();
    let e = add(&h, &master_pane, &draft, false).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::CreatedDifferently);
    assert_eq!(
        e.message,
        "This conversation already added the skill sonata-seo from this draft, which has changed since. A changed \
         skill needs a new draft folder."
    );

    std::fs::write(draft.join("SKILL.md"), &original).unwrap();
    let skill = h.engine.list_skills().unwrap().into_iter().find(|v| v.skill.slug == "sonata-seo").unwrap().skill;
    std::fs::remove_dir_all(h.engine.delete_skill(&skill.id).unwrap()).unwrap();
    let e = add(&h, &master_pane, &draft, false).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::CreatedThenDeleted);
    assert_eq!(
        e.message,
        "The skill sonata-seo this conversation added was deleted in the app. Ask the user; only if they want it \
         again, repeat the command with --recreate."
    );
    assert!(!has_skill(&h, "sonata-seo"));
    let recreated = add(&h, &master_pane, &draft, true).await.created.unwrap();
    assert_eq!(recreated, Created { name: "sonata-seo".into(), already: false });
    assert!(add(&h, &master_pane, &draft, false).await.created.unwrap().already);
}

#[tokio::test]
async fn adding_a_skill_posts_a_notice_and_tells_the_ui() {
    let h = harness();
    let (session, master_pane, drafts) = drafting(&h).await;
    let draft = skill_folder(&drafts, "sonata-seo");
    let mut rx = h.engine.subscribe_ui();
    add(&h, &master_pane, &draft, false).await.created.unwrap();
    assert_eq!(last_notice(&h, &session, "master_added_skill"), json!({"slug": "sonata-seo"}));
    assert!(events(&mut rx).contains(&UiEvent::SkillsChanged));
    assert_eq!(serde_json::to_value(UiEvent::SkillsChanged).unwrap(), json!({"type": "skills_changed"}));
    assert!(add(&h, &master_pane, &draft, false).await.created.unwrap().already);
    assert!(!events(&mut rx).contains(&UiEvent::SkillsChanged));
    let notices = h.engine.session_messages(&session.id).unwrap();
    assert_eq!(notices.iter().filter(|m| m.system_code.as_deref() == Some("master_added_skill")).count(), 1);
}

#[tokio::test]
async fn removing_a_session_hands_back_its_drafts() {
    let h = harness();
    let (session, master_pane, drafts) = drafting(&h).await;
    copy(&h, &master_pane, "sonata-plan").await;
    let removed = h.engine.remove_session(&session.id).await.unwrap();
    assert_eq!(removed.drafts, vec![drafts]);
}

#[tokio::test]
async fn drafts_never_made_or_behind_a_link_are_not_handed_back() {
    let h = harness();
    let (s1, _, _) = drafting(&h).await;
    let (s2, _) = master_session(&h, "Make a docs bot.").await;
    let (s3, _) = master_session(&h, "Make a QA bot.").await;
    assert!(h.engine.remove_session(&s1.id).await.unwrap().drafts.is_empty(), "session 1 drafted nothing");

    let mine = tempfile::tempdir().unwrap();
    let root = h.paths.bots_root.join("master/skill-drafts");
    std::fs::create_dir_all(&root).unwrap();
    std::os::unix::fs::symlink(mine.path(), root.join("session-2")).unwrap();
    assert!(h.engine.remove_session(&s2.id).await.unwrap().drafts.is_empty());

    std::fs::remove_dir_all(&root).unwrap();
    std::fs::create_dir_all(mine.path().join("session-3")).unwrap();
    std::os::unix::fs::symlink(mine.path(), &root).unwrap();
    assert!(h.engine.remove_session(&s3.id).await.unwrap().drafts.is_empty());
    assert!(mine.path().join("session-3").is_dir());
}

#[tokio::test]
async fn a_deleted_copy_is_named_by_the_slug_it_got() {
    let h = harness();
    let (_, master_pane, _) = drafting(&h).await;
    let draft = PathBuf::from(copy(&h, &master_pane, "sonata-plan").await.path);
    let added = add(&h, &master_pane, &draft, false).await.created.unwrap();
    assert_eq!(added, Created { name: "sonata-plan-2".into(), already: false });
    let skill = h.engine.list_skills().unwrap().into_iter().find(|v| v.skill.slug == "sonata-plan-2").unwrap().skill;
    std::fs::remove_dir_all(h.engine.delete_skill(&skill.id).unwrap()).unwrap();
    let e = add(&h, &master_pane, &draft, false).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::CreatedThenDeleted);
    assert!(e.message.starts_with("The skill sonata-plan-2 this conversation added was deleted"), "{}", e.message);
}

async fn create(
    h: &Harness,
    pane: &str,
    name: &str,
    skills: &[&str],
    instructions: &str,
    recreate: bool,
) -> BridgeResponse {
    let command = BridgeCommand::BotCreate {
        name: name.into(),
        color: "#0ea5e9".into(),
        provider: "claude".into(),
        skills: skills.iter().map(|s| s.to_string()).collect(),
        instructions: instructions.into(),
        recreate,
        model: None,
    };
    h.engine.handle_bridge(request(pane, command)).await
}

async fn create_on(h: &Harness, pane: &str, name: &str, provider: &str, model: Option<&str>) -> BridgeResponse {
    let command = BridgeCommand::BotCreate {
        name: name.into(),
        color: "#0ea5e9".into(),
        provider: provider.into(),
        skills: vec![],
        instructions: "Scouts.".into(),
        recreate: false,
        model: model.map(String::from),
    };
    h.engine.handle_bridge(request(pane, command)).await
}

#[tokio::test]
async fn master_creates_a_bot_with_a_model() {
    let h = harness();
    let (_, pane, _) = creating(&h).await;
    create_on(&h, &pane, "Scout", "opencode", Some(" opencode-go/kimi-k3 ")).await.created.unwrap();
    let scout = bot(&h, "scout").await;
    assert_eq!((scout.provider, scout.model.as_deref()), (Provider::Opencode, Some("opencode-go/kimi-k3")));
    create_on(&h, &pane, "Plain", "claude", None).await.created.unwrap();
    assert_eq!(bot(&h, "plain").await.model, None);
}

#[tokio::test]
async fn a_repeat_with_another_model_is_other_settings() {
    let h = harness();
    let (_, pane, _) = creating(&h).await;
    create_on(&h, &pane, "Scout", "opencode", Some("opencode-go/kimi-k3")).await.created.unwrap();
    let same = create_on(&h, &pane, "Scout", "opencode", Some("opencode-go/kimi-k3")).await.created.unwrap();
    assert!(same.already);
    let e = create_on(&h, &pane, "Scout", "opencode", Some("opencode-go/glm-5.3")).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::CreatedDifferently);
}

#[tokio::test]
async fn a_create_without_a_model_is_the_request_it_was_before() {
    let h = harness();
    let (_, pane, _) = creating(&h).await;
    create(&h, &pane, "SEO", &[], "Scouts.", false).await.created.unwrap();
    let again = create_on(&h, &pane, "SEO", "claude", Some("  ")).await;
    let again = again.created.unwrap_or_else(|| panic!("{:?}", again.error));
    assert!(again.already);
}

#[tokio::test]
async fn master_cannot_create_an_opencode_bot_without_a_provider_part() {
    let h = harness();
    let (_, pane, _) = creating(&h).await;
    let e = create_on(&h, &pane, "Scout", "opencode", Some("kimi")).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::BadRequest);
    assert!(e.message.contains("provider/model"), "{}", e.message);
    assert!(!handles(&h).await.contains(&"scout".to_string()));
}

#[tokio::test]
async fn bot_show_names_the_model() {
    let h = harness();
    let (_, pane, _) = creating(&h).await;
    create_on(&h, &pane, "Scout", "opencode", Some("opencode-go/kimi-k3")).await.created.unwrap();
    let show = |handle: &str| BridgeCommand::BotShow { handle: handle.into() };
    let detail = h.engine.handle_bridge(request(&pane, show("@scout"))).await.bot.unwrap();
    assert_eq!(detail.model.as_deref(), Some("opencode-go/kimi-k3"));
    let plan = h.engine.handle_bridge(request(&pane, show("@plan"))).await.bot.unwrap();
    assert_eq!(plan.model, None);
}

fn library_skill(h: &Harness, slug: &str) {
    h.engine
        .create_skill(NewSkill { name: slug.into(), description: format!("{slug}."), body: "Body.".into() })
        .unwrap();
}

async fn creating(h: &Harness) -> (Session, String, tempfile::TempDir) {
    let checkout = tempfile::tempdir().unwrap();
    setup_team(h, Some(checkout.path()));
    library_skill(h, "sonata-seo");
    let (session, pane) = master_session(h, "Make an SEO bot.").await;
    (session, pane, checkout)
}

#[tokio::test]
async fn bot_create_makes_a_sonata_bot() {
    let h = harness();
    let (_, pane, checkout) = creating(&h).await;
    let created =
        create(&h, &pane, "SEO", &["sonata-seo"], "Manages SEO for the Sonata site.\n", false).await.created.unwrap();
    assert_eq!(created, Created { name: "seo".into(), already: false });
    let seo = bot(&h, "seo").await;
    assert_eq!(seo.display_name, "SEO");
    assert_eq!(seo.color, "#0ea5e9");
    assert_eq!(seo.provider, Provider::Claude);
    assert_eq!(skill_slugs_of(&h, &seo), ["sonata-seo"]);
    assert!(seo.instructions.starts_with("Manages SEO for the Sonata site.\n\n"));
    assert!(seo.instructions.ends_with(&checkout_paragraph(Some(checkout.path()))));
    create(&h, &pane, "Other", &["sonata", "sonata-seo", "sonata-seo"], "Does other things.", false)
        .await
        .created
        .unwrap();
    assert_eq!(skill_slugs_of(&h, &bot(&h, "other").await), ["sonata", "sonata-seo"]);
}

#[tokio::test]
async fn a_name_whose_handle_is_taken_gets_the_next_handle() {
    let h = harness();
    let (session, pane, _) = creating(&h).await;
    let review = bot(&h, "review").await.instructions;
    assert_eq!(create(&h, &pane, "Review", &[], "Reviews copy.\n", false).await.created.unwrap().name, "review-2");
    assert_eq!(bot(&h, "review").await.instructions, review);
    assert_eq!(last_notice(&h, &session, "master_created_bot")["handle"], "review-2");
}

#[tokio::test]
async fn bot_create_refuses_unknown_skills_and_providers() {
    let h = harness();
    let (_, pane, _) = creating(&h).await;
    let e = create(&h, &pane, "SEO", &["nope"], "Does SEO.", false).await.error.unwrap();
    assert_eq!((e.code, e.message.as_str()), (ErrorCode::BadRequest, "No skill named nope. Run `sbot skills`."));
    for provider in ["cursor", "gemini"] {
        let command = BridgeCommand::BotCreate {
            name: "SEO".into(),
            color: "#0ea5e9".into(),
            provider: provider.into(),
            skills: vec![],
            instructions: "Does SEO.".into(),
            recreate: false,
            model: None,
        };
        let e = h.engine.handle_bridge(request(&pane, command)).await.error.unwrap();
        assert_eq!(e.code, ErrorCode::BadRequest);
        assert_eq!(e.message, format!("provider {provider} is not available yet"));
    }
    assert!(!handles(&h).await.contains(&"seo".to_string()));
}

#[tokio::test]
async fn bot_create_refuses_a_color_that_is_not_rrggbb() {
    let h = harness();
    let (_, pane, _) = creating(&h).await;
    let command = |name: &str, color: &str| BridgeCommand::BotCreate {
        name: name.into(),
        color: color.into(),
        provider: "claude".into(),
        skills: vec![],
        instructions: "Does SEO.".into(),
        recreate: false,
        model: None,
    };
    for color in ["indigo", "#4f46e", "#4f46e5ff", "4f46e5", "#4g46e5", " #4f46e5", ""] {
        let e = h.engine.handle_bridge(request(&pane, command("SEO", color))).await.error.unwrap();
        assert_eq!(
            (e.code, e.message.as_str()),
            (ErrorCode::BadRequest, "Give the color as #rrggbb, for example #4f46e5."),
            "{color:?}"
        );
    }
    assert!(!handles(&h).await.contains(&"seo".to_string()));
    let made = h.engine.handle_bridge(request(&pane, command("SEO", "#4F46E5"))).await.created.unwrap();
    assert_eq!(bot(&h, &made.name).await.color, "#4F46E5");
}

#[tokio::test]
async fn bot_create_works_with_no_global_skill_and_no_selected_skills() {
    let h = harness();
    h.engine.finish_setup(None).unwrap();
    let (session, pane) = master_session(&h, "Create a helper.").await;
    let response = create(&h, &pane, "Helper", &[], "Help with the request.", false).await;
    assert!(response.ok, "{:?}", response.error);
    let helper = bot(&h, "helper").await;
    assert!(helper.skill_ids.is_empty());
    assert!(helper.instructions.contains(&checkout_paragraph(None)));
    assert_eq!(last_notice(&h, &session, "master_created_bot")["skills"], "—");
    assert_eq!(h.engine.list_skills().unwrap().len(), 1);
}

#[tokio::test]
async fn bot_create_waits_for_a_turn_on_the_users_message() {
    let h = harness();
    let (master_session, mp, _) = creating(&h).await;
    assert!(h.engine.handle_bridge(send(&mp, "@plan", "Plan it.")).await.ok);
    settle().await;
    let plan = bot(&h, "plan").await;
    let pp = pane(&h, &opened(&h, &plan, &master_session));
    assert!(h.engine.handle_bridge(reply(&pp, "Plan.")).await.ok);
    finish_turn(&h, &pp).await;
    finish_turn(&h, &mp).await;
    let e = create(&h, &mp, "SEO", &[], "Does SEO.", false).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::NotAllowed);
    assert!(e.message.starts_with("Create skills and bots only"), "{}", e.message);
    finish_turn(&h, &mp).await;
    assert_eq!(create(&h, &mp, "SEO", &[], "Does SEO.", false).await.error.unwrap().code, ErrorCode::NotAllowed);
    assert!(!handles(&h).await.contains(&"seo".to_string()));
}

#[tokio::test]
async fn only_master_may_create_bots() {
    let h = harness();
    setup_team(&h, None);
    let plan_pane = plan_pane(&h).await;
    let e = create(&h, &plan_pane, "SEO", &[], "Does SEO.", false).await.error.unwrap();
    assert_eq!((e.code, e.message.as_str()), (ErrorCode::NotAllowed, "Only @master can use `sbot bot create`."));
}

#[tokio::test]
async fn the_same_create_returns_the_bot_a_change_is_refused_a_deletion_needs_recreate() {
    let h = harness();
    let (_, pane, _) = creating(&h).await;
    let first = create(&h, &pane, "SEO", &["sonata-seo"], "Does SEO.", false).await.created.unwrap();
    assert_eq!(first, Created { name: "seo".into(), already: false });
    let id = bot(&h, "seo").await.id;
    let again = create(&h, &pane, "SEO", &["sonata-seo"], "Does SEO.", false).await.created.unwrap();
    assert_eq!(again, Created { name: "seo".into(), already: true });
    assert_eq!(h.engine.list_bots().await.unwrap().iter().filter(|v| v.bot.handle.starts_with("seo")).count(), 1);

    let e = create(&h, &pane, "seo", &["sonata-seo"], "Does more SEO.", false).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::CreatedDifferently);
    assert_eq!(
        e.message,
        "This conversation already created @seo as seo with other settings. A changed bot needs a new name."
    );

    std::fs::remove_dir_all(h.engine.delete_bot(&id).await.unwrap()).unwrap();
    let e = create(&h, &pane, "SEO", &["sonata-seo"], "Does SEO.", false).await.error.unwrap();
    assert_eq!(e.code, ErrorCode::CreatedThenDeleted);
    assert_eq!(
        e.message,
        "The bot @seo this conversation created was deleted in the app. Ask the user; only if they want it again, \
         repeat the command with --recreate."
    );
    assert!(!handles(&h).await.contains(&"seo".to_string()));
    let recreated = create(&h, &pane, "SEO", &["sonata-seo"], "Does SEO.", true).await.created.unwrap();
    assert_eq!(recreated, Created { name: "seo".into(), already: false });
    assert_ne!(bot(&h, "seo").await.id, id);
    assert!(create(&h, &pane, "SEO", &["sonata-seo"], "Does SEO.", false).await.created.unwrap().already);
}

#[tokio::test]
async fn creating_a_bot_posts_a_notice_and_tells_the_ui() {
    let h = harness();
    let (session, pane, _) = creating(&h).await;
    let mut rx = h.engine.subscribe_ui();
    create(&h, &pane, "SEO", &["sonata-seo"], "Does SEO.", false).await.created.unwrap();
    assert_eq!(last_notice(&h, &session, "master_created_bot"), json!({"handle": "seo", "skills": "sonata-seo"}));
    let added: Vec<BotView> = events(&mut rx)
        .into_iter()
        .filter_map(|e| match e {
            UiEvent::BotAdded { bot } => Some(bot),
            _ => None,
        })
        .collect();
    assert_eq!(added.len(), 1);
    assert_eq!((added[0].bot.handle.as_str(), added[0].status), ("seo", BotStatus::Offline));
    let json = serde_json::to_value(UiEvent::BotAdded { bot: added[0].clone() }).unwrap();
    assert_eq!((json["type"].as_str(), json["bot"]["handle"].as_str()), (Some("bot_added"), Some("seo")));
    assert!(create(&h, &pane, "SEO", &["sonata-seo"], "Does SEO.", false).await.created.unwrap().already);
    assert!(!events(&mut rx).iter().any(|e| matches!(e, UiEvent::BotAdded { .. })));
}

async fn user_says(h: &Harness, session: &Session, body: &str) -> Message {
    let message = h.engine.queue_user_message_to_session(&session.id, body, &[]).unwrap();
    h.engine.pump_session(&session.id).await;
    message
}

#[tokio::test]
async fn master_asking_the_user_on_a_bots_message_opens_no_request() {
    let h = harness_after(|_| {});
    setup_team(&h, None);
    h.rt.state().output = "terminal text".into();
    let plan_pane = plan_pane(&h).await;
    let plan_session = h.engine.sessions_of(&bot(&h, "plan").await.id).unwrap().remove(0);
    let note = h.engine.handle_bridge(send(&plan_pane, "@master", "A plan is ready.")).await.message_id.unwrap();
    settle().await;
    let m = opened(&h, &master(&h).await, &plan_session);
    let m_pane = pane(&h, &m);

    let resp = h.engine.handle_bridge(ask(&m_pane, "Shall @feature build it?")).await;
    assert!(resp.ok, "{resp:?}");
    let m_now = h.engine.find_session_view(&m.id).await.unwrap().session;
    assert_eq!(m_now.open_request_id, None, "Master owes no bot a result");
    assert_eq!(m_now.waiting_question_id, resp.message_id);
    assert_eq!(h.engine.answered_by(&note).unwrap(), resp.message_id, "the question still answers the turn");
    let plans_chat = h.engine.session_messages(&plan_session.id).unwrap();
    assert!(plans_chat.iter().all(|m| m.system_code.as_deref() != Some("session_waiting")), "{plans_chat:?}");
    finish_turn(&h, &m_pane).await;

    user_says(&h, &m, "Yes.").await;
    let prompt = h.rt.state().prompts.iter().rev().find(|(p, _)| *p == m_pane).unwrap().1.clone();
    assert!(!prompt.contains("sbot result"), "{prompt}");
    let refused = h.engine.handle_bridge(result(&m_pane, "Sent to @feature.")).await.error.unwrap();
    assert_eq!(refused.code, ErrorCode::NoOpenRequest);
    finish_turn(&h, &m_pane).await;
    let all: Vec<Message> =
        h.engine.bot_threads().unwrap().iter().flat_map(|t| h.engine.thread_messages(&t.id).unwrap()).collect();
    assert!(all.iter().all(|m| m.kind != MessageKind::RawFallback), "{all:?}");
}

#[tokio::test]
async fn a_delegated_ticket_runs_without_stray_fallbacks() {
    let h = harness_after(|_| {});
    setup_team(&h, None);
    h.rt.state().output = "terminal text".into();
    let (m, m_pane) = master_session(&h, "Build TASK-1.").await;
    let (feature, review) = (bot(&h, "feature").await, bot(&h, "review").await);
    let mut calls = Vec::new();

    let brief = h.engine.handle_bridge(send(&m_pane, "@feature", "Brief: build TASK-1, plan first.")).await;
    calls.push(h.engine.handle_bridge(reply(&m_pane, "Started @feature.")).await);
    let brief_id = brief.message_id.clone().expect("the brief has an id");
    calls.push(brief);
    finish_turn(&h, &m_pane).await;
    settle().await;

    let f = opened(&h, &feature, &m);
    let f_pane = pane(&h, &f);
    calls.push(h.engine.handle_bridge(send(&f_pane, "@master", "Which database?")).await);
    finish_turn(&h, &f_pane).await;
    calls.push(h.engine.handle_bridge(reply(&m_pane, "Postgres.")).await);
    finish_turn(&h, &m_pane).await;
    calls.push(h.engine.handle_bridge(ask(&f_pane, "Plan: steps 1 to 3. Approve?")).await);
    finish_turn(&h, &f_pane).await;

    user_says(&h, &f, "Change step 2.").await;
    calls.push(h.engine.handle_bridge(ask(&f_pane, "Revised plan. Approve?")).await);
    finish_turn(&h, &f_pane).await;

    let approved = user_says(&h, &f, "Approved.").await;
    calls.push(h.engine.handle_bridge(send(&f_pane, "@master", "Halfway.")).await);
    settle().await;
    calls.push(h.engine.handle_bridge(reply(&m_pane, "Noted.")).await);
    finish_turn(&h, &m_pane).await;

    let done = h.engine.handle_bridge(result(&f_pane, "Branch TASK-1 at abc123; gates clean.")).await;
    let result_id = done.message_id.clone().expect("the result has an id");
    calls.push(done);
    finish_turn(&h, &f_pane).await;
    finish_turn(&h, &f_pane).await;

    calls.push(h.engine.handle_bridge(send(&m_pane, "@review", "Local review of TASK-1 at abc123.")).await);
    finish_turn(&h, &m_pane).await;

    let r = opened(&h, &review, &m);
    let r_pane = pane(&h, &r);
    calls.push(h.engine.handle_bridge(reply(&r_pane, "abc123: 1 major at app/x.rb:3.")).await);
    finish_turn(&h, &r_pane).await;
    calls.push(h.engine.handle_bridge(send(&m_pane, "@feature", "Fix app/x.rb:3.")).await);
    finish_turn(&h, &m_pane).await;

    calls.push(h.engine.handle_bridge(reply(&f_pane, "Fixed at def456.")).await);
    finish_turn(&h, &f_pane).await;
    calls.push(h.engine.handle_bridge(send(&m_pane, "@review", "Re-review at def456.")).await);
    finish_turn(&h, &m_pane).await;
    calls.push(h.engine.handle_bridge(reply(&r_pane, "def456: no findings.")).await);
    finish_turn(&h, &r_pane).await;

    calls.push(h.engine.handle_bridge(send(&m_pane, "user", "Done. Push and open the MR?")).await);
    finish_turn(&h, &m_pane).await;
    finish_turn(&h, &f_pane).await;
    finish_turn(&h, &r_pane).await;

    assert!(calls.iter().all(|c| c.ok), "{calls:?}");

    let store = Store::open(&h.paths.db_path()).unwrap();
    let mut all: Vec<Message> =
        h.engine.bot_threads().unwrap().iter().flat_map(|t| h.engine.thread_messages(&t.id).unwrap()).collect();
    for bot in store.list_bots().unwrap() {
        for session in store.sessions_of(&bot.id).unwrap() {
            all.extend(h.engine.session_messages(&session.id).unwrap());
        }
    }
    assert!(all.iter().all(|m| m.kind != MessageKind::RawFallback), "{all:?}");

    let result_message = all.iter().find(|m| m.id == result_id).unwrap();
    assert_eq!(result_message.in_reply_to.as_deref(), Some(brief_id.as_str()));
    let clarifying = all.iter().find(|m| m.body == "Which database?").unwrap();
    assert_eq!(clarifying.in_reply_to.as_deref(), Some(brief_id.as_str()), "the question answered the brief in form");
    assert_eq!(h.engine.answered_by(&approved.id).unwrap().as_deref(), Some(result_id.as_str()));
    let f_now = h.engine.find_session_view(&f.id).await.unwrap().session;
    assert_eq!((f_now.open_request_id, f_now.waiting_question_id), (None, None));

    let chat = h.engine.session_messages(&m.id).unwrap();
    assert_eq!(chat.iter().filter(|m| m.system_code.as_deref() == Some("session_waiting")).count(), 1, "{chat:?}");
    let last = chat.iter().rev().find(|m| m.sender != USER && m.system_code.is_none()).unwrap();
    assert_eq!(last.body, "Done. Push and open the MR?");
}

fn playbook_skill(h: &Harness) -> (Skill, String) {
    let skill = h.engine.list_skills().unwrap().into_iter().find(|v| v.skill.slug == "sonata-master").unwrap().skill;
    let body = h.engine.skill_body(&skill.id).unwrap();
    (skill, body)
}

fn set_playbook(h: &Harness, playbook: &str) {
    let (skill, _) = playbook_skill(h);
    let edit =
        NewSkill { name: skill.name.clone(), description: skill.description.clone(), body: playbook.to_string() };
    h.engine.update_skill(&skill.id, edit).unwrap();
}

const CUSTOM_PLAYBOOK: &str = "User-authored coordination instructions.";

#[tokio::test]
async fn relaunch_preserves_the_current_master_playbook() {
    let h = harness_after(|_| {});
    h.engine.finish_setup(None).unwrap();
    let before = master(&h).await;
    let again = relaunch(&h);
    assert!(again.engine.ensure_default_bots().unwrap().is_empty());
    assert_eq!(playbook_skill(&again).1.trim_end(), bundled::master_skill_body().trim_end());
    assert_eq!(master(&again).await.config_version, before.config_version);
}

#[tokio::test]
async fn an_edited_playbook_is_kept() {
    let h = harness_after(|_| {});
    setup_team(&h, None);
    set_playbook(&h, &format!("{CUSTOM_PLAYBOOK}\nThe user's own line.\n"));
    let (_, edited) = playbook_skill(&h);
    let version = master(&h).await.config_version;

    let again = relaunch(&h);
    again.engine.ensure_default_bots().unwrap();
    assert_eq!(playbook_skill(&again).1, edited);
    assert_eq!(master(&again).await.config_version, version);
}

#[tokio::test]
async fn the_playbook_names_the_new_commands() {
    let body = bundled::master_skill_body();
    for command in [
        "sbot task create",
        "sbot tasks",
        "sbot task edit",
        "sbot task delete",
        "sbot sessions",
        "sbot session show",
        "sbot session archive",
        "sbot skill update",
        "sbot bot edit",
    ] {
        assert!(body.contains(command), "the playbook never names `{command}`");
    }
}

#[test]
fn the_playbook_lets_master_set_a_model_and_find_skills_on_every_cli() {
    let body = bundled::master_skill_body();
    for text in ["codex, grok or opencode", r#"--model "<model>""#, "provider/model", ".agents/skills"] {
        assert!(body.contains(text), "the playbook never says `{text}`");
    }
}

async fn requesting(h: &Harness, body: &str) -> (Session, Message, String) {
    let (session, message) = h.engine.start_workflow_request(body).unwrap();
    h.engine.pump_session(&session.id).await;
    let pane = pane(h, &session);
    (session, message, pane)
}

const SEO_REQUEST: &str = "SEO workflow\nFetch the metrics from a JSON endpoint, evaluate them and suggest \
     improvements, then open a ticket for each one I approve.";

#[tokio::test]
async fn a_workflow_request_opens_a_new_master_session_that_says_what_to_do() {
    let h = harness();
    setup_team(&h, None);
    let master = master(&h).await;
    let (earlier, earlier_pane) = master_session(&h, "Build TASK-1.").await;
    let (session, message, pane) = requesting(&h, SEO_REQUEST).await;
    assert_eq!(session.bot_id, master.id);
    assert_ne!(session.id, earlier.id);
    assert_ne!(pane, earlier_pane);
    assert_eq!(session.title.as_deref(), Some("SEO workflow"));
    assert_eq!(session.opened_by, None);
    assert_eq!((message.body.as_str(), message.workflow_request), (SEO_REQUEST, true));
    let stored = h.engine.session_messages(&session.id).unwrap();
    assert!(stored.iter().any(|m| m.id == message.id && m.workflow_request), "the mark is stored");
    let prompts = h.rt.state().prompts.clone();
    let text = &prompts.iter().find(|(to, _)| to == &pane).expect("the request was delivered").1;
    assert_eq!(
        text,
        &format!(
            "[Message from user · thread you↔user · language: English · from the Workflows page]\n{SEO_REQUEST}\n\n\
             {WORKFLOW_REQUEST_NOTE}"
        )
    );
    let plain = &prompts.iter().find(|(to, _)| to == &earlier_pane).unwrap().1;
    assert_eq!(plain, "[Message from user · thread you↔user · language: English]\nBuild TASK-1.");
    assert!(!h.engine.session_messages(&earlier.id).unwrap().iter().any(|m| m.workflow_request));
}

#[tokio::test]
async fn masters_turn_on_a_workflow_request_is_the_users() {
    let h = harness();
    let checkout = tempfile::tempdir().unwrap();
    setup_team(&h, Some(checkout.path()));
    library_skill(&h, "sonata-seo");
    let (_, _, pane) = requesting(&h, SEO_REQUEST).await;
    let created = create(&h, &pane, "SEO", &["sonata-seo"], "Evaluates Sonata's SEO metrics.\n", false).await;
    assert_eq!(created.created.unwrap(), Created { name: "seo".into(), already: false });
}

#[tokio::test]
async fn a_retried_workflow_request_keeps_its_mark() {
    let h = harness();
    setup_team(&h, None);
    let (session, message, pane) = requesting(&h, SEO_REQUEST).await;
    h.engine.on_runtime_event(RuntimeEvent::Exited { pane_id: pane }).await;
    settle().await;
    let interrupted = h.engine.session_messages(&session.id).unwrap().into_iter().find(|m| m.id == message.id);
    assert_eq!(interrupted.unwrap().status, MessageStatus::Interrupted);
    let copy = h.engine.retry_message(&message.id).unwrap();
    assert!(copy.workflow_request);
    assert_eq!(copy.recipient_session.as_deref(), Some(session.id.as_str()));
}

#[tokio::test]
async fn a_workflow_request_needs_master() {
    let h = harness();
    setup_team(&h, None);
    let master = master(&h).await;
    h.engine.delete_bot(&master.id).await.unwrap();
    let err = h.engine.start_workflow_request(SEO_REQUEST).unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::MasterMissing)), "{err}");
    let err = h.engine.start_workflow_request("  ").unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::MasterMissing)), "Master is checked first: {err}");
}
