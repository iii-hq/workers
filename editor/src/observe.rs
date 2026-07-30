//! Watching what the agent does, without the agent cooperating.
//!
//! The workspace only reflected files someone deliberately routed through
//! `editor::open`. Agents do not: they call `coder::update-file` and
//! `shell::fs::write`, because that is what their prompts and skills point at.
//! So the editor was blind to exactly the edits it exists to show.
//!
//! The fix is to observe rather than require cooperation. Every call already
//! crosses the bus, and the harness fans its calls out to bound hooks, so this
//! binds `harness::hook::post-trigger` on the filesystem-touching functions and
//! turns each one into an `editor::changed` event. `post-trigger` rather than
//! `pre-`: the write has to have happened before there is anything to report.
//!
//! Two things fall out of the hook payload for free. `metadata.fs_scope.root`
//! is the session's own workspace, so the editor can follow the agent instead
//! of needing a root set by hand. And `call.function_id` names the cause, so a
//! surface can say what did it.
//!
//! The hook is **fail-open and advisory**: it returns `Continue` unconditionally
//! and never inspects the result for a decision. Holding or denying a write
//! because a viewer was slow would be indefensible.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::bus::Bus;
use crate::config::WorkerConfig;
use crate::configuration::ConfigCell;
use crate::events::{ChangedEmitter, ChangedEvent};
use crate::{diff, lang, workspace};

pub const HOOK_FN_ID: &str = "editor::on-file-change";
/// The two families that touch files. Matching broadly and filtering on the
/// verb keeps this from breaking when either worker grows a new write path.
const HOOK_FUNCTIONS: &[&str] = &["shell::*", "coder::*"];
const HOOK_TIMEOUT_MS: u64 = 3_000;
/// A viewer must never be able to block a write.
const HOOK_ON_ERROR: &str = "fail_open";

/// The subset of the harness hook payload this worker reads.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct HookInput {
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub call: Option<HookCall>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct HookCall {
    pub function_id: String,
    #[serde(default)]
    pub arguments: Value,
}

/// Always `continue`. This hook observes; it never decides.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HookOutput {
    pub decision: &'static str,
}

impl Default for HookOutput {
    fn default() -> Self {
        Self {
            decision: "continue",
        }
    }
}

/// What a watched function did to which path.
#[derive(Debug, PartialEq, Eq)]
pub struct Touch {
    pub path: String,
    pub kind: &'static str,
}

/// Read the touched path out of a call, or `None` when the call did not write.
///
/// Deliberately a whitelist of verbs rather than "anything under shell": most
/// of that namespace reads, and reporting a read as a change would make the
/// feed useless. `shell::exec` is excluded on purpose — a command can write
/// anything and its argv does not say what, so guessing would produce phantom
/// events. Those changes still surface through git status.
pub fn touched(call: &HookCall) -> Option<Touch> {
    let kind = match call.function_id.as_str() {
        "shell::fs::write" | "coder::update-file" => "modified",
        "coder::create-file" => "created",
        "shell::fs::rm" | "coder::delete-file" => "deleted",
        "shell::fs::sed" => "modified",
        "shell::fs::mv" | "coder::move" => "moved",
        _ => return None,
    };

    // `path` covers most; `mv` uses `dst`; the batch shapes carry `files`.
    let args = &call.arguments;
    let path = args
        .get("dst")
        .and_then(Value::as_str)
        .or_else(|| args.get("path").and_then(Value::as_str))
        .map(str::to_string)
        .or_else(|| {
            args.get("files")
                .and_then(Value::as_array)
                .and_then(|f| f.first())
                .and_then(|f| f.get("path").or_else(|| f.get("dst")))
                .and_then(Value::as_str)
                .map(str::to_string)
        })?;

    Some(Touch { path, kind })
}

/// The session's workspace root, when the harness stamped one.
pub fn session_root(metadata: Option<&Value>) -> Option<String> {
    metadata?
        .get("fs_scope")?
        .get("root")?
        .as_str()
        .map(str::to_string)
}

/// Make `path` relative to `root`, leaving anything outside it alone.
pub fn relative(path: &str, root: &str) -> String {
    if root == "." || root.is_empty() {
        return path.to_string();
    }
    let trimmed = root.trim_end_matches('/');
    path.strip_prefix(trimmed)
        .map(|rest| rest.trim_start_matches('/'))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// Bind the observer. A failed bind is logged, not fatal: the harness may not
/// be installed, and an editor without a live feed is still an editor.
pub fn bind(iii: &Arc<IIIClient>, cfg: &ConfigCell, bus: &Arc<Bus>, emitter: ChangedEmitter) {
    let cfg = cfg.clone();
    let bus = bus.clone();
    iii.register_function(
        HOOK_FN_ID,
        RegisterFunction::new_async(move |input: HookInput| {
            let cfg = cfg.clone();
            let bus = bus.clone();
            let emitter = emitter.clone();
            async move {
                // Nobody watching means nothing to compute.
                if emitter.has_subscribers() {
                    let snapshot = cfg.read().await.clone();
                    report(&bus, &snapshot, &emitter, input).await;
                }
                Ok::<HookOutput, Error>(HookOutput::default())
            }
        })
        .description(
            "Internal: turns a filesystem call made by anything into an editor::changed \
             event. Observes only — always continues.",
        )
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: "harness::hook::post-trigger".to_string(),
        function_id: HOOK_FN_ID.to_string(),
        config: json!({
            "functions": HOOK_FUNCTIONS,
            "timeout_ms": HOOK_TIMEOUT_MS,
            "on_error": HOOK_ON_ERROR,
        }),
        metadata: None,
    }) {
        Ok(_) => tracing::info!(function_id = HOOK_FN_ID, "file-change observer bound"),
        Err(e) => tracing::warn!(
            error = %e,
            "failed to bind the file-change observer; the workspace will not see \
             edits made outside this worker"
        ),
    }
}

/// Build and emit the event for one observed call.
async fn report(bus: &Bus, cfg: &WorkerConfig, emitter: &ChangedEmitter, input: HookInput) {
    let Some(call) = input.call else { return };
    let Some(touch) = touched(&call) else { return };

    // Prefer the session's own workspace: it is where the agent is actually
    // working, which is not necessarily where the editor was last pointed.
    let root = match session_root(input.metadata.as_ref()) {
        Some(root) => root,
        None => bus
            .state_get(workspace::ACTIVE_ROOT_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| ".".to_string()),
    };
    let rel = relative(&touch.path, &root);

    // A deleted file has nothing to read; everything else gets a patch against
    // HEAD when the folder is a repo, and none when it is not. Either way the
    // event goes out — the notification matters more than the preview.
    let (added, removed, patch) = if touch.kind == "deleted" {
        (0, 0, String::new())
    } else {
        match bus
            .git(&["diff", "-U3", "--no-color", "--", &rel], Some(&root))
            .await
        {
            Ok(out) if out.exit_code == Some(0) && !out.stdout.trim().is_empty() => {
                let hunks = crate::git::parse_hunk_headers(&out.stdout);
                (
                    hunks.iter().map(|h| h.added).sum(),
                    hunks.iter().map(|h| h.removed).sum(),
                    out.stdout,
                )
            }
            // Not a repo, or a brand-new file git cannot diff: fall back to
            // counting the file as added so the row still says something.
            _ => match bus.read(&rel, cfg.max_file_bytes).await {
                Ok(file) => {
                    let d = diff::diff(
                        "",
                        &file.content,
                        Some(&rel),
                        cfg.diff_context_lines,
                        cfg.max_diff_bytes,
                    );
                    (d.added, d.removed, d.patch)
                }
                Err(_) => (0, 0, String::new()),
            },
        }
    };

    let _ = lang::for_path(&rel);
    emitter
        .emit(ChangedEvent {
            path: rel,
            cause: call.function_id,
            kind: touch.kind.to_string(),
            added,
            removed,
            patch,
            truncated: false,
            root,
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str, args: Value) -> HookCall {
        HookCall {
            function_id: id.to_string(),
            arguments: args,
        }
    }

    #[test]
    fn a_write_is_reported_as_modified() {
        let t = touched(&call("shell::fs::write", json!({ "path": "a.rs" }))).unwrap();
        assert_eq!(
            t,
            Touch {
                path: "a.rs".into(),
                kind: "modified"
            }
        );
    }

    #[test]
    fn a_create_is_reported_as_created() {
        let t = touched(&call("coder::create-file", json!({ "path": "new.rs" }))).unwrap();
        assert_eq!(t.kind, "created");
    }

    #[test]
    fn a_move_reports_its_destination() {
        let t = touched(&call("shell::fs::mv", json!({ "src": "a", "dst": "b" }))).unwrap();
        assert_eq!(t.path, "b", "the new location is what a surface shows");
        assert_eq!(t.kind, "moved");
    }

    #[test]
    fn a_batch_shape_reports_its_first_path() {
        let t = touched(&call(
            "coder::create-file",
            json!({ "files": [{ "path": "one.rs" }, { "path": "two.rs" }] }),
        ))
        .unwrap();
        assert_eq!(t.path, "one.rs");
    }

    /// Reads must not appear in a change feed.
    #[test]
    fn reads_are_not_changes() {
        assert!(touched(&call("shell::fs::read", json!({ "path": "a" }))).is_none());
        assert!(touched(&call("coder::read-file", json!({ "path": "a" }))).is_none());
        assert!(touched(&call("shell::fs::ls", json!({ "path": "." }))).is_none());
        assert!(touched(&call("coder::tree", json!({ "path": "." }))).is_none());
    }

    /// `shell::exec` can write anything and its argv does not say what, so a
    /// guess would produce phantom events.
    #[test]
    fn exec_is_not_guessed_at() {
        assert!(touched(&call("shell::exec", json!({ "command": "rm -rf x" }))).is_none());
    }

    #[test]
    fn a_write_without_a_path_is_skipped() {
        assert!(touched(&call("shell::fs::write", json!({}))).is_none());
    }

    #[test]
    fn the_session_workspace_is_read_from_the_stamp() {
        let md = json!({ "fs_scope": { "root": "/srv/app" } });
        assert_eq!(session_root(Some(&md)).as_deref(), Some("/srv/app"));
        assert!(session_root(None).is_none());
        assert!(session_root(Some(&json!({}))).is_none());
    }

    #[test]
    fn paths_are_made_relative_to_the_root() {
        assert_eq!(relative("/srv/app/src/a.rs", "/srv/app"), "src/a.rs");
        assert_eq!(relative("/srv/app/src/a.rs", "/srv/app/"), "src/a.rs");
    }

    #[test]
    fn a_path_outside_the_root_is_left_absolute() {
        assert_eq!(relative("/elsewhere/a.rs", "/srv/app"), "/elsewhere/a.rs");
    }

    #[test]
    fn a_dot_root_leaves_the_path_alone() {
        assert_eq!(relative("src/a.rs", "."), "src/a.rs");
    }

    #[test]
    fn the_hook_always_continues() {
        assert_eq!(HookOutput::default().decision, "continue");
    }
}
