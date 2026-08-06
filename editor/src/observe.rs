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
use crate::{diff, workspace};

pub const HOOK_FN_ID: &str = "editor::on-file-change";
/// The two families that touch files. Matching broadly and filtering on the
/// verb keeps this from breaking when either worker grows a new write path.
const HOOK_FUNCTIONS: &[&str] = &["shell::*", "coder::*"];
const HOOK_TIMEOUT_MS: u64 = 3_000;
/// A viewer must never be able to block a write.
const HOOK_ON_ERROR: &str = "fail_open";

/// The subset of the harness hook payload this worker reads.
///
/// `session_id` and `turn_id` come from the hook envelope, which is the only
/// place the identity of the writer exists: the call itself says a file was
/// written, not who was writing. Carrying them onto the event is what lets a
/// surface say *this* agent session made *this* change, rather than reporting
/// an anonymous edit. Both stay optional — a hook fired outside a turn (or by
/// an operator reproducing one) has no session, and that is not an error.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct HookInput {
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub call: Option<HookCall>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    /// Outcome of the call this hook is reporting on. A write that failed did
    /// not change anything, and the hook fires either way.
    #[serde(default)]
    pub result: Option<HookResult>,
}

/// The part of the hook's result payload that says whether the call worked.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct HookResult {
    #[serde(default)]
    pub is_error: bool,
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

/// Collapse the platform's aliases for the same directory.
///
/// On macOS `/tmp` is a symlink to `/private/tmp`, and `/var` to `/private/var`.
/// Two workers writing the same file by different names produced two rows for
/// one file — the feed's dedupe is by path, and these are the same path spelled
/// two ways. Resolving to the real location makes them one entry again.
pub fn canonical(path: &str) -> String {
    for alias in ["/tmp/", "/var/"] {
        if let Some(rest) = path.strip_prefix(alias) {
            return format!("/private{alias}{rest}");
        }
    }
    path.to_string()
}

/// Make `path` relative to `root`, leaving anything outside it alone.
pub fn relative(path: &str, root: &str) -> String {
    if root == "." || root.is_empty() {
        return path.to_string();
    }
    let trimmed = root.trim_end_matches('/');
    path.strip_prefix(trimmed)
        // Whole segments only. A raw `strip_prefix` treats the root as a string
        // rather than a path, so root `/srv/app` turned `/srv/application/a.rs`
        // into `lication/a.rs` — a plausible-looking path pointing nowhere,
        // reported as the file that changed. This is the same invariant
        // `Session::remap` is built on, for the same reason: a prefix that ends
        // mid-segment is not a parent.
        .filter(|rest| rest.is_empty() || rest.starts_with('/'))
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
                // Recording no longer waits for a subscriber. Skipping when
                // nobody was watching meant the feed could only ever show what
                // happened while a page was open, so the question it exists to
                // answer — what did the agent do while I was elsewhere — was
                // exactly the one it could not answer. The work is bounded and
                // fail-open, and the whole hook is `fail_open` besides.
                let snapshot = cfg.read().await.clone();
                // Named span under the caller's trace: this fires inside an
                // agent turn, and an observed edit that dangled as its own
                // root would be unreadable in the traces view — the whole
                // point is seeing the write and the event as one chain.
                iii_helpers::observability::run_in_span(
                    "editor::observe file change",
                    None,
                    || report(&bus, &snapshot, &emitter, input),
                )
                .await;
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
        namespace: iii.namespace(),
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
    let session_id = input.session_id.clone();
    let turn_id = input.turn_id.clone();
    // A failed write is not a change. The hook runs after every call whether
    // it worked or not, so without this a `shell::fs::write` refused by the
    // filesystem jail was reported as an edit — and the file it named appeared
    // in the feed as something that had happened.
    if input.result.as_ref().is_some_and(|r| r.is_error) {
        tracing::debug!("observer: the call failed, nothing changed");
        return;
    }
    let Some(call) = input.call else {
        tracing::debug!("observer: hook fired with no call payload");
        return;
    };
    let Some(touch) = touched(&call) else {
        tracing::debug!(function_id = %call.function_id, "observer: not a write, ignoring");
        return;
    };

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
    // Canonicalize both sides before comparing them: a write addressed as
    // /tmp/x and a root of /private/tmp are the same place, and comparing the
    // spellings would call the file "outside the workspace".
    let rel = relative(&canonical(&touch.path), &canonical(&root));
    // `bus.read` resolves through shell, whose jail/working_dir is its own, NOT
    // this root. Handing it the root-relative path made every read outside
    // shell's cwd fail into an empty patch — silently, because the fallback
    // swallows the error. Reads therefore go out absolute; only the reported
    // path stays relative.
    let absolute = if touch.path.starts_with('/') {
        touch.path.clone()
    } else if root == "." {
        rel.clone()
    } else {
        format!("{}/{}", root.trim_end_matches('/'), rel)
    };
    tracing::debug!(
        function_id = %call.function_id,
        kind = touch.kind,
        path = %rel,
        root = %root,
        "observer: reporting a change"
    );

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
            _ => match bus.read(&absolute, cfg.max_file_bytes).await {
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

    let event = ChangedEvent {
        path: rel,
        cause: call.function_id,
        kind: touch.kind.to_string(),
        added,
        removed,
        patch,
        truncated: false,
        root,
        session_id,
        turn_id,
    };

    // Record before pushing. A surface that opens later reads the log, and a
    // surface that is already open gets the push; recording first means the
    // two never disagree about what happened. Best-effort, like the emit: a
    // state write that fails must not fail the agent's write, which has
    // already happened.
    record(bus, &event).await;
    emitter.emit(event).await;
}

/// Append one change to the durable log, newest first.
///
/// Read-modify-write against `state` is not serialized here on purpose: the
/// hook runs inside one turn at a time per session, and a lost entry in a
/// recent-activity feed is a far smaller cost than holding a lock across a
/// bus round trip on the write path of every agent edit.
async fn record(bus: &Bus, event: &ChangedEvent) {
    let existing = bus
        .state_get(workspace::CHANGES_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value::<Vec<Value>>(v).ok())
        .unwrap_or_default();
    let Ok(entry) = serde_json::to_value(event) else {
        return;
    };
    let next = workspace::record_change(&existing, entry, |e| {
        e.get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    });
    if let Err(e) = bus
        .state_set(workspace::CHANGES_KEY, Value::Array(next))
        .await
    {
        tracing::debug!(error = %e, "observer: could not record the change");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_collapses_the_macos_symlinked_roots() {
        assert_eq!(canonical("/tmp/demo/a.md"), "/private/tmp/demo/a.md");
        assert_eq!(canonical("/var/folders/x"), "/private/var/folders/x");
        // Already real, or nothing to do with those roots: untouched.
        assert_eq!(
            canonical("/private/tmp/demo/a.md"),
            "/private/tmp/demo/a.md"
        );
        assert_eq!(canonical("/Users/me/repo/a.rs"), "/Users/me/repo/a.rs");
        assert_eq!(canonical("src/main.rs"), "src/main.rs");
    }

    #[test]
    fn one_file_addressed_two_ways_is_one_path() {
        // The bug this exists for: /tmp/x and /private/tmp/x are the same
        // file, and the feed keys on the path, so two spellings made two rows.
        assert_eq!(
            relative(
                &canonical("/tmp/demo/a.md"),
                &canonical("/private/tmp/demo")
            ),
            relative(
                &canonical("/private/tmp/demo/a.md"),
                &canonical("/tmp/demo")
            ),
        );
    }

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

    /// The whole write whitelist in one place. A verb dropped from the table
    /// stops producing events silently, which is exactly the blindness this
    /// module exists to fix — and only three of the eight were pinned.
    #[test]
    fn every_watched_verb_maps_to_its_kind() {
        for (id, kind) in [
            ("shell::fs::write", "modified"),
            ("shell::fs::sed", "modified"),
            ("coder::update-file", "modified"),
            ("coder::create-file", "created"),
            ("shell::fs::rm", "deleted"),
            ("coder::delete-file", "deleted"),
            ("shell::fs::mv", "moved"),
            ("coder::move", "moved"),
        ] {
            let t = touched(&call(id, json!({ "path": "a.rs" })))
                .unwrap_or_else(|| panic!("{id} produced no touch"));
            assert_eq!(t.kind, kind, "{id} reported the wrong kind");
            assert_eq!(t.path, "a.rs", "{id} lost its path");
        }
    }

    /// A move carries both ends. The destination is the one a surface opens,
    /// so it has to win even when the source is present under `path`.
    #[test]
    fn a_destination_outranks_the_source_path() {
        let t = touched(&call(
            "coder::move",
            json!({ "path": "old.rs", "dst": "new.rs" }),
        ))
        .unwrap();
        assert_eq!(t.path, "new.rs");
    }

    #[test]
    fn a_batch_move_reports_its_destination() {
        let t = touched(&call(
            "coder::move",
            json!({ "files": [{ "dst": "new.rs" }] }),
        ))
        .unwrap();
        assert_eq!(t.path, "new.rs");
    }

    #[test]
    fn an_empty_batch_is_skipped() {
        assert!(touched(&call("coder::create-file", json!({ "files": [] }))).is_none());
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

    /// A sibling whose name merely starts with the root's is not inside it.
    /// Stripping by string rather than by segment turned
    /// `/srv/application/a.rs` into `lication/a.rs`: a path that looks real,
    /// resolves nowhere, and would be reported as the file that changed.
    #[test]
    fn a_sibling_sharing_the_roots_prefix_is_not_inside_it() {
        assert_eq!(
            relative("/srv/application/a.rs", "/srv/app"),
            "/srv/application/a.rs"
        );
        assert_eq!(relative("/srv/app-2/a.rs", "/srv/app"), "/srv/app-2/a.rs");
        assert_eq!(relative("/srv/appendix", "/srv/app"), "/srv/appendix");
        // The genuine child still resolves, so the guard did not overshoot.
        assert_eq!(relative("/srv/app/a.rs", "/srv/app"), "a.rs");
    }

    /// The harness stamps `fs_scope` with whatever the session had; anything
    /// that is not a string root is no root at all, and the observer falls back
    /// to the workspace's own.
    #[test]
    fn a_root_that_is_not_a_string_is_not_a_session_root() {
        assert!(session_root(Some(&json!({ "fs_scope": { "root": 7 } }))).is_none());
        assert!(session_root(Some(&json!({ "fs_scope": {} }))).is_none());
        assert!(session_root(Some(&json!({ "fs_scope": null }))).is_none());
    }

    #[test]
    fn a_dot_root_leaves_the_path_alone() {
        assert_eq!(relative("src/a.rs", "."), "src/a.rs");
    }

    /// No root stamped and none stored: the path is already the best answer.
    #[test]
    fn an_empty_root_leaves_the_path_alone() {
        assert_eq!(relative("src/a.rs", ""), "src/a.rs");
        assert_eq!(relative("/srv/app/a.rs", ""), "/srv/app/a.rs");
    }

    /// A write reported against the root itself must not collapse to an empty
    /// path, which no surface can render and no read can resolve.
    #[test]
    fn a_path_equal_to_the_root_is_left_alone() {
        assert_eq!(relative("/srv/app", "/srv/app"), "/srv/app");
        assert_eq!(relative("/srv/app/", "/srv/app"), "/srv/app/");
    }

    #[test]
    fn the_hook_always_continues() {
        assert_eq!(HookOutput::default().decision, "continue");
    }
}
