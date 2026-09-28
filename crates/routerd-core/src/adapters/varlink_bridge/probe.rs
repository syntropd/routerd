use super::VarlinkBridgeAdapter;
use crate::error::Result;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::timeout;

impl VarlinkBridgeAdapter {
    pub(super) async fn check_health(&self) -> Result<bool> {
        if !self.socket_path.exists() {
            return Ok(false);
        }

        let res = timeout(
            Duration::from_millis(1000),
            UnixStream::connect(&self.socket_path),
        )
        .await;

        match res {
            Ok(Ok(mut stream)) => {
                let req = json!({
                    "method": "org.varlink.service.GetInfo",
                    "parameters": {}
                });
                let mut b = serde_json::to_vec(&req)?;
                b.push(0);
                let _ = stream.write_all(&b).await;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub(super) async fn available_models(&self) -> Result<Vec<String>> {
        if !self.socket_path.exists() {
            return Ok(self.configured_models.clone());
        }

        let stream_res = timeout(
            Duration::from_millis(1500),
            UnixStream::connect(&self.socket_path),
        )
        .await;

        let mut stream = match stream_res {
            Ok(Ok(s)) => s,
            _ => return Ok(self.configured_models.clone()),
        };

        let req = json!({
            "method": "io.syntrop.Inference1.ListModels",
            "parameters": {}
        });
        let mut b = serde_json::to_vec(&req)?;
        b.push(0);

        if stream.write_all(&b).await.is_err() {
            return Ok(self.configured_models.clone());
        }

        let mut buf = Vec::with_capacity(512);
        let mut byte = [0u8; 1];

        let read_res = timeout(Duration::from_millis(1500), async {
            loop {
                let n = stream.read(&mut byte).await?;
                if n == 0 || byte[0] == 0 {
                    break;
                }
                buf.push(byte[0]);
            }
            Ok::<(), std::io::Error>(())
        })
        .await;

        if read_res.is_err() || buf.is_empty() {
            return Ok(self.configured_models.clone());
        }

        let val: Value = serde_json::from_slice(&buf).unwrap_or(Value::Null);
        let mut models = Vec::new();
        if let Some(list) = val
            .get("parameters")
            .and_then(|p| p.get("models"))
            .and_then(|m| m.as_array())
        {
            for item in list {
                if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    models.push(id.to_string());
                } else if let Some(s) = item.as_str() {
                    models.push(s.to_string());
                }
            }
        }

        if models.is_empty() {
            Ok(self.configured_models.clone())
        } else {
            Ok(models)
        }
    }
}
