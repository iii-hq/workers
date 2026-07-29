//! Everything this worker does to a filesystem, it asks another worker to do.
//!
//! `editor` deliberately owns no jail, no roots and no denylist. `shell`
//! already has all three, and the repo consolidated its file surface into that
//! one worker precisely so there would be a single boundary. Adding a second
//! one here would recreate the problem: two configs to keep in sync, two
//! places to get an escape check wrong, and no way for an operator to say
//! "this tree is off limits" once.
//!
//! So every read, write, stat and git invocation below is a bus call into
//! `shell`, and the failures it returns are surfaced verbatim — an S215
//! jail-escape from `shell` must read as a jail-escape to the caller, not as
//! an `editor` error.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use iii_sdk::channels::{ChannelReader, StreamChannelRef};
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::Deserialize;
use serde_json::{json, Value};

/// Result of a `shell::exec` call, narrowed to what the git parsers need.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ExecOutcome {
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default)]
    pub stdout_truncated: bool,
}

/// What `shell::fs::stat` reports about a path.
#[derive(Debug, Clone, Deserialize)]
pub struct FileStat {
    pub size: u64,
    /// Last-modified time, Unix seconds. The conflict guard compares this and
    /// nothing else — content hashing every save would double the read cost
    /// for a check that only has to catch "someone else touched this".
    pub mtime: i64,
}

/// A file pulled back through the read channel, with the stat that came with it.
#[derive(Debug, Clone)]
pub struct FileRead {
    pub content: String,
    pub size: u64,
    pub mtime: i64,
    /// True when the file was larger than the caller's cap and `content` holds
    /// only its first bytes.
    pub truncated: bool,
}

/// Thin, typed front for the two workers `editor` talks to.
#[derive(Clone)]
pub struct Bus {
    iii: Arc<IIIClient>,
    /// Engine WebSocket base, needed to open the read channel `shell::fs::read`
    /// hands back. Same string the binary was started with.
    ws_url: String,
    /// Shared with the configuration reload path rather than copied: every
    /// other limit is read from the live snapshot per call, and a timeout
    /// frozen at boot would be the one field that quietly ignored an edit.
    git_timeout_ms: Arc<AtomicU64>,
}

impl Bus {
    pub fn new(iii: Arc<IIIClient>, ws_url: String, git_timeout_ms: Arc<AtomicU64>) -> Self {
        Self {
            iii,
            ws_url,
            git_timeout_ms,
        }
    }

    fn git_timeout(&self) -> u64 {
        self.git_timeout_ms.load(Ordering::Relaxed)
    }

    async fn call(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: u64,
    ) -> Result<Value, Error> {
        self.iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms: Some(timeout_ms),
            })
            .await
    }

    /// Run `git <args>` through `shell::exec`.
    ///
    /// Returns the outcome even for a non-zero exit: "not a git repository"
    /// and "unknown revision" are answers a caller wants to see, not transport
    /// failures. Only a bus-level failure is an `Err`.
    pub async fn git(&self, args: &[&str], cwd: Option<&str>) -> Result<ExecOutcome, Error> {
        let mut payload = json!({
            "command": "git",
            "args": args,
            "timeout_ms": self.git_timeout(),
        });
        if let Some(dir) = cwd {
            payload["cwd"] = json!(dir);
        }
        let value = self
            .call("shell::exec", payload, self.git_timeout() + 5_000)
            .await?;
        serde_json::from_value(value)
            .map_err(|e| Error::Handler(format!("shell::exec returned an unexpected shape: {e}")))
    }

    /// Run git and require success, folding a non-zero exit into a readable error.
    pub async fn git_ok(&self, args: &[&str], cwd: Option<&str>) -> Result<ExecOutcome, Error> {
        let out = self.git(args, cwd).await?;
        if out.timed_out {
            return Err(Error::Handler(format!(
                "git {} timed out after {}ms",
                args.join(" "),
                self.git_timeout()
            )));
        }
        if out.exit_code != Some(0) {
            let detail = if out.stderr.trim().is_empty() {
                out.stdout.trim().to_string()
            } else {
                out.stderr.trim().to_string()
            };
            return Err(Error::Handler(format!(
                "git {} failed (exit {}): {}",
                args.join(" "),
                out.exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".into()),
                detail
            )));
        }
        Ok(out)
    }

    pub async fn stat(&self, path: &str) -> Result<FileStat, Error> {
        let value = self
            .call("shell::fs::stat", json!({ "path": path }), 10_000)
            .await?;
        serde_json::from_value(value).map_err(|e| {
            Error::Handler(format!("shell::fs::stat returned an unexpected shape: {e}"))
        })
    }

    /// Read a file's text through the channel `shell::fs::read` returns.
    ///
    /// The cap is enforced while draining, not after: the reader stops pulling
    /// once it holds more than `max_bytes`, so an oversized file costs one
    /// chunk past the limit rather than its whole length in memory.
    pub async fn read(&self, path: &str, max_bytes: usize) -> Result<FileRead, Error> {
        let value = self
            .call("shell::fs::read", json!({ "path": path }), 30_000)
            .await?;

        let size = value.get("size").and_then(Value::as_u64).unwrap_or(0);
        let mtime = value.get("mtime").and_then(Value::as_i64).unwrap_or(0);
        let channel_ref: StreamChannelRef = value
            .get("content")
            .cloned()
            .ok_or_else(|| Error::Handler("shell::fs::read returned no content channel".into()))
            .and_then(|v| {
                serde_json::from_value(v).map_err(|e| {
                    Error::Handler(format!("shell::fs::read content is not a channel ref: {e}"))
                })
            })?;

        let reader = ChannelReader::new(&self.ws_url, &channel_ref);
        let mut bytes: Vec<u8> = Vec::new();
        // Drain chunk by chunk so a huge file cannot be materialised in full
        // just to be discarded. One chunk of overshoot is enough to know the
        // file exceeded the cap.
        while let Some(chunk) = reader.next_binary().await? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() > max_bytes {
                break;
            }
        }
        let _ = reader.close().await;

        let truncated = bytes.len() > max_bytes;
        let slice = if truncated {
            &bytes[..max_bytes]
        } else {
            &bytes[..]
        };

        // A truncated read can land mid-codepoint; cut back to the last valid
        // boundary instead of failing the whole open.
        let content = match std::str::from_utf8(slice) {
            Ok(s) => s.to_string(),
            Err(e) if truncated => String::from_utf8_lossy(&slice[..e.valid_up_to()]).into_owned(),
            Err(_) => {
                return Err(Error::Handler(format!(
                    "{path} is not valid UTF-8 text; editor::open reads text files only"
                )))
            }
        };

        Ok(FileRead {
            content,
            size,
            mtime,
            truncated,
        })
    }

    /// Recursive directory listing — the shell worker's own walk.
    ///
    /// Browsing is not reimplemented here: `coder::tree` already applies the
    /// noise-folder excludes, honours the jail, and bounds the traversal. This
    /// is a pass-through so the tree and the buffers come from one place.
    pub async fn tree(&self, path: &str, max_depth: u32) -> Result<Value, Error> {
        self.call(
            "coder::tree",
            json!({
                "path": path,
                "max_depth": max_depth,
                "per_folder_limit": 500,
            }),
            30_000,
        )
        .await
    }

    /// Move or rename a path. The buffer remap that must follow is the
    /// caller's job — see `workspace::Session::remap`.
    pub async fn mv(&self, src: &str, dst: &str) -> Result<(), Error> {
        self.call("shell::fs::mv", json!({ "src": src, "dst": dst }), 30_000)
            .await
            .map(|_| ())
    }

    pub async fn mkdir(&self, path: &str) -> Result<(), Error> {
        self.call(
            "shell::fs::mkdir",
            json!({ "path": path, "parents": true }),
            30_000,
        )
        .await
        .map(|_| ())
    }

    pub async fn rm(&self, path: &str, recursive: bool) -> Result<(), Error> {
        self.call(
            "shell::fs::rm",
            json!({ "path": path, "recursive": recursive }),
            30_000,
        )
        .await
        .map(|_| ())
    }

    /// Content search — shell's own recursive grep, not a second walk.
    pub async fn grep(
        &self,
        path: &str,
        pattern: &str,
        ignore_case: bool,
        include_glob: &[String],
        max_matches: u64,
    ) -> Result<Value, Error> {
        self.call(
            "shell::fs::grep",
            json!({
                "path": path,
                "pattern": pattern,
                "recursive": true,
                "ignore_case": ignore_case,
                "include_glob": include_glob,
                "max_matches": max_matches,
            }),
            60_000,
        )
        .await
    }

    /// Read one key from the `state` worker. `None` when unset.
    pub async fn state_get(&self, key: &str) -> Result<Option<Value>, Error> {
        let value = self
            .call(
                "state::get",
                json!({ "scope": crate::workspace::SCOPE, "key": key }),
                10_000,
            )
            .await?;
        Ok(if value.is_null() { None } else { Some(value) })
    }

    pub async fn state_set(&self, key: &str, value: Value) -> Result<(), Error> {
        self.call(
            "state::set",
            json!({ "scope": crate::workspace::SCOPE, "key": key, "value": value }),
            10_000,
        )
        .await
        .map(|_| ())
    }

    pub async fn write(&self, path: &str, content: &str) -> Result<(), Error> {
        self.call(
            "shell::fs::write",
            json!({ "path": path, "content": content }),
            30_000,
        )
        .await
        .map(|_| ())
    }
}
