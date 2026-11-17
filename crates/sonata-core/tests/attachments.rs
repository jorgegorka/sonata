mod support;

use std::path::Path;

use bridge_proto::*;
use sonata_core::model::*;
use support::*;

fn write(dir: &Path, name: &str, text: &str) -> String {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path.to_string_lossy().into_owned()
}

fn send_files(pane: &str, to: &str, body: &str, files: Vec<String>) -> BridgeRequest {
    BridgeRequest {
        v: 1,
        pane_id: pane.into(),
        cli_session_id: None,
        command: BridgeCommand::Send { to: to.into(), body: body.into(), files },
    }
}

fn reply_files(pane: &str, body: &str, files: Vec<String>) -> BridgeRequest {
    BridgeRequest {
        v: 1,
        pane_id: pane.into(),
        cli_session_id: None,
        command: BridgeCommand::Reply { body: body.into(), files },
    }
}

#[tokio::test]
async fn a_users_file_is_copied_into_the_bots_inbox_and_listed_in_the_prompt() {
    let h = harness();
    let src = tempfile::tempdir().unwrap();
    let notes = write(src.path(), "my notes.md", "# Notes");
    let bot = h.engine.create_bot(claude_bot("Researcher")).unwrap();
    let msg = h.engine.queue_user_message_with_files(&bot.id, "", std::slice::from_ref(&notes)).unwrap();
    let stored = format!("{}-my notes.md", msg.id);
    let copy = h.paths.bot_dir("researcher").join("inbox").join(&stored);
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "# Notes");
    assert_eq!(
        msg.attachments,
        vec![Attachment { name: "my notes.md".into(), path: copy.to_string_lossy().into_owned() }]
    );
    assert!(Path::new(&notes).exists(), "the user's own file stays");
    h.engine.pump(&bot.id).await;
    let prompt = h.rt.state().prompts[0].1.clone();
    assert!(prompt.ends_with(&format!("\n\nAttachments:\n- `inbox/{stored}`")), "{prompt}");
}

#[tokio::test]
async fn a_user_message_needs_a_body_or_a_file() {
    let h = harness();
    let bot = h.engine.create_bot(claude_bot("Researcher")).unwrap();
    assert!(h.engine.queue_user_message_with_files(&bot.id, "  ", &[]).is_err());
}

#[tokio::test]
async fn a_file_sent_to_a_bot_is_copied_into_its_inbox() {
    let h = harness();
    team(&h).await;
    write(&h.paths.bot_dir("researcher"), "draft.md", "v1");
    let resp = h.engine.handle_bridge(send_files("w1:p1", "@writer", "Please review.", vec!["draft.md".into()])).await;
    assert!(resp.ok, "{resp:?}");
    settle().await;
    let stored = format!("{}-draft.md", resp.message_id.unwrap());
    assert_eq!(std::fs::read_to_string(h.paths.bot_dir("writer").join("inbox").join(&stored)).unwrap(), "v1");
    let (pane, prompt) = h.rt.state().prompts.last().unwrap().clone();
    assert_eq!(pane, "w1:p2");
    assert!(prompt.ends_with(&format!("Please review.\n\nAttachments:\n- `inbox/{stored}`")), "{prompt}");
}

#[tokio::test]
async fn a_file_for_the_user_stays_where_the_bot_keeps_it() {
    let h = harness();
    team(&h).await;
    let report = write(&h.paths.bot_dir("researcher"), "report.md", "R");
    let resp = h.engine.handle_bridge(reply_files("w1:p1", "Report attached.", vec![report.clone()])).await;
    let id = resp.message_id.expect("sent");
    assert_eq!(h.engine.attachment_path(&id, 0).unwrap(), Path::new(&report));
    assert!(h.engine.attachment_path(&id, 1).is_err());
}

#[tokio::test]
async fn bad_files_are_refused_with_their_codes_and_nothing_is_stored() {
    let h = harness();
    let (researcher, _, _) = team(&h).await;
    let dir = h.paths.bot_dir("researcher");
    let big = dir.join("big.bin");
    std::fs::File::create(&big).unwrap().set_len(sonata_core::attachments::MAX_FILE_BYTES + 1).unwrap();
    let before = h.engine.user_thread(&researcher.id).unwrap().len();
    let code = |files: Vec<String>| {
        let h = &h;
        async move { h.engine.handle_bridge(reply_files("w1:p1", "x", files)).await.error.map(|e| e.code) }
    };
    assert_eq!(code(vec!["missing.md".into()]).await, Some(ErrorCode::FileNotFound));
    assert_eq!(code(vec![dir.to_string_lossy().into_owned()]).await, Some(ErrorCode::FileNotFound));
    assert_eq!(code(vec![big.to_string_lossy().into_owned()]).await, Some(ErrorCode::FileTooLarge));
    assert_eq!(h.engine.user_thread(&researcher.id).unwrap().len(), before);
    let err = h.engine.queue_user_message_with_files(&researcher.id, "x", &["/nonexistent/a.md".into()]).unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err}");
}

#[tokio::test]
async fn a_bots_file_is_not_sent_to_a_bot_whose_folder_links_elsewhere() {
    let h = harness();
    let (researcher, writer, _) = team(&h).await;
    write(&h.paths.bot_dir("researcher"), "draft.md", "v1");
    let folder = h.paths.bot_dir("writer");
    let elsewhere = h.paths.data_dir.parent().unwrap().join("elsewhere-writer");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::remove_dir_all(&folder).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &folder).unwrap();

    let resp = h.engine.handle_bridge(send_files("w1:p1", "@writer", "Please review.", vec!["draft.md".into()])).await;
    assert!(!resp.ok, "{resp:?}");
    assert_eq!(resp.error.map(|e| e.code), Some(ErrorCode::Internal));
    assert!(std::fs::read_dir(&elsewhere).unwrap().next().is_none(), "nothing landed in the link target");
    let _ = (researcher, writer);
    assert!(h.engine.bot_threads().unwrap().is_empty(), "no message was stored");
}
