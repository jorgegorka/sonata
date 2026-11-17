use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::UnixStream;

use crate::client::timed_out;
use crate::protocol::{AgentStatus, ResponseLine};
use crate::{HerdrClient, HerdrError};

#[derive(Debug, Clone, PartialEq)]
pub struct Subscription(Value);

impl Subscription {
    pub fn agent_status(pane_id: &str) -> Self {
        Self(json!({"type": "pane.agent_status_changed", "pane_id": pane_id}))
    }

    pub fn pane_exited() -> Self {
        Self(json!({"type": "pane.exited"}))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    PaneAgentStatusChanged { pane_id: String, agent_status: AgentStatus },
    PaneExited { pane_id: String },
    Other,
}

pub struct EventStream {
    lines: Lines<BufReader<UnixStream>>,
}

impl EventStream {
    pub async fn next(&mut self) -> Result<Option<Event>, HerdrError> {
        #[derive(Deserialize)]
        struct EventLine {
            #[serde(default)]
            event: String,
            #[serde(default)]
            data: Value,
        }
        #[derive(Deserialize)]
        struct StatusData {
            pane_id: String,
            agent_status: AgentStatus,
        }
        #[derive(Deserialize)]
        struct PaneData {
            pane_id: String,
        }
        loop {
            let Some(line) = self.lines.next_line().await? else {
                return Ok(None);
            };
            if line.trim().is_empty() {
                continue;
            }
            let parsed: EventLine =
                serde_json::from_str(&line).map_err(|e| HerdrError::Protocol(format!("event: {e}")))?;
            let data_type = parsed.data.get("type").and_then(Value::as_str).unwrap_or("");
            let name = if parsed.event.is_empty() { data_type } else { parsed.event.as_str() };
            let event = match name.replace('.', "_").as_str() {
                "pane_agent_status_changed" => {
                    let d: StatusData =
                        serde_json::from_value(parsed.data).map_err(|e| HerdrError::Protocol(format!("event: {e}")))?;
                    Event::PaneAgentStatusChanged { pane_id: d.pane_id, agent_status: d.agent_status }
                }
                "pane_exited" => {
                    let d: PaneData =
                        serde_json::from_value(parsed.data).map_err(|e| HerdrError::Protocol(format!("event: {e}")))?;
                    Event::PaneExited { pane_id: d.pane_id }
                }
                _ => Event::Other,
            };
            return Ok(Some(event));
        }
    }
}

impl HerdrClient {
    pub async fn subscribe(&self, subscriptions: &[Subscription]) -> Result<EventStream, HerdrError> {
        let limit = self.timeout();
        tokio::time::timeout(limit, self.subscribe_once(subscriptions))
            .await
            .unwrap_or_else(|_| Err(timed_out("events.subscribe", limit)))
    }

    async fn subscribe_once(&self, subscriptions: &[Subscription]) -> Result<EventStream, HerdrError> {
        let mut stream = UnixStream::connect(self.socket_path()).await?;
        let id = self.next_id();
        let subs: Vec<&Value> = subscriptions.iter().map(|s| &s.0).collect();
        stream.write_all(&Self::request_line(&id, "events.subscribe", &json!({"subscriptions": subs}))?).await?;
        let mut lines = BufReader::new(stream).lines();
        let first = lines
            .next_line()
            .await?
            .ok_or_else(|| HerdrError::Protocol("events.subscribe: connection closed".into()))?;
        let resp: ResponseLine =
            serde_json::from_str(&first).map_err(|e| HerdrError::Protocol(format!("events.subscribe: {e}")))?;
        if let Some(e) = resp.error {
            return Err(HerdrError::Server { code: e.code, message: e.message });
        }
        Ok(EventStream { lines })
    }
}
