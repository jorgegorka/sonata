#![allow(dead_code)]
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

pub struct FakeHerdr {
    pub socket: PathBuf,
    pub requests: Arc<Mutex<Vec<Value>>>,
    _dir: tempfile::TempDir,
}

impl FakeHerdr {
    pub fn start(respond: impl Fn(&Value) -> Vec<Value> + Send + Sync + 'static) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("h.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        let respond = Arc::new(respond);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let log = log.clone();
                let respond = respond.clone();
                tokio::spawn(async move {
                    let (r, mut w) = stream.into_split();
                    let mut reader = BufReader::new(r);
                    let mut line = String::new();
                    if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                        return;
                    }
                    let req: Value = serde_json::from_str(&line).unwrap();
                    log.lock().unwrap().push(req.clone());
                    let mut lines = respond(&req);
                    if lines.is_empty() {
                        std::future::pending::<()>().await;
                    }
                    if let Some(first) = lines.first_mut() {
                        first["id"] = req["id"].clone();
                    }
                    for l in lines {
                        let mut out = serde_json::to_vec(&l).unwrap();
                        out.push(b'\n');
                        if w.write_all(&out).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        Self { socket, requests, _dir: dir }
    }
}
