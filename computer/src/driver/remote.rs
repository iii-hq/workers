//! The remote driver: an executor running inside the desktop guest, reached
//! over a socket. It speaks a tiny request/response protocol: each call is a
//! JSON frame `{"command": <name>, "params": {...}}` and the reply is
//! `{"success": <bool>, ...}`. We use the WebSocket channel (`/ws`), which is
//! strictly one-reply-per-command, so the socket is held behind a mutex and
//! each command serializes send-then-recv under it.
//!
//! Transport failures (send error, server close, transport error, timeout)
//! drop the socket so every later call fails fast with a clear message rather
//! than hanging; logical failures (`success: false`) surface as errors but
//! keep the connection.

use std::time::Duration;

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use super::{Driver, Screen, Shot};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct RemoteClient {
    endpoint: String,
    socket: Mutex<Option<Socket>>,
    command_timeout: Duration,
}

impl RemoteClient {
    /// Connect to a guest executor endpoint. `endpoint` may be a full ws/wss
    /// url, an http/https url (mapped to ws/wss), or a bare `host:port` (ws
    /// assumed). A `/ws` path is appended unless already present.
    pub async fn connect(
        endpoint: &str,
        connect_timeout_ms: u64,
        command_timeout_ms: u64,
    ) -> Result<Self, String> {
        let url = ws_url(endpoint)?;
        let (socket, _resp) = timeout(
            Duration::from_millis(connect_timeout_ms),
            connect_async(&url),
        )
        .await
        .map_err(|_| format!("timed out connecting to {url} after {connect_timeout_ms}ms"))?
        .map_err(|e| format!("failed to connect to {url}: {e}"))?;
        Ok(Self {
            endpoint: url,
            socket: Mutex::new(Some(socket)),
            command_timeout: Duration::from_millis(command_timeout_ms),
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Send one command, read its reply. Serializes on the socket mutex; skips
    /// server-initiated ping/pong; distinguishes transport failures (drop the
    /// socket) from logical `success:false` failures (keep it).
    async fn command(&self, command: &str, params: Value) -> Result<Value, String> {
        let mut guard = self.socket.lock().await;
        let socket = guard
            .as_mut()
            .ok_or_else(|| format!("{command}: driver connection is closed"))?;
        let frame = json!({ "command": command, "params": params }).to_string();

        let outcome: Result<Value, CommandError> = timeout(self.command_timeout, async {
            socket
                .send(Message::text(frame))
                .await
                .map_err(|e| CommandError::Transport(format!("{command}: send failed: {e}")))?;
            loop {
                match socket.next().await {
                    Some(Ok(Message::Text(t))) => {
                        return parse_reply(command, &t).map_err(CommandError::Logical)
                    }
                    Some(Ok(Message::Binary(b))) => {
                        return parse_reply(command, &String::from_utf8_lossy(&b))
                            .map_err(CommandError::Logical)
                    }
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
                    Some(Ok(Message::Close(_))) => {
                        return Err(CommandError::Transport(format!(
                            "{command}: server closed the connection"
                        )))
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => {
                        return Err(CommandError::Transport(format!(
                            "{command}: transport error: {e}"
                        )))
                    }
                    None => {
                        return Err(CommandError::Transport(format!(
                            "{command}: connection ended"
                        )))
                    }
                }
            }
        })
        .await
        .unwrap_or_else(|_| {
            Err(CommandError::Transport(format!(
                "{command}: timed out after {}ms",
                self.command_timeout.as_millis()
            )))
        });

        match outcome {
            Ok(v) => Ok(v),
            // A transport failure may leave the socket desynced; drop it so the
            // next call fails fast instead of hanging on a stale connection.
            Err(CommandError::Transport(msg)) => {
                *guard = None;
                Err(msg)
            }
            Err(CommandError::Logical(msg)) => Err(msg),
        }
    }

    /// A command whose reply carries nothing beyond success (pointer, keyboard,
    /// and file-write actions). Collapses the `.await?; Ok(())` boilerplate.
    async fn command_ok(&self, command: &str, params: Value) -> Result<(), String> {
        self.command(command, params).await.map(|_| ())
    }
}

/// Whether a failed command left the socket usable. Transport failures drop the
/// connection; logical `success:false` failures keep it.
enum CommandError {
    Transport(String),
    Logical(String),
}

/// Parse an executor reply. `success:true` returns the whole object (so
/// callers can read `image_data`, `stdout`, etc.); `success:false` maps to a
/// readable error, preferring an `error` string and falling back to the
/// `return_code`/`stderr` composite the wire uses for shell failures.
fn parse_reply(command: &str, text: &str) -> Result<Value, String> {
    let v: Value =
        serde_json::from_str(text).map_err(|e| format!("{command}: invalid reply json: {e}"))?;
    if v.get("success").and_then(Value::as_bool).unwrap_or(false) {
        return Ok(v);
    }
    if let Some(err) = v.get("error").and_then(Value::as_str) {
        return Err(format!("{command}: {err}"));
    }
    if let Some(rc) = v.get("return_code").or_else(|| v.get("returncode")) {
        let stderr = v.get("stderr").and_then(Value::as_str).unwrap_or("");
        return Err(format!("{command}: failed (return_code={rc}) {stderr}"));
    }
    Err(format!("{command}: driver returned success=false"))
}

#[async_trait]
impl Driver for RemoteClient {
    async fn screen_size(&self) -> Result<Screen, String> {
        let v = self.command("get_screen_size", json!({})).await?;
        // Some builds nest under `size`, others return width/height flat.
        let obj = v.get("size").unwrap_or(&v);
        let width = obj
            .get("width")
            .and_then(Value::as_u64)
            .ok_or("get_screen_size: reply missing width")?;
        let height = obj
            .get("height")
            .and_then(Value::as_u64)
            .ok_or("get_screen_size: reply missing height")?;
        Ok(Screen {
            width: width as u32,
            height: height as u32,
        })
    }

    async fn screenshot(&self) -> Result<Shot, String> {
        let v = self.command("screenshot", json!({})).await?;
        let b64 = v
            .get("image_data")
            .and_then(Value::as_str)
            .ok_or("screenshot: reply missing image_data")?;
        let bytes = STANDARD
            .decode(b64)
            .map_err(|e| format!("screenshot: invalid base64: {e}"))?;
        if bytes.is_empty() {
            return Err("screenshot: driver returned an empty image".to_string());
        }
        Ok(Shot::new(bytes))
    }

    async fn left_click(&self, x: i64, y: i64) -> Result<(), String> {
        self.command_ok("left_click", json!({ "x": x, "y": y }))
            .await
    }

    async fn right_click(&self, x: i64, y: i64) -> Result<(), String> {
        self.command_ok("right_click", json!({ "x": x, "y": y }))
            .await
    }

    async fn double_click(&self, x: i64, y: i64) -> Result<(), String> {
        self.command_ok("double_click", json!({ "x": x, "y": y }))
            .await
    }

    async fn move_cursor(&self, x: i64, y: i64) -> Result<(), String> {
        self.command_ok("move_cursor", json!({ "x": x, "y": y }))
            .await
    }

    async fn scroll(&self, x: i64, y: i64, scroll_x: i64, scroll_y: i64) -> Result<(), String> {
        self.command_ok(
            "scroll",
            json!({ "x": x, "y": y, "scroll_x": scroll_x, "scroll_y": scroll_y }),
        )
        .await
    }

    async fn drag(&self, from: (i64, i64), to: (i64, i64), button: &str) -> Result<(), String> {
        self.command_ok(
            "drag",
            json!({ "path": [[from.0, from.1], [to.0, to.1]], "button": button }),
        )
        .await
    }

    async fn type_text(&self, text: &str) -> Result<(), String> {
        self.command_ok("type_text", json!({ "text": text })).await
    }

    async fn keypress(&self, keys: &[String]) -> Result<(), String> {
        self.command_ok("hotkey", json!({ "keys": keys })).await
    }

    async fn accessibility_tree(&self) -> Result<serde_json::Value, String> {
        let v = self.command("get_accessibility_tree", json!({})).await?;
        Ok(v.get("tree").cloned().unwrap_or(v))
    }

    async fn close(&self) -> Result<(), String> {
        let mut guard = self.socket.lock().await;
        if let Some(mut socket) = guard.take() {
            let _ = socket.close(None).await;
        }
        Ok(())
    }
}

/// Normalize a caller endpoint into a `ws://.../ws` (or `wss`) url.
fn ws_url(endpoint: &str) -> Result<String, String> {
    let e = endpoint.trim();
    if e.is_empty() {
        return Err(
            "endpoint is empty: pass one to sessions::start or set default_endpoint".to_string(),
        );
    }
    let base = if let Some(rest) = e.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = e.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if e.starts_with("ws://") || e.starts_with("wss://") {
        e.to_string()
    } else {
        format!("ws://{e}")
    };
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/ws") {
        Ok(trimmed.to_string())
    } else {
        Ok(format!("{trimmed}/ws"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_url_normalizes_schemes_and_path() {
        assert_eq!(ws_url("127.0.0.1:8000").unwrap(), "ws://127.0.0.1:8000/ws");
        assert_eq!(ws_url("http://host:8000").unwrap(), "ws://host:8000/ws");
        assert_eq!(ws_url("https://host:8443").unwrap(), "wss://host:8443/ws");
        assert_eq!(ws_url("ws://host:8000/ws").unwrap(), "ws://host:8000/ws");
        assert_eq!(ws_url("ws://host:8000/ws/").unwrap(), "ws://host:8000/ws");
        assert_eq!(ws_url("wss://host/ws").unwrap(), "wss://host/ws");
        assert!(ws_url("   ").is_err());
    }

    #[test]
    fn parse_reply_ok_returns_object() {
        let v = parse_reply("screenshot", r#"{"success":true,"image_data":"abc"}"#).unwrap();
        assert_eq!(v["image_data"], "abc");
    }

    #[test]
    fn parse_reply_surfaces_error_string() {
        let err =
            parse_reply("left_click", r#"{"success":false,"error":"no display"}"#).unwrap_err();
        assert!(err.contains("no display"), "{err}");
    }

    #[test]
    fn parse_reply_surfaces_return_code_composite() {
        let err = parse_reply(
            "run_command",
            r#"{"success":false,"return_code":2,"stderr":"boom"}"#,
        )
        .unwrap_err();
        assert!(err.contains("return_code=2"), "{err}");
        assert!(err.contains("boom"), "{err}");
    }

    #[test]
    fn parse_reply_rejects_invalid_json() {
        assert!(parse_reply("x", "not json").is_err());
    }
}
