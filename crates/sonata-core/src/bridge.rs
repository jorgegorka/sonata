use std::io;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::Path;

use bridge_proto::{BridgeRequest, BridgeResponse, ErrorCode};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

use crate::engine::Engine;

pub fn serve(socket: &Path, engine: Engine) -> std::io::Result<tokio::task::JoinHandle<()>> {
    let parent = socket
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "socket path has no parent directory"))?;
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;

    match std::fs::symlink_metadata(socket) {
        Ok(metadata) => {
            if !metadata.file_type().is_socket() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} exists and is not a socket", socket.display()),
                ));
            }

            if std::os::unix::net::UnixStream::connect(socket).is_ok() {
                return Err(io::Error::new(io::ErrorKind::AddrInUse, "another Sonata instance is already running"));
            }

            std::fs::remove_file(socket)?;
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(e);
        }
    }

    let listener = UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    Ok(tokio::spawn(async move {
        loop {
            let stream = match listener.accept().await {
                Ok((stream, _)) => stream,
                Err(e) => {
                    tracing::error!(error = %e, "bridge accept failed");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    continue;
                }
            };
            let engine = engine.clone();
            tokio::spawn(async move {
                let (reader, mut writer) = stream.into_split();
                let mut line = String::new();
                if BufReader::new(reader).read_line(&mut line).await.unwrap_or(0) == 0 {
                    return;
                }
                let response = match serde_json::from_str::<BridgeRequest>(&line) {
                    Ok(request) => engine.handle_bridge(request).await,
                    Err(e) => BridgeResponse::error(ErrorCode::BadRequest, format!("Could not read the request: {e}")),
                };
                let mut out = serde_json::to_vec(&response).unwrap_or_default();
                out.push(b'\n');
                let _ = writer.write_all(&out).await;
            });
        }
    }))
}
