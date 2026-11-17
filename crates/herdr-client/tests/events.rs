mod support;

use herdr_client::*;
use serde_json::json;
use support::FakeHerdr;

#[tokio::test]
async fn subscribe_streams_status_and_exit_events() {
    let fake = FakeHerdr::start(|_| {
        vec![
            json!({"result":{"type":"subscription_started"}}),
            json!({"event":"pane.agent_status_changed","data":{
                   "pane_id":"w1:p2","workspace_id":"w1","agent_status":"working","agent":"claude"}}),
            json!({"event":"pane_scroll_changed","data":{"type":"pane_scroll_changed","pane_id":"w1:p2"}}),
            json!({"event":"pane_exited","data":{"type":"pane_exited","pane_id":"w1:p2","workspace_id":"w1"}}),
        ]
    });
    let client = HerdrClient::new(&fake.socket);
    let mut events =
        client.subscribe(&[Subscription::agent_status("w1:p2"), Subscription::pane_exited()]).await.unwrap();
    assert_eq!(
        events.next().await.unwrap(),
        Some(Event::PaneAgentStatusChanged { pane_id: "w1:p2".into(), agent_status: AgentStatus::Working })
    );
    assert_eq!(events.next().await.unwrap(), Some(Event::Other));
    assert_eq!(events.next().await.unwrap(), Some(Event::PaneExited { pane_id: "w1:p2".into() }));
    assert_eq!(events.next().await.unwrap(), None);
    assert_eq!(
        fake.requests.lock().unwrap()[0]["params"],
        json!({"subscriptions":[{"type":"pane.agent_status_changed","pane_id":"w1:p2"},{"type":"pane.exited"}]})
    );
}

#[tokio::test]
async fn subscribe_error_is_returned() {
    let fake = FakeHerdr::start(|_| vec![json!({"error":{"code":"invalid_params","message":"bad"}})]);
    let err = HerdrClient::new(&fake.socket)
        .subscribe(&[Subscription::pane_exited()])
        .await
        .err()
        .expect("subscribe should fail");
    assert_eq!(err.code(), Some("invalid_params"));
}

#[tokio::test]
async fn tolerates_dotted_exit_missing_data_and_malformed_lines() {
    let fake = FakeHerdr::start(|_| {
        vec![
            json!({"result":{"type":"subscription_started"}}),
            json!({"event":"pane.exited","data":{"pane_id":"w1:p2","workspace_id":"w1"}}),
            json!({"event":"pane.something"}),
            json!({"event":"pane.agent_status_changed","data":{"agent":"claude"}}),
            json!({"event":"pane.agent_status_changed","data":{"pane_id":"w1:p2","agent_status":"idle"}}),
        ]
    });
    let client = HerdrClient::new(&fake.socket);
    let mut events = client.subscribe(&[Subscription::pane_exited()]).await.unwrap();
    assert_eq!(events.next().await.unwrap(), Some(Event::PaneExited { pane_id: "w1:p2".into() }));
    assert_eq!(events.next().await.unwrap(), Some(Event::Other));
    assert!(matches!(events.next().await, Err(HerdrError::Protocol(_))));
    assert_eq!(
        events.next().await.unwrap(),
        Some(Event::PaneAgentStatusChanged { pane_id: "w1:p2".into(), agent_status: AgentStatus::Idle })
    );
    assert_eq!(events.next().await.unwrap(), None);
}
