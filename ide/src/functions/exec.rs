use std::sync::Arc;

use crate::config::ShellConfig;
use crate::exec::host::parse_argv;
use crate::exec::policy::{build_overrides, scope_root_for_target};
use crate::exec_dispatch::pick_exec_backend;
use crate::functions::types::{ExecRequest, ExecResponse};

pub async fn handle(
    cfg: Arc<ShellConfig>,
    iii: iii_sdk::IIIClient,
    req: ExecRequest,
) -> Result<ExecResponse, iii_sdk::errors::Error> {
    // Field-level type errors (wrong-type `command`, non-string `args[i]`,
    // bad `target.kind`) come from the per-field deserializers in
    // `functions::types`; they surface here as the trigger `Err` carrying
    // the actionable text the LLM needs to self-correct.
    // `args.as_ref()` preserves the legacy two-mode contract on `parse_argv`:
    //   None → tokenize `command` via shell-words (single-string path)
    //   Some(_) → use args verbatim, even if empty
    // The typed-schema path must NOT collapse "absent args" into
    // "args: []" or callers lose the shell-words path.
    // argv-parse and denylist rejections are plain Strings with no
    // S-code; via `From<String> for Error` they become the engine's
    // `invocation_failed` envelope, message naming the violation. Only the
    // backend `ExecError` below carries an S-code, surfaced as the wire `code`
    // through `From<ExecError> for Error` (Remote) so an agent can branch
    // on `error.code`.
    let argv = parse_argv(&req.command, req.args.as_ref()).map_err(|e| format!("argv: {}", e))?;

    cfg.is_command_allowed(&argv)?;

    // Gate the per-call cwd/env BEFORE picking a backend. A jail-escaping cwd
    // (S215) or an env key in DANGEROUS_ENV_KEYS (S210) rejects here, carrying
    // the S-code to the wire via From<ExecError>. The
    // sandbox backend additionally rejects any populated override (host-only).
    // `scope_root` only scopes the host working directory: drop it for a sandbox
    // target so the harness-injected session dir does not surface as a host cwd
    // override and get rejected (see `scope_root_for_target`).
    let scope_root =
        scope_root_for_target(&req.target, crate::fs::scope_anchor(req.fs_scope.as_ref()));
    let mut overrides = build_overrides(
        req.cwd.as_deref(),
        req.env.as_ref(),
        scope_root,
        crate::fs::scope_grants(req.fs_scope.as_ref()),
        crate::fs::scope_boundary(req.fs_scope.as_ref()),
        &cfg,
    )
    .map_err(iii_sdk::errors::Error::from)?;
    // stdin needs no gating (opaque input bytes); it is host-only, enforced by
    // the sandbox backend's is_empty() rejection of any populated override.
    overrides.stdin = req.stdin;

    let timeout = cfg.resolve_timeout(req.timeout_ms);
    // `resolve_timeout` clamps a longer request down silently. Remember what
    // was asked for so a kill can say why it happened (see `timeout_note`).
    let requested = req.timeout_ms;

    let backend = pick_exec_backend(req.target, cfg, iii);

    let mut out = backend
        .run(&argv, timeout, &overrides)
        .await
        .map_err(iii_sdk::errors::Error::from)?;

    if out.timed_out {
        let note = timeout_note(requested, timeout);
        // Append; never discard what the command printed before it was killed.
        if out.stderr.is_empty() {
            out.stderr = note;
        } else {
            out.stderr.push('\n');
            out.stderr.push_str(&note);
        }
    }

    Ok(ExecResponse::from(out))
}

/// Why the command was killed, and what to reach for instead.
///
/// A caller that asked for 60s and was clamped to 30s otherwise gets a bare
/// `timed_out: true` at 30s and no hint that its own number was ignored — so
/// it asks again with the same number. Five such retries inside one chapter
/// of `linkly_tutorial` were what surfaced MOT-4766.
fn timeout_note(requested: Option<u64>, effective: u64) -> String {
    let cause = match requested {
        Some(requested) if requested > effective => format!(
            "shell::exec is capped at {effective}ms and the requested {requested}ms was clamped to it; \
             the command was killed at the cap"
        ),
        _ => format!("shell::exec exceeded its {effective}ms timeout and the command was killed"),
    };
    format!(
        "{cause}. For work that runs longer, start it with shell::exec_bg and bind the \
         shell::job-finished trigger to that job_id to be woken when it ends, instead of \
         waiting or re-running it."
    )
}

#[cfg(test)]
mod tests {
    use super::timeout_note;
    use crate::target::Target;

    // The clamp is the part a caller cannot see: it asked for 60s, the cap is
    // 30s, and without this the kill looks like the command's own fault.
    #[test]
    fn the_note_names_the_clamp_and_points_at_the_background_path() {
        let clamped = timeout_note(Some(60_000), 30_000);
        assert!(clamped.contains("60000ms was clamped"), "{clamped}");
        assert!(clamped.contains("capped at 30000ms"), "{clamped}");
        assert!(clamped.contains("shell::exec_bg"), "{clamped}");
        assert!(clamped.contains("shell::job-finished"), "{clamped}");

        // Asking for exactly the cap, or not asking at all, is an honest
        // timeout — do not accuse the runtime of clamping.
        for note in [
            timeout_note(Some(30_000), 30_000),
            timeout_note(None, 10_000),
        ] {
            assert!(!note.contains("clamped"), "{note}");
            assert!(note.contains("shell::exec_bg"), "{note}");
        }
    }
    use serde_json::{json, Value};

    #[test]
    fn target_defaults_to_host_when_absent() {
        let payload = json!({ "command": "echo" });
        let target: Target = match payload.get("target") {
            None | Some(Value::Null) => Target::default(),
            Some(v) => serde_json::from_value(v.clone()).unwrap(),
        };
        assert_eq!(target, Target::Host);
    }

    #[test]
    fn target_field_parses_sandbox_kind() {
        let id = uuid::Uuid::new_v4();
        let payload = json!({
            "command": "ls",
            "target": { "kind": "sandbox", "sandbox_id": id.to_string() },
        });
        let target: Target = serde_json::from_value(payload["target"].clone()).unwrap();
        assert_eq!(target, Target::Sandbox { sandbox_id: id });
    }

    #[test]
    fn malformed_target_returns_error() {
        let payload = json!({
            "command": "ls",
            "target": { "kind": "moon" },
        });
        let result: Result<Target, _> = serde_json::from_value(payload["target"].clone());
        assert!(result.is_err());
    }
}
