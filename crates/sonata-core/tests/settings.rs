mod support;

use sonata_core::engine::MAX_HOP_LIMIT;
use sonata_core::model::*;
use sonata_core::runtime::RuntimeError;
use sonata_core::{CoreError, Refusal};
use support::*;

#[tokio::test]
async fn the_hop_limit_is_set_within_its_range_and_read_within_it() {
    let h = harness();
    assert_eq!(h.engine.hop_limit().unwrap(), 12);
    h.engine.set_hop_limit(5).unwrap();
    assert_eq!(h.engine.hop_limit().unwrap(), 5);
    for refused in [0, -3, MAX_HOP_LIMIT + 1] {
        let err = h.engine.set_hop_limit(refused).unwrap_err();
        assert!(matches!(err, CoreError::Refused(Refusal::HopLimitRange { min: 1, max: MAX_HOP_LIMIT })), "{err}");
    }
    let stored = harness_after(|store| store.set_setting("hop_limit", "-3").unwrap());
    assert_eq!(stored.engine.hop_limit().unwrap(), 1, "a stored value below 1 counts as 1");
}

#[tokio::test]
async fn setup_is_finished_once() {
    let h = harness();
    assert!(!h.engine.setup_done().unwrap());
    h.engine.finish_setup(None).unwrap();
    assert!(h.engine.setup_done().unwrap());
}

#[tokio::test]
async fn the_close_hint_is_shown_once() {
    let h = harness_after(|_| {});
    assert!(!h.engine.close_hint_shown().unwrap());
    h.engine.mark_close_hint_shown().unwrap();
    assert!(h.engine.close_hint_shown().unwrap());
    let again = relaunch(&h);
    assert!(again.engine.close_hint_shown().unwrap(), "remembered across runs");
}

#[tokio::test]
async fn the_bots_folder_can_be_chosen_until_the_first_bot_exists() {
    let h = harness();
    let chosen = h.paths.data_dir.parent().unwrap().join("chosen bots");
    h.engine.set_bots_root(&chosen).unwrap();
    assert_eq!(h.engine.paths().bots_root, chosen);
    let bot = h.engine.create_bot(claude_bot("Researcher")).unwrap();
    assert!(chosen.join("researcher").is_dir(), "the new bot's folder is in the chosen folder");
    assert_eq!(h.engine.bot_folder(&bot.id).unwrap(), chosen.join("researcher"));
    let err = h.engine.set_bots_root(&h.paths.bots_root).unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::BotsFolderLocked)), "{err}");
    let err = h.engine.set_bots_root(std::path::Path::new("relative/bots")).unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::BadFolder(_))), "{err}");
}

#[tokio::test]
async fn a_bot_folder_that_links_elsewhere_is_refused() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Researcher")).unwrap();
    let folder = h.paths.bot_dir(&bot.handle);
    let elsewhere = h.paths.data_dir.parent().unwrap().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::remove_dir_all(&folder).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &folder).unwrap();

    h.engine.queue_user_message(&bot.id, "hello").unwrap();
    h.engine.pump(&bot.id).await;

    assert!(h.rt.state().starts.is_empty(), "the CLI was not started");
    assert_eq!(status_of(&h, &bot.id).await, BotStatus::NeedsAttention);
    let codes: Vec<_> = h.engine.user_thread(&bot.id).unwrap().into_iter().filter_map(|m| m.system_code).collect();
    assert_eq!(codes, vec!["bot_folder_outside".to_string()]);
    assert!(std::fs::read_dir(&elsewhere).unwrap().next().is_none(), "nothing was written there");
}

#[tokio::test]
async fn the_runtime_health_comes_from_the_runtime() {
    let h = harness();
    assert_eq!(h.engine.runtime_health().await.unwrap().version, "0.9.3");
    h.rt.state().health = Err(RuntimeError::Other("herdr 0.8.0 speaks protocol 19".into()));
    assert!(h.engine.runtime_health().await.is_err());
}

fn link_folder_elsewhere(h: &Harness, handle: &str) -> std::path::PathBuf {
    let folder = h.paths.bot_dir(handle);
    let elsewhere = h.paths.data_dir.parent().unwrap().join(format!("elsewhere-{handle}"));
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::remove_dir_all(&folder).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &folder).unwrap();
    elsewhere
}

#[tokio::test]
async fn a_users_file_is_not_copied_into_a_folder_that_links_elsewhere() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Researcher")).unwrap();
    let elsewhere = link_folder_elsewhere(&h, &bot.handle);
    let file = h.paths.data_dir.parent().unwrap().join("note.md");
    std::fs::write(&file, "secret").unwrap();

    let err =
        h.engine.queue_user_message_with_files(&bot.id, "see", &[file.to_string_lossy().into_owned()]).unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::BotFolderOutside(_))), "{err}");
    assert!(std::fs::read_dir(&elsewhere).unwrap().next().is_none(), "nothing was copied there");
    assert!(h.engine.user_thread(&bot.id).unwrap().is_empty(), "no message was stored");
}

#[tokio::test]
async fn a_relaunched_app_uses_the_bots_folder_setup_chose() {
    let first = harness_after(|_| {});
    let chosen = first.paths.data_dir.parent().unwrap().join("chosen bots");
    first.engine.set_bots_root(&chosen).unwrap();

    let second = relaunch(&first);
    assert_eq!(second.engine.paths().bots_root, chosen);
    second.engine.create_bot(claude_bot("Researcher")).unwrap();
    assert!(chosen.join("researcher").is_dir(), "the bot lives in the chosen folder");
    assert!(!first.paths.bots_root.join("researcher").exists(), "and not in the default one");
}

#[tokio::test]
async fn a_bot_is_not_stored_when_its_folder_cannot_be_created() {
    let h = harness();
    std::fs::remove_dir_all(&h.paths.bots_root).unwrap();
    std::fs::write(&h.paths.bots_root, "not a folder").unwrap();
    assert!(h.engine.create_bot(claude_bot("Researcher")).is_err());
    assert!(h.engine.list_bots().await.unwrap().is_empty(), "no bot row is left behind");
}

#[tokio::test]
async fn a_bots_folder_that_cannot_be_written_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let h = harness();
    let locked = h.paths.data_dir.parent().unwrap().join("locked");
    std::fs::create_dir_all(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = h.engine.set_bots_root(&locked);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    let err = result.unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::BadFolder(_))), "{err}");
    assert_ne!(h.engine.paths().bots_root, locked, "the folder was not taken");
}

#[tokio::test]
async fn the_language_follows_the_choice_or_the_system() {
    let h = harness();
    let both = |h: &Harness| (h.engine.language_choice().unwrap(), h.engine.language().unwrap());
    assert_eq!(both(&h), ("system".to_string(), "en".to_string()));
    h.engine.set_system_language("es");
    assert_eq!(h.engine.language().unwrap(), "es");
    h.engine.set_language("fr").unwrap();
    assert_eq!(both(&h), ("fr".to_string(), "fr".to_string()));
    h.engine.set_language("system").unwrap();
    assert_eq!(h.engine.language().unwrap(), "es");
    let err = h.engine.set_language("pt").unwrap_err();
    assert!(matches!(err, CoreError::Refused(Refusal::LanguageUnknown(_))), "{err}");
    h.engine.set_system_language("pt");
    assert_eq!(h.engine.language().unwrap(), "en", "an unsupported system language means English");
}

#[tokio::test]
async fn an_unknown_stored_language_falls_back_to_the_system_language() {
    let h = harness_after(|store| store.set_setting("language", "pt").unwrap());
    h.engine.set_system_language("de");
    assert_eq!(h.engine.language_choice().unwrap(), "system", "a value the app does not know is System default");
    assert_eq!(h.engine.language().unwrap(), "de");
}

#[tokio::test]
async fn bots_are_told_the_resolved_language() {
    let h = harness();
    h.engine.set_system_language("es");
    let (bot, _) = bot_with_delivered(&h, "hola").await;
    let agents = std::fs::read_to_string(h.paths.bot_dir(&bot.handle).join("AGENTS.md")).unwrap();
    assert!(agents.contains("The user's preferred language is Spanish."), "{agents}");
    assert!(h.rt.state().prompts[0].1.contains("language: Spanish"));
}
