//! Per-session working-directory injection (tech-specs/2026-06-agentic).
//!
//! The harness owns workspace scoping: when a turn carries a `working_dir`, the
//! harness stamps it onto every outbound `shell::*` / `coder::*` call as the
//! per-call `base_dir` field. The worker then anchors relative paths at that
//! directory and rejects absolute paths that escape it (the containment check
//! lives in the worker; this module is only the control-plane stamp).
//!
//! This is a control-plane decision, not a hint: the model never gets to widen
//! its own scope, so `base_dir` is OVERWRITTEN even if the caller supplied one.

use serde_json::Value;

/// The per-call field the harness stamps onto scoped function arguments.
/// `snake_case` on the wire — the shell/coder workers read it verbatim.
const BASE_DIR_FIELD: &str = "base_dir";
const EXTRA_ROOTS_FIELD: &str = "extra_roots";

/// True when `function_id` names a workspace-scoped worker call (`shell::*` or
/// `coder::*`) whose paths must be anchored at the session working directory.
fn is_scoped_function(function_id: &str) -> bool {
    (function_id.starts_with("shell::") && !function_id.starts_with("shell::workspace::"))
        || function_id.starts_with("coder::")
}

/// Stamp the session `working_dir` onto a scoped call's arguments as `base_dir`.
///
/// Returns a NEW `Value` (the codebase favours immutability — no in-place
/// mutation of the caller's args). The original is returned unchanged when
/// `function_id` is not a `shell::*` / `coder::*` call, or when `args` is not a
/// JSON object (nowhere to place a top-level field).
///
/// When it does apply, the top-level `base_dir` is SET to `working_dir`,
/// OVERWRITING any caller-supplied value. If no `working_dir` is active, a
/// caller-supplied `base_dir` is removed. `extra_roots` is always replaced with
/// the trusted grant list, or stripped when that list is empty. The harness
/// controls scoping and the model must not be able to widen it.
pub fn inject(
    function_id: &str,
    args: Value,
    working_dir: Option<&str>,
    extra_roots: &[String],
) -> Value {
    if !is_scoped_function(function_id) {
        return args;
    }
    let Value::Object(mut map) = args else {
        return args;
    };
    match working_dir {
        Some(dir) => {
            map.insert(BASE_DIR_FIELD.to_string(), Value::String(dir.to_string()));
        }
        None => {
            map.remove(BASE_DIR_FIELD);
        }
    }
    if extra_roots.is_empty() {
        map.remove(EXTRA_ROOTS_FIELD);
    } else {
        map.insert(
            EXTRA_ROOTS_FIELD.to_string(),
            Value::Array(
                extra_roots
                    .iter()
                    .map(|root| Value::String(root.clone()))
                    .collect(),
            ),
        );
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stamps_base_dir_for_shell_calls() {
        let out = inject(
            "shell::exec",
            json!({ "command": "ls" }),
            Some("/work/session-7"),
            &[],
        );
        assert_eq!(
            out,
            json!({ "command": "ls", "base_dir": "/work/session-7" })
        );
    }

    #[test]
    fn stamps_base_dir_for_coder_calls() {
        let out = inject(
            "coder::fs::read",
            json!({ "path": "src/main.rs" }),
            Some("/work/session-7"),
            &[],
        );
        assert_eq!(
            out,
            json!({ "path": "src/main.rs", "base_dir": "/work/session-7" })
        );
    }

    #[test]
    fn overwrites_caller_supplied_base_dir() {
        // The model must not be able to widen its own scope: a caller-supplied
        // base_dir is replaced by the harness-controlled working_dir.
        let out = inject(
            "shell::exec",
            json!({ "command": "ls", "base_dir": "/etc" }),
            Some("/work/session-7"),
            &[],
        );
        assert_eq!(
            out,
            json!({ "command": "ls", "base_dir": "/work/session-7" })
        );
    }

    #[test]
    fn overwrites_caller_supplied_extra_roots_with_trusted_grants() {
        let grants = vec!["/approved".to_string()];
        let out = inject(
            "shell::exec",
            json!({ "command": "ls", "base_dir": "/etc", "extra_roots": ["/model"] }),
            Some("/work/session-7"),
            &grants,
        );
        assert_eq!(
            out,
            json!({ "command": "ls", "base_dir": "/work/session-7", "extra_roots": ["/approved"] })
        );
    }

    #[test]
    fn strips_caller_supplied_base_dir_when_working_dir_absent() {
        // With no session scope, the model must not be able to invent one.
        let args = json!({ "command": "ls", "base_dir": "/etc", "extra_roots": ["/model"] });
        let out = inject("shell::exec", args, None, &[]);
        assert_eq!(out, json!({ "command": "ls" }));
    }

    #[test]
    fn passthrough_for_workspace_control_plane_functions() {
        let args = json!({ "path": "/Users/example/project" });
        let out = inject(
            "shell::workspace::validate",
            args.clone(),
            Some("/work/session-7"),
            &["/approved".to_string()],
        );
        assert_eq!(out, args);
    }

    #[test]
    fn passthrough_for_non_scoped_function() {
        let args = json!({ "to": "user", "text": "hi" });
        let out = inject(
            "telegram::send",
            args.clone(),
            Some("/work/session-7"),
            &["/approved".to_string()],
        );
        assert_eq!(out, args);
    }

    #[test]
    fn passthrough_when_args_not_an_object() {
        // A non-object payload has no top-level slot for base_dir; return it as
        // received rather than reshaping it.
        let args = json!(["ls", "-la"]);
        let out = inject("shell::exec", args.clone(), Some("/work/session-7"), &[]);
        assert_eq!(out, args);

        let scalar = json!("raw");
        let out = inject(
            "coder::fs::read",
            scalar.clone(),
            Some("/work/session-7"),
            &[],
        );
        assert_eq!(out, scalar);
    }
}
