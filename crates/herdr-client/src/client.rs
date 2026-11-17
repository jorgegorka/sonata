use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::protocol::*;

#[derive(Debug, thiserror::Error)]
pub enum HerdrError {
    #[error("cannot reach herdr: {0}")]
    Io(#[from] std::io::Error),
    #[error("herdr error {code}: {message}")]
    Server { code: String, message: String },
    #[error("unexpected herdr response: {0}")]
    Protocol(String),
}

impl HerdrError {
    pub fn code(&self) -> Option<&str> {
        match self {
            HerdrError::Server { code, .. } => Some(code),
            _ => None,
        }
    }
}

pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(30);
const SERVER_TIMEOUT_MARGIN: Duration = Duration::from_secs(10);

pub(crate) fn timed_out(what: &str, limit: Duration) -> HerdrError {
    HerdrError::Io(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        format!("{what}: herdr did not answer within {limit:?}"),
    ))
}

#[derive(Debug, Clone)]
pub struct HerdrClient {
    socket: PathBuf,
    next_id: Arc<AtomicU64>,
    timeout: Duration,
}

impl HerdrClient {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self { socket: socket.into(), next_id: Arc::new(AtomicU64::new(1)), timeout: DEFAULT_CALL_TIMEOUT }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn call_timeout(&self, params: &Value) -> Duration {
        match params.get("timeout_ms").and_then(Value::as_u64) {
            Some(ms) => Duration::from_millis(ms) + SERVER_TIMEOUT_MARGIN,
            None => self.timeout,
        }
    }

    pub(crate) fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    pub(crate) fn next_id(&self) -> String {
        format!("req_{}", self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    pub(crate) fn request_line(id: &str, method: &str, params: &Value) -> Result<Vec<u8>, HerdrError> {
        let mut line = serde_json::to_vec(&json!({"id": id, "method": method, "params": params}))
            .map_err(|e| HerdrError::Protocol(e.to_string()))?;
        line.push(b'\n');
        Ok(line)
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, HerdrError> {
        let limit = self.call_timeout(&params);
        tokio::time::timeout(limit, self.call_once(method, params))
            .await
            .unwrap_or_else(|_| Err(timed_out(method, limit)))
    }

    async fn call_once(&self, method: &str, params: Value) -> Result<Value, HerdrError> {
        let mut stream = UnixStream::connect(&self.socket).await?;
        let id = self.next_id();
        stream.write_all(&Self::request_line(&id, method, &params)?).await?;
        let mut reader = BufReader::new(stream);
        let mut buf = String::new();
        if reader.read_line(&mut buf).await? == 0 {
            return Err(HerdrError::Protocol(format!("{method}: connection closed without a response")));
        }
        let resp: ResponseLine =
            serde_json::from_str(&buf).map_err(|e| HerdrError::Protocol(format!("{method}: {e}")))?;
        if resp.id != id {
            return Err(HerdrError::Protocol(format!("{method}: response id {} != {id}", resp.id)));
        }
        match (resp.result, resp.error) {
            (_, Some(e)) => Err(HerdrError::Server { code: e.code, message: e.message }),
            (Some(r), None) => Ok(r),
            (None, None) => Err(HerdrError::Protocol(format!("{method}: empty response"))),
        }
    }

    async fn call_as<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, HerdrError> {
        let v = self.call(method, params).await?;
        serde_json::from_value(v).map_err(|e| HerdrError::Protocol(format!("{method}: {e}")))
    }

    pub async fn ping(&self) -> Result<Pong, HerdrError> {
        self.call_as("ping", json!({})).await
    }

    pub async fn workspace_create(&self, label: &str, cwd: &Path) -> Result<Created, HerdrError> {
        self.call_as("workspace.create", json!({"label": label, "cwd": cwd, "focus": false})).await
    }

    pub async fn workspace_list(&self) -> Result<Vec<WorkspaceInfo>, HerdrError> {
        #[derive(Deserialize)]
        struct List {
            workspaces: Vec<WorkspaceInfo>,
        }
        Ok(self.call_as::<List>("workspace.list", json!({})).await?.workspaces)
    }

    pub async fn tab_create(
        &self,
        workspace_id: &str,
        label: &str,
        cwd: &Path,
        env: &BTreeMap<String, String>,
    ) -> Result<Created, HerdrError> {
        self.call_as(
            "tab.create",
            json!({"workspace_id": workspace_id, "label": label, "cwd": cwd, "env": env, "focus": false}),
        )
        .await
    }

    pub async fn tab_close(&self, tab_id: &str) -> Result<(), HerdrError> {
        self.call("tab.close", json!({"tab_id": tab_id})).await.map(|_| ())
    }

    pub async fn pane_send_input(&self, pane_id: &str, text: &str, keys: &[&str]) -> Result<(), HerdrError> {
        self.call("pane.send_input", json!({"pane_id": pane_id, "text": text, "keys": keys})).await.map(|_| ())
    }

    pub async fn pane_wait_for_substring(
        &self,
        pane_id: &str,
        needle: &str,
        timeout_ms: u64,
    ) -> Result<(), HerdrError> {
        self.call(
            "pane.wait_for_output",
            json!({"pane_id": pane_id, "source": ReadSource::Visible,
                   "match": {"type": "substring", "value": needle}, "timeout_ms": timeout_ms}),
        )
        .await
        .map(|_| ())
    }

    pub async fn pane_read(&self, pane_id: &str, source: ReadSource, lines: u32) -> Result<String, HerdrError> {
        let r: ReadEnvelope =
            self.call_as("pane.read", json!({"pane_id": pane_id, "source": source, "lines": lines})).await?;
        Ok(r.read.text)
    }

    pub async fn pane_read_as(
        &self,
        pane_id: &str,
        source: ReadSource,
        format: ReadFormat,
        lines: u32,
    ) -> Result<String, HerdrError> {
        let params = json!({"pane_id": pane_id, "source": source, "format": format, "lines": lines});
        let r: ReadEnvelope = self.call_as("pane.read", params).await?;
        Ok(r.read.text)
    }

    pub async fn agent_start(
        &self,
        name: &str,
        kind: &str,
        pane_id: &str,
        args: &[String],
        timeout_ms: u64,
    ) -> Result<AgentInfo, HerdrError> {
        #[derive(Deserialize)]
        struct Started {
            agent: AgentInfo,
        }
        let s: Started = self
            .call_as(
                "agent.start",
                json!({"name": name, "kind": kind, "pane_id": pane_id, "args": args, "timeout_ms": timeout_ms}),
            )
            .await?;
        Ok(s.agent)
    }

    pub async fn agent_prompt(&self, target: &str, text: &str) -> Result<(), HerdrError> {
        self.call("agent.prompt", json!({"target": target, "text": text})).await.map(|_| ())
    }

    pub async fn agent_get(&self, target: &str) -> Result<AgentInfo, HerdrError> {
        #[derive(Deserialize)]
        struct Got {
            agent: AgentInfo,
        }
        Ok(self.call_as::<Got>("agent.get", json!({"target": target})).await?.agent)
    }

    pub async fn agent_read(&self, target: &str, source: ReadSource, lines: u32) -> Result<String, HerdrError> {
        let r: ReadEnvelope =
            self.call_as("agent.read", json!({"target": target, "source": source, "lines": lines})).await?;
        Ok(r.read.text)
    }

    pub async fn agent_read_as(
        &self,
        target: &str,
        source: ReadSource,
        format: ReadFormat,
        lines: u32,
    ) -> Result<String, HerdrError> {
        let params = json!({"target": target, "source": source, "format": format, "lines": lines});
        let r: ReadEnvelope = self.call_as("agent.read", params).await?;
        Ok(r.read.text)
    }

    pub async fn agent_send_keys(&self, target: &str, keys: &[&str]) -> Result<(), HerdrError> {
        self.call("agent.send_keys", json!({"target": target, "keys": keys})).await.map(|_| ())
    }

    pub async fn server_stop(&self) -> Result<(), HerdrError> {
        self.call("server.stop", json!({})).await.map(|_| ())
    }
}
