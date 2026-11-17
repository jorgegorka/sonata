mod support;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use herdr_client::*;
use serde_json::json;
use support::FakeHerdr;

#[tokio::test]
async fn ping_returns_version_and_protocol() {
    let fake = FakeHerdr::start(|_| vec![json!({"result":{"type":"pong","version":"0.8.0","protocol":19}})]);
    let pong = HerdrClient::new(&fake.socket).ping().await.unwrap();
    assert_eq!(pong.protocol, 19);
    assert_eq!(pong.version, "0.8.0");
    assert_eq!(fake.requests.lock().unwrap()[0]["method"], "ping");
}

#[tokio::test]
async fn each_request_uses_a_new_connection() {
    let fake = FakeHerdr::start(|_| vec![json!({"result":{"type":"pong","version":"0.8.0","protocol":19}})]);
    let client = HerdrClient::new(&fake.socket);
    client.ping().await.unwrap();
    client.ping().await.unwrap();
    assert_eq!(fake.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn server_error_maps_to_code() {
    let fake =
        FakeHerdr::start(|_| vec![json!({"error":{"code":"agent_not_found","message":"agent target nope not found"}})]);
    let err = HerdrClient::new(&fake.socket).agent_get("nope").await.unwrap_err();
    assert_eq!(err.code(), Some("agent_not_found"));
}

#[tokio::test]
async fn tab_create_sends_cwd_env_and_parses_ids() {
    let fake = FakeHerdr::start(|_| {
        vec![json!({"result":{"type":"tab_created",
            "tab":{"tab_id":"w1:t2","workspace_id":"w1","number":2,"label":"bot","focused":false,"pane_count":1,"agent_status":"idle"},
            "root_pane":{"pane_id":"w1:p2","tab_id":"w1:t2","workspace_id":"w1","terminal_id":"t2","focused":false,"agent_status":"idle","revision":0}}})]
    });
    let env = BTreeMap::from([("SBOT_SOCKET".to_string(), "/x/bridge.sock".to_string())]);
    let created = HerdrClient::new(&fake.socket).tab_create("w1", "bot", Path::new("/tmp/bot"), &env).await.unwrap();
    assert_eq!(created.root_pane.pane_id, "w1:p2");
    assert_eq!(created.tab.tab_id, "w1:t2");
    let req = fake.requests.lock().unwrap()[0].clone();
    assert_eq!(req["method"], "tab.create");
    assert_eq!(
        req["params"],
        json!({"workspace_id":"w1","label":"bot","cwd":"/tmp/bot","env":{"SBOT_SOCKET":"/x/bridge.sock"},"focus":false})
    );
}

#[tokio::test]
async fn agent_start_returns_session_id_when_reported() {
    let fake = FakeHerdr::start(|_| {
        vec![json!({"result":{"type":"agent_started","argv":["claude"],
            "agent":{"pane_id":"w1:p2","tab_id":"w1:t2","workspace_id":"w1","terminal_id":"t2","name":"researcher",
                     "agent_status":"idle","focused":false,"revision":3,
                     "agent_session":{"agent":"claude","kind":"id","source":"hook","value":"abc-123"}}}})]
    });
    let info = HerdrClient::new(&fake.socket)
        .agent_start("researcher", "claude", "w1:p2", &["--dangerously-skip-permissions".to_string()], 60_000)
        .await
        .unwrap();
    assert_eq!(info.session_id(), Some("abc-123"));
    assert_eq!(info.agent_status, AgentStatus::Idle);
    assert_eq!(
        fake.requests.lock().unwrap()[0]["params"],
        json!({"name":"researcher","kind":"claude","pane_id":"w1:p2","args":["--dangerously-skip-permissions"],"timeout_ms":60000})
    );
}

#[tokio::test]
async fn agent_read_requests_source_and_returns_text() {
    let fake = FakeHerdr::start(|_| {
        vec![json!({"result":{"type":"pane_read","read":{"pane_id":"w1:p2","workspace_id":"w1","tab_id":"w1:t2",
            "source":"recent_unwrapped","format":"text","text":"hello","revision":1,"truncated":false}}})]
    });
    let text = HerdrClient::new(&fake.socket).agent_read("w1:p2", ReadSource::RecentUnwrapped, 200).await.unwrap();
    assert_eq!(text, "hello");
    assert_eq!(
        fake.requests.lock().unwrap()[0]["params"],
        json!({"target":"w1:p2","source":"recent_unwrapped","lines":200})
    );
}

#[tokio::test]
async fn agent_read_as_requests_the_format() {
    let fake = FakeHerdr::start(|_| {
        vec![json!({"result":{"type":"pane_read","read":{"pane_id":"w1:p2","workspace_id":"w1","tab_id":"w1:t2",
            "source":"visible","format":"ansi","text":"\u{1b}[31mred\u{1b}[0m","revision":1,"truncated":false}}})]
    });
    let client = HerdrClient::new(&fake.socket);
    let text = client.agent_read_as("w1:p2", ReadSource::Visible, ReadFormat::Ansi, 40).await.unwrap();
    assert_eq!(text, "\u{1b}[31mred\u{1b}[0m");
    assert_eq!(
        fake.requests.lock().unwrap()[0]["params"],
        json!({"target":"w1:p2","source":"visible","format":"ansi","lines":40})
    );
}

#[tokio::test]
async fn a_herdr_that_never_answers_times_out() {
    let fake = FakeHerdr::start(|_| vec![]);
    let client = HerdrClient::new(&fake.socket).with_timeout(Duration::from_millis(200));
    let started = Instant::now();
    let err = client.ping().await.unwrap_err();
    assert!(matches!(&err, HerdrError::Io(e) if e.kind() == std::io::ErrorKind::TimedOut), "got {err:?}");
    assert_eq!(err.code(), None);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(fake.requests.lock().unwrap().len(), 1, "the request was sent");
}

#[tokio::test]
async fn a_subscription_that_is_never_confirmed_times_out() {
    let fake = FakeHerdr::start(|_| vec![]);
    let client = HerdrClient::new(&fake.socket).with_timeout(Duration::from_millis(200));
    let err = client.subscribe(&[Subscription::pane_exited()]).await.err().expect("timeout");
    assert!(matches!(&err, HerdrError::Io(e) if e.kind() == std::io::ErrorKind::TimedOut), "got {err:?}");
}

#[test]
fn calls_with_a_server_side_timeout_get_that_long_plus_ten_seconds() {
    let client = HerdrClient::new("/x.sock");
    assert_eq!(client.call_timeout(&json!({})), Duration::from_secs(30));
    assert_eq!(client.call_timeout(&json!({"timeout_ms": 60_000})), Duration::from_secs(70));
    let short = client.with_timeout(Duration::from_millis(200));
    assert_eq!(short.call_timeout(&json!({"pane_id": "w1:p1"})), Duration::from_millis(200));
    assert_eq!(short.call_timeout(&json!({"timeout_ms": 500})), Duration::from_millis(10_500));
}

#[test]
fn agent_info_reports_the_detected_cli_and_its_readiness() {
    let info = |extra: serde_json::Value| {
        let mut v = json!({"pane_id":"w1:p2","tab_id":"w1:t2","workspace_id":"w1","terminal_id":"t2",
                           "agent_status":"idle","focused":false,"revision":3});
        v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        serde_json::from_value::<AgentInfo>(v).unwrap()
    };
    let ready = info(json!({"agent":"claude","interactive_ready":true}));
    assert_eq!((ready.agent.as_deref(), ready.interactive_ready), (Some("claude"), Some(true)));
    let launching = info(json!({"launch_pending":true}));
    assert_eq!((launching.agent, launching.interactive_ready), (None, None));
    assert_eq!(info(json!({"agent":null,"interactive_ready":true})).agent, None);
}

#[test]
fn unknown_agent_status_strings_map_to_unknown() {
    let status = |s: &str| serde_json::from_value::<AgentStatus>(json!(s)).unwrap();
    assert_eq!(status("working"), AgentStatus::Working);
    assert_eq!(status("done"), AgentStatus::Done);
    assert_eq!(status("unknown"), AgentStatus::Unknown);
    assert_eq!(status("compacting"), AgentStatus::Unknown);
}

#[tokio::test]
async fn missing_socket_is_io_error() {
    let err = HerdrClient::new("/nonexistent/herdr.sock").ping().await.unwrap_err();
    assert!(matches!(err, HerdrError::Io(_)), "got {err:?}");
}
