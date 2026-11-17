mod support;

use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};

use serde_json::Value;
use support::*;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream as TokioUnixStream;

async fn roundtrip(socket: &std::path::Path, line: &str) -> Value {
    let mut stream = TokioUnixStream::connect(socket).await.unwrap();
    stream.write_all(format!("{line}\n").as_bytes()).await.unwrap();
    let mut out = String::new();
    BufReader::new(stream).read_line(&mut out).await.unwrap();
    serde_json::from_str(&out).unwrap()
}

#[tokio::test]
async fn serves_replies_and_rejects_garbage() {
    let h = harness();
    let socket = h.paths.bridge_socket();

    {
        let _listener = UnixListener::bind(&socket).unwrap();
    }

    sonata_core::bridge::serve(&socket, h.engine.clone()).unwrap();
    assert_eq!(std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777, 0o600);

    let parent = socket.parent().unwrap();
    assert_eq!(std::fs::metadata(parent).unwrap().permissions().mode() & 0o777, 0o700);

    let (bot, _) = bot_with_delivered(&h, "hello").await;
    let ok = roundtrip(&socket, r#"{"v":1,"pane_id":"w1:p1","cmd":"reply","body":"hi"}"#).await;
    assert_eq!(ok["ok"], true);
    assert!(h.engine.user_thread(&bot.id).unwrap().iter().any(|m| m.body == "hi"));

    let bad = roundtrip(&socket, "not json").await;
    assert_eq!(bad["ok"], false);
    assert_eq!(bad["error"]["code"], "bad_request");
}

#[tokio::test]
async fn rejects_live_socket() {
    let h = harness();
    let socket = h.paths.bridge_socket();

    let _live_listener = UnixListener::bind(&socket).unwrap();

    let err = sonata_core::bridge::serve(&socket, h.engine.clone()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
    assert!(err.to_string().contains("another Sonata instance"));

    let _ = UnixStream::connect(&socket).expect("live listener should still be accessible");
}

#[tokio::test]
async fn rejects_regular_file() {
    let h = harness();
    let socket = h.paths.bridge_socket();

    std::fs::write(&socket, "regular file").unwrap();
    let file_content = std::fs::read_to_string(&socket).unwrap();

    let err = sonata_core::bridge::serve(&socket, h.engine.clone()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert!(err.to_string().contains("is not a socket"));

    let file_content_after = std::fs::read_to_string(&socket).unwrap();
    assert_eq!(file_content, file_content_after);
}
