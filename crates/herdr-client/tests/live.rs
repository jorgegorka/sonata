use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use herdr_client::*;

static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Cleanup {
    bin: PathBuf,
    session: String,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::process::Command::new(&self.bin).args(["--session", &self.session, "server", "stop"]).output();
        let _ = std::process::Command::new(&self.bin).args(["session", "delete", &self.session]).output();
        let _ = std::fs::remove_file(server_config_path(&self.session));
    }
}

fn test_session() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    format!("sonata-test-{}-{nanos}", std::process::id())
}

#[tokio::test]
async fn live_session_tab_env_input_events_and_stop() {
    let Some(bin) = find_herdr_bin() else {
        eprintln!("herdr not installed; skipping live test");
        return;
    };
    let _turn = ONE_AT_A_TIME.lock().await;
    let session = test_session();
    let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };

    let server = HerdrServer::start(&bin, &session).await.unwrap();
    let c = server.client().clone();
    let pong = c.ping().await.unwrap();
    assert_eq!(pong.protocol, PINNED_PROTOCOL, "herdr {} speaks protocol {}", pong.version, pong.protocol);

    let dir = tempfile::tempdir().unwrap();
    let ws = c.workspace_create("live-test", dir.path()).await.unwrap();
    let ws_id = ws.workspace.expect("workspace in result").workspace_id;
    let env = BTreeMap::from([("SBOT_SOCKET".to_string(), "/x/bridge.sock".to_string())]);
    let tab = c.tab_create(&ws_id, "probe", dir.path(), &env).await.unwrap();
    let pane = tab.root_pane.pane_id.clone();
    let mut events = c.subscribe(&[Subscription::pane_exited()]).await.unwrap();

    c.pane_send_input(&pane, " unset HISTFILE; printf '__SONATA_%s__ %s\\n' DONE \"$SBOT_SOCKET\"", &["enter"])
        .await
        .unwrap();
    c.pane_wait_for_substring(&pane, "__SONATA_DONE__ /x/bridge.sock", 30_000).await.unwrap();
    let screen = c.pane_read(&pane, ReadSource::Visible, 50).await.unwrap();
    assert!(screen.contains("__SONATA_DONE__ /x/bridge.sock"), "screen was: {screen}");

    c.pane_send_input(&pane, "exit", &["enter"]).await.unwrap();
    let ev =
        tokio::time::timeout(Duration::from_secs(10), events.next()).await.expect("pane_exited within 10s").unwrap();
    assert_eq!(ev, Some(Event::PaneExited { pane_id: pane.clone() }));

    assert!(server_config_path(&session).exists(), "config should exist while running");
    server.stop().await.unwrap();
    assert!(!server_config_path(&session).exists(), "config should be removed by stop()");
    assert!(c.ping().await.is_err(), "server should be stopped");
}

#[tokio::test]
async fn live_server_env_reaches_panes() {
    let Some(bin) = find_herdr_bin() else {
        eprintln!("herdr not installed; skipping live test");
        return;
    };
    let _turn = ONE_AT_A_TIME.lock().await;
    let session = test_session();
    let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
    let env = [("SONATA_ENV_PROBE".to_string(), "from-the-server".to_string())];
    let server = HerdrServer::start_with_env(&bin, &session, &env).await.unwrap();
    let c = server.client().clone();
    let dir = tempfile::tempdir().unwrap();
    let ws_id = c.workspace_create("live-env", dir.path()).await.unwrap().workspace.expect("workspace").workspace_id;
    let tab = c.tab_create(&ws_id, "probe", dir.path(), &BTreeMap::new()).await.unwrap();
    let pane = tab.root_pane.pane_id.clone();
    c.pane_send_input(&pane, " unset HISTFILE; printf '__SONATA_%s__ %s\\n' ENV \"$SONATA_ENV_PROBE\"", &["enter"])
        .await
        .unwrap();
    c.pane_wait_for_substring(&pane, "__SONATA_ENV__ from-the-server", 30_000).await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn live_ansi_read_keeps_colors() {
    let Some(bin) = find_herdr_bin() else {
        eprintln!("herdr not installed; skipping live test");
        return;
    };
    let _turn = ONE_AT_A_TIME.lock().await;
    let session = test_session();
    let _cleanup = Cleanup { bin: bin.clone(), session: session.clone() };
    let server = HerdrServer::start(&bin, &session).await.unwrap();
    let c = server.client().clone();
    let dir = tempfile::tempdir().unwrap();
    let ws_id = c.workspace_create("live-ansi", dir.path()).await.unwrap().workspace.expect("workspace").workspace_id;
    let tab = c.tab_create(&ws_id, "probe", dir.path(), &BTreeMap::new()).await.unwrap();
    let pane = tab.root_pane.pane_id.clone();
    c.pane_send_input(&pane, " unset HISTFILE; printf '\\033[31m__SONATA_%s__\\033[0m\\n' 'RED'", &["enter"])
        .await
        .unwrap();
    c.pane_wait_for_substring(&pane, "__SONATA_RED__", 30_000).await.unwrap();
    let plain = c.pane_read_as(&pane, ReadSource::Visible, ReadFormat::Text, 50).await.unwrap();
    assert!(plain.contains("__SONATA_RED__") && !plain.contains('\u{1b}'), "text read: {plain:?}");
    let ansi = c.pane_read_as(&pane, ReadSource::Visible, ReadFormat::Ansi, 50).await.unwrap();
    assert!(ansi.contains("__SONATA_RED__") && ansi.contains("\u{1b}["), "ansi read: {ansi:?}");
    server.stop().await.unwrap();
}
