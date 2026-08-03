//! The iii-sandbox driver: a desktop running inside an iii-sandbox microVM,
//! driven entirely through iii primitives. No executor, no socket into the
//! guest.
//!
//! iii-sandbox boots a libkrun microVM from an OCI image and exposes it only
//! through `sandbox::exec` / `sandbox::fs` over the engine bus; there is no
//! inbound TCP into the guest. So this driver does not connect to anything.
//! It boots a sandbox, brings up a virtual display (Xvfb) with a fixed
//! resolution, and maps every desktop semantic onto a `sandbox::exec` call:
//!
//! - screenshot: `import` grabs the X root and pipes JPEG to stdout, base64'd.
//! - pointer/keyboard: `xdotool` against `DISPLAY=:0`.
//!
//! A fixed virtual resolution means 1:1 coordinates and no HiDPI or
//! multi-monitor ambiguity: the pixel the model reads is the pixel it clicks.
//! Pointer/keyboard calls run as a raw argv (no shell), so caller text and
//! coordinates are literal arguments and never shell-interpreted.

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::{json, Value};
use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;

use super::{Driver, Screen, Shot};

/// Longest a `sandbox::create` (incl. a cold OCI pull) may take before we give
/// up. The daemon recommends 300s for a cold pull.
const BOOT_TIMEOUT_MS: u64 = 300_000;
/// The virtual display the desktop renders to.
const DISPLAY: &str = ":0";
/// Most wheel notches one scroll call will send.
const MAX_WHEEL_NOTCHES: i64 = 50;

/// One live desktop inside an iii-sandbox microVM.
pub struct IiiSandboxHost {
    iii: Arc<IIIClient>,
    sandbox_id: String,
    screen: Screen,
    jpeg_quality: u8,
    command_timeout_ms: u64,
}

impl IiiSandboxHost {
    /// Boot a sandbox from `image`, bring up Xvfb at `width`x`height`, and
    /// return a ready host. On any bootstrap failure the sandbox is stopped so
    /// a failed start never leaks a live VM.
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        iii: Arc<IIIClient>,
        image: &str,
        width: u32,
        height: u32,
        jpeg_quality: u8,
        network: bool,
        idle_timeout_secs: u64,
        command_timeout_ms: u64,
    ) -> Result<Self, String> {
        let created = iii
            .trigger(TriggerRequest {
                function_id: "sandbox::create".to_string(),
                payload: json!({
                    "image": image,
                    "network": network,
                    "idle_timeout_secs": idle_timeout_secs,
                }),
                action: None,
                timeout_ms: Some(BOOT_TIMEOUT_MS),
            })
            .await
            .map_err(|e| format!("sandbox::create({image}) failed: {e}"))?;
        let sandbox_id = created
            .get("sandbox_id")
            .and_then(Value::as_str)
            .ok_or("sandbox::create reply missing sandbox_id")?
            .to_string();

        let host = Self {
            iii,
            sandbox_id,
            screen: Screen { width, height },
            jpeg_quality,
            command_timeout_ms,
        };
        if let Err(e) = host.bootstrap().await {
            // Best-effort teardown: a half-booted sandbox must not linger.
            let _ = host.stop_sandbox().await;
            return Err(e);
        }
        Ok(host)
    }

    /// Re-attach to a sandbox that outlived a worker restart (its id was
    /// persisted). Re-runs the idempotent bootstrap; if the sandbox is gone the
    /// bootstrap exec fails and the caller drops the record.
    pub async fn attach(
        iii: Arc<IIIClient>,
        sandbox_id: String,
        width: u32,
        height: u32,
        jpeg_quality: u8,
        command_timeout_ms: u64,
    ) -> Result<Self, String> {
        let host = Self {
            iii,
            sandbox_id,
            screen: Screen { width, height },
            jpeg_quality,
            command_timeout_ms,
        };
        host.bootstrap().await?;
        Ok(host)
    }

    pub fn sandbox_id(&self) -> &str {
        &self.sandbox_id
    }

    /// Start Xvfb + a window manager and block until the display answers.
    /// Detach with `setsid` (a new session, not just `nohup`): the daemon
    /// SIGKILLs the exec child's process group on host disconnect, and only a
    /// fresh session escapes that, so the display keeps running between exec
    /// calls. Idempotent: re-running against a live display is a no-op, so
    /// `attach` reuses it.
    async fn bootstrap(&self) -> Result<(), String> {
        let script = format!(
            "if ! xdpyinfo >/dev/null 2>&1; then \
                setsid Xvfb {DISPLAY} -screen 0 {w}x{h}x24 -nolisten tcp >/tmp/xvfb.log 2>&1 & \
             fi; \
             n=0; \
             while ! xdpyinfo >/dev/null 2>&1; do \
                n=$((n+1)); \
                if [ $n -gt 100 ]; then echo xvfb-timeout >&2; tail -n 20 /tmp/xvfb.log >&2; exit 1; fi; \
                sleep 0.1; \
             done; \
             if ! pgrep -x openbox >/dev/null 2>&1; then setsid openbox >/tmp/openbox.log 2>&1 & fi; \
             echo ready",
            w = self.screen.width,
            h = self.screen.height,
        );
        self.exec_sh(&script)
            .await
            .map_err(|e| format!("sandbox {} display bootstrap failed: {e}", self.sandbox_id))?;
        Ok(())
    }

    /// Run a command as a raw argv (no shell): caller text and coordinates are
    /// literal arguments, never shell-parsed. `DISPLAY` is injected so X tools
    /// target the virtual display.
    async fn exec_argv(&self, cmd: &str, args: Vec<String>) -> Result<ExecOut, String> {
        self.exec(json!({
            "sandbox_id": self.sandbox_id,
            "cmd": cmd,
            "args": args,
            "env": { "DISPLAY": DISPLAY },
            "timeout_ms": self.command_timeout_ms,
        }))
        .await
    }

    /// Run a `sh -lc` line, for the few cases that need a pipe (screenshot) or
    /// shell control flow (bootstrap). Never interpolates caller-supplied text.
    async fn exec_sh(&self, script: &str) -> Result<ExecOut, String> {
        self.exec(json!({
            "sandbox_id": self.sandbox_id,
            "cmd": "sh",
            "args": ["-lc", script],
            "env": { "DISPLAY": DISPLAY },
            "timeout_ms": self.command_timeout_ms,
        }))
        .await
    }

    async fn exec(&self, payload: Value) -> Result<ExecOut, String> {
        let v = self
            .iii
            .trigger(TriggerRequest {
                function_id: "sandbox::exec".to_string(),
                payload,
                action: None,
                // Give the bus a margin over the daemon's own exec deadline.
                timeout_ms: Some(self.command_timeout_ms + 5_000),
            })
            .await
            .map_err(|e| format!("sandbox::exec failed: {e}"))?;
        ExecOut::from_reply(&v)
    }

    async fn stop_sandbox(&self) -> Result<(), String> {
        self.iii
            .trigger(TriggerRequest {
                function_id: "sandbox::stop".to_string(),
                payload: json!({ "sandbox_id": self.sandbox_id }),
                action: None,
                timeout_ms: Some(self.command_timeout_ms),
            })
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Move the pointer, then click `button` `count` times.
    async fn click_at(&self, x: i64, y: i64, button: &str, count: u32) -> Result<(), String> {
        let mut args = vec![
            "mousemove".to_string(),
            clamp(x),
            clamp(y),
            "click".to_string(),
        ];
        if count > 1 {
            args.push("--repeat".to_string());
            args.push(count.to_string());
            args.push("--delay".to_string());
            args.push("40".to_string());
        }
        args.push(button.to_string());
        self.exec_argv("xdotool", args).await.map(|_| ())
    }

    /// Spin the wheel `button` `count` times at the current pointer position.
    /// The count is capped: a caller asking for thousands of notches would
    /// otherwise hold the exec open until it times out.
    async fn wheel(&self, x: i64, y: i64, button: &str, count: i64) -> Result<(), String> {
        if count <= 0 {
            return Ok(());
        }
        let count = count.min(MAX_WHEEL_NOTCHES);
        self.exec_argv(
            "xdotool",
            vec![
                "mousemove".to_string(),
                clamp(x),
                clamp(y),
                "click".to_string(),
                "--repeat".to_string(),
                count.to_string(),
                button.to_string(),
            ],
        )
        .await
        .map(|_| ())
    }
}

#[async_trait]
impl Driver for IiiSandboxHost {
    async fn screen_size(&self) -> Result<Screen, String> {
        // Fixed virtual resolution set at boot; no round-trip needed.
        Ok(self.screen)
    }

    async fn screenshot(&self) -> Result<Shot, String> {
        let script = format!(
            "import -window root -quality {q} jpg:- | base64 -w0",
            q = self.jpeg_quality
        );
        let out = self.exec_sh(&script).await?;
        let b64 = out.stdout.trim();
        if b64.is_empty() {
            return Err(format!(
                "screenshot: empty capture (stderr: {})",
                out.stderr.trim()
            ));
        }
        let bytes = STANDARD
            .decode(b64)
            .map_err(|e| format!("screenshot: invalid base64 from guest: {e}"))?;
        if bytes.is_empty() {
            return Err("screenshot: guest returned an empty image".to_string());
        }
        Ok(Shot::new(bytes))
    }

    async fn left_click(&self, x: i64, y: i64) -> Result<(), String> {
        self.click_at(x, y, "1", 1).await
    }

    async fn right_click(&self, x: i64, y: i64) -> Result<(), String> {
        self.click_at(x, y, "3", 1).await
    }

    async fn double_click(&self, x: i64, y: i64) -> Result<(), String> {
        self.click_at(x, y, "1", 2).await
    }

    async fn move_cursor(&self, x: i64, y: i64) -> Result<(), String> {
        self.exec_argv("xdotool", vec!["mousemove".to_string(), clamp(x), clamp(y)])
            .await
            .map(|_| ())
    }

    async fn scroll(&self, x: i64, y: i64, scroll_x: i64, scroll_y: i64) -> Result<(), String> {
        // X wheel buttons: 4 up, 5 down, 6 left, 7 right. Positive scroll_y
        // scrolls down (matches the act default), positive scroll_x right.
        if scroll_y > 0 {
            self.wheel(x, y, "5", scroll_y).await?;
        } else if scroll_y < 0 {
            self.wheel(x, y, "4", -scroll_y).await?;
        }
        if scroll_x > 0 {
            self.wheel(x, y, "7", scroll_x).await?;
        } else if scroll_x < 0 {
            self.wheel(x, y, "6", -scroll_x).await?;
        }
        Ok(())
    }

    async fn drag(&self, from: (i64, i64), to: (i64, i64), button: &str) -> Result<(), String> {
        let b = button_number(button);
        self.exec_argv(
            "xdotool",
            vec![
                "mousemove".to_string(),
                clamp(from.0),
                clamp(from.1),
                "mousedown".to_string(),
                b.to_string(),
                "mousemove".to_string(),
                clamp(to.0),
                clamp(to.1),
                "mouseup".to_string(),
                b.to_string(),
            ],
        )
        .await
        .map(|_| ())
    }

    async fn type_text(&self, text: &str) -> Result<(), String> {
        self.exec_argv(
            "xdotool",
            vec![
                "type".to_string(),
                "--clearmodifiers".to_string(),
                "--".to_string(),
                text.to_string(),
            ],
        )
        .await
        .map(|_| ())
    }

    async fn keypress(&self, keys: &[String]) -> Result<(), String> {
        let combo = keys
            .iter()
            .map(|k| map_key(k))
            .collect::<Vec<_>>()
            .join("+");
        self.exec_argv(
            "xdotool",
            vec!["key".to_string(), "--clearmodifiers".to_string(), combo],
        )
        .await
        .map(|_| ())
    }

    async fn accessibility_tree(&self) -> Result<serde_json::Value, String> {
        // A Linux X guest exposes no macOS-style AX tree.
        Ok(serde_json::Value::Null)
    }

    async fn close(&self) -> Result<(), String> {
        // Idempotent: a stop against an already-reaped sandbox is a success.
        match self.stop_sandbox().await {
            Ok(()) => Ok(()),
            Err(e) => {
                tracing::warn!(sandbox = %self.sandbox_id, error = %e, "sandbox stop failed on close; the VM may still be running");
                Ok(())
            }
        }
    }
}

/// The subset of a `sandbox::exec` reply we act on.
#[derive(Debug)]
struct ExecOut {
    stdout: String,
    stderr: String,
}

impl ExecOut {
    fn from_reply(v: &Value) -> Result<Self, String> {
        let stdout = v
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let stderr = v
            .get("stderr")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let exit_code = v.get("exit_code").and_then(Value::as_i64);
        let success = v
            .get("success")
            .and_then(Value::as_bool)
            .unwrap_or(exit_code == Some(0));
        if !success {
            let code = exit_code.unwrap_or(-1);
            let detail = if !stderr.trim().is_empty() {
                stderr.trim()
            } else {
                stdout.trim()
            };
            return Err(format!("guest command exited {code}: {detail}"));
        }
        Ok(Self { stdout, stderr })
    }
}

/// Clamp a coordinate to a non-negative integer string for xdotool.
fn clamp(v: i64) -> String {
    v.max(0).to_string()
}

/// X button number for a named mouse button.
fn button_number(button: &str) -> u8 {
    match button {
        "right" => 3,
        "middle" => 2,
        _ => 1,
    }
}

/// Map a caller key name onto the X keysym `xdotool key` expects. Unknown
/// single names pass through (a literal char or an already-valid keysym).
fn map_key(key: &str) -> String {
    match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => "Return",
        "esc" | "escape" => "Escape",
        "tab" => "Tab",
        "space" | "spacebar" => "space",
        "backspace" => "BackSpace",
        "delete" | "del" => "Delete",
        "up" | "arrowup" => "Up",
        "down" | "arrowdown" => "Down",
        "left" | "arrowleft" => "Left",
        "right" | "arrowright" => "Right",
        "home" => "Home",
        "end" => "End",
        "pageup" | "pgup" => "Prior",
        "pagedown" | "pgdn" => "Next",
        "ctrl" | "control" => "ctrl",
        "alt" | "option" => "alt",
        "shift" => "shift",
        "cmd" | "command" | "super" | "meta" | "win" => "super",
        _ => return key.to_string(),
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_floors_negatives() {
        assert_eq!(clamp(-5), "0");
        assert_eq!(clamp(0), "0");
        assert_eq!(clamp(1280), "1280");
    }

    #[test]
    fn button_number_maps_names() {
        assert_eq!(button_number("left"), 1);
        assert_eq!(button_number("middle"), 2);
        assert_eq!(button_number("right"), 3);
        assert_eq!(button_number("weird"), 1);
    }

    #[test]
    fn map_key_normalizes_common_names() {
        assert_eq!(map_key("enter"), "Return");
        assert_eq!(map_key("Escape"), "Escape");
        assert_eq!(map_key("cmd"), "super");
        assert_eq!(map_key("ctrl"), "ctrl");
        assert_eq!(map_key("a"), "a");
        assert_eq!(map_key("F5"), "F5");
    }

    #[test]
    fn exec_out_flags_nonzero_exit() {
        let ok = ExecOut::from_reply(&json!({ "stdout": "hi", "exit_code": 0, "success": true }));
        assert!(ok.is_ok());
        let bad = ExecOut::from_reply(
            &json!({ "stdout": "", "stderr": "boom", "exit_code": 2, "success": false }),
        );
        let err = bad.unwrap_err();
        assert!(err.contains("exited 2"), "{err}");
        assert!(err.contains("boom"), "{err}");
    }

    #[test]
    fn exec_out_infers_success_from_exit_code() {
        // No explicit success flag: fall back to exit_code == 0.
        let ok = ExecOut::from_reply(&json!({ "stdout": "x", "exit_code": 0 }));
        assert!(ok.is_ok());
        let bad = ExecOut::from_reply(&json!({ "stderr": "no", "exit_code": 1 }));
        assert!(bad.is_err());
    }
}
