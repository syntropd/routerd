pub mod protocol;
pub mod router1;
pub mod service;

use protocol::{make_method_not_found, parse_request};
use routerd_core::RouterEngine;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, info, warn};

#[allow(dead_code)]
pub const DEFAULT_VARLINK_PATH: &str = "/run/syntrop/io.syntrop.Router1";

pub fn bind_or_create_listener(path: impl AsRef<Path>) -> std::io::Result<UnixListener> {
    let p = path.as_ref();
    if p.exists() {
        let _ = std::fs::remove_file(p);
    }
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    UnixListener::bind(p)
}

pub async fn run_varlink_listener(listener: UnixListener, engine: Arc<RouterEngine>) {
    info!("Varlink IPC server listening for connections");

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let eng = engine.clone();
                tokio::spawn(async move {
                    handle_varlink_connection(stream, eng).await;
                });
            }
            Err(e) => {
                warn!("Varlink accept error: {}", e);
            }
        }
    }
}

async fn handle_varlink_connection(stream: UnixStream, engine: Arc<RouterEngine>) {
    let (mut reader, mut writer) = stream.into_split();
    let mut buf = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];

    loop {
        buf.clear();
        loop {
            match reader.read(&mut byte).await {
                Ok(0) => return, // Connection closed
                Ok(_) => {
                    if byte[0] == 0 {
                        break;
                    }
                    buf.push(byte[0]);
                }
                Err(e) => {
                    debug!("Varlink stream read closed/error: {}", e);
                    return;
                }
            }
        }

        if buf.is_empty() {
            continue;
        }

        let request = match parse_request(&buf) {
            Ok(req) => req,
            Err(e) => {
                let err_reply = protocol::VarlinkReply::error(
                    "org.varlink.service.InvalidParameter",
                    serde_json::json!({ "error": e }),
                );
                let _ = writer.write_all(&err_reply.to_bytes()).await;
                continue;
            }
        };

        debug!("Varlink call received: {}", request.method);

        let reply = match request.method.as_str() {
            "org.varlink.service.GetInfo" => Some(service::handle_get_info()),
            "org.varlink.service.GetInterfaceDescription" => {
                Some(service::handle_get_interface_description(request.parameters.as_ref()))
            }
            m if m.starts_with("io.syntrop.Router1.") => {
                router1::handle_method(m, request.parameters.as_ref(), &engine).await
            }
            _ => None,
        };

        let response_bytes = match reply {
            Some(rep) => rep.to_bytes(),
            None => make_method_not_found(&request.method),
        };

        if let Err(e) = writer.write_all(&response_bytes).await {
            warn!("Failed to write Varlink reply: {}", e);
            break;
        }
        let _ = writer.flush().await;

        if request.oneway.unwrap_or(false) {
            break;
        }
    }
}
