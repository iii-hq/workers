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

                // Hybrid dispatch:
                // - state-machine ops (initialize, session/new, ...) run
                //   serially so handshake ordering and session-existence
                //   invariants hold.
                // - session/prompt is long-running (LLM turn). Spawn it so
                //   stdin keeps reading — otherwise session/cancel can't
                //   reach the handler until the prompt finishes.
                // The Outbound writer is locked, so concurrent writes still
                // produce intact frames.
                let method = body.get("method").and_then(|v| v.as_str()).unwrap_or("");
                if method == "session/prompt" {
                    let h = handler.clone();
                    tokio::spawn(async move {
                        if let Some(reply) = h.handle(body).await {
                            if let Err(e) = h.outbound().write(&reply).await {
                                tracing::error!(error = %e, "stdout write failed");
                            }
                        }
                    });
                } else if let Some(reply) = handler.handle(body).await {
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
