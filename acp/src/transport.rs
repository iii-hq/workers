use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::Mutex;

use crate::handler::AcpHandler;
use crate::types::{JsonRpcResponse, PARSE_ERROR};

pub struct Outbound {
    writer: Mutex<BufWriter<tokio::io::Stdout>>,
}

impl Default for Outbound {
    fn default() -> Self {
        Self::new()
    }
}

impl Outbound {
    pub fn new() -> Self {
        Self {
            writer: Mutex::new(BufWriter::new(tokio::io::stdout())),
        }
    }

    pub async fn write(&self, value: &Value) -> anyhow::Result<()> {
        let mut w = self.writer.lock().await;
        let json = serde_json::to_string(value)?;
        w.write_all(json.as_bytes()).await?;
        w.write_all(b"\n").await?;
        w.flush().await?;
        Ok(())
    }
}

pub async fn run_stdio(handler: Arc<AcpHandler>) -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);

    tracing::info!("acp stdio transport started");

    loop {
        let mut line = String::new();
        match reader.read_line(&mut line).await {
            Ok(0) => {
                tracing::info!("stdin closed");
                break;
            }
            Ok(_) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                let body: Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(err) => {
                        tracing::warn!(error = %err, "parse error");
                        let resp = JsonRpcResponse::error(
                            None,
                            PARSE_ERROR,
                            format!("parse error: {}", err),
                        );
                        if let Ok(v) = serde_json::to_value(&resp) {
                            handler.outbound().write(&v).await?;
                        }
                        continue;
                    }
                };

                // Serial dispatch. ACP is a state machine: initialize must
                // complete before session/new, session/new before
                // session/prompt, etc. Concurrent in-flight requests on
                // independent sessions are a v1 concern. Notifications
                // stream out via the same Outbound writer regardless.
                if let Some(reply) = handler.handle(body).await {
                    if let Err(e) = handler.outbound().write(&reply).await {
                        tracing::error!(error = %e, "stdout write failed");
                    }
                }
            }
            Err(err) => {
                tracing::error!(error = %err, "stdin read error");
                break;
            }
        }
    }

    Ok(())
}
