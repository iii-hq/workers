//! Trusted per-session filesystem-scope injection.
//!
//! The harness owns this control plane. When a turn carries
//! `metadata.fs_scope.root`, the harness stamps one trusted `fs_scope` object
//! onto every outbound `shell::*` / `coder::*` call. The worker enforces the
//! root and grants; this module only stamps trusted metadata and strips any
//! model-supplied scope.

use serde_json::{json, Value};

pub const FS_SCOPE_FIELD: &str = "fs_scope";

/// True when `function_id` names a filesystem-scoped worker call whose paths
/// must be constrained by the harness-owned scope.
fn is_scoped_function(function_id: &str) -> bool {
    (function_id.starts_with("shell::") && !function_id.starts_with("shell::filesystem::"))
        || function_id.starts_with("coder::")
}

/// Stamp the trusted filesystem scope onto a scoped call's arguments.
/// `origin` carries the issuing (session_id, turn_id) so the shell's write
/// journal can attribute mutations and support undo-by-turn.
pub fn inject(
    function_id: &str,
    args: Value,
    root: Option<&str>,
    grants: &[String],
    origin: Option<(&str, &str)>,
) -> Value {
    if !is_scoped_function(function_id) {
        return args;
    }
    let Value::Object(mut map) = args else {
        return args;
    };

    if let Some(root) = root {
        let mut scope = json!({
            "root": root,
            "grants": grants,
        });
        if let Some((session_id, turn_id)) = origin {
            scope["session_id"] = json!(session_id);
            scope["turn_id"] = json!(turn_id);
        }
        map.insert(FS_SCOPE_FIELD.to_string(), scope);
    } else {
        map.remove(FS_SCOPE_FIELD);
    }

    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stamps_fs_scope_for_shell_calls() {
        let out = inject(
            "shell::exec",
            json!({ "command": "ls" }),
            Some("/work/session-7"),
            &[],
            None,
        );
        assert_eq!(
            out,
            json!({ "command": "ls", "fs_scope": { "root": "/work/session-7", "grants": [] } })
        );
    }

    #[test]
    fn stamps_fs_scope_for_coder_calls() {
        let out = inject(
            "coder::fs::read",
            json!({ "path": "src/main.rs" }),
            Some("/work/session-7"),
            &[],
            None,
        );
        assert_eq!(
            out,
            json!({ "path": "src/main.rs", "fs_scope": { "root": "/work/session-7", "grants": [] } })
        );
    }

    #[test]
    fn overwrites_caller_supplied_fs_scope() {
        let out = inject(
            "shell::exec",
            json!({ "command": "ls", "fs_scope": { "root": "/etc", "grants": ["/model"] } }),
            Some("/work/session-7"),
            &[],
            None,
        );
        assert_eq!(
            out,
            json!({ "command": "ls", "fs_scope": { "root": "/work/session-7", "grants": [] } })
        );
    }

    #[test]
    fn overwrites_caller_supplied_grants_with_trusted_grants() {
        let grants = vec!["/approved".to_string()];
        let out = inject(
            "shell::exec",
            json!({ "command": "ls", "fs_scope": { "root": "/etc", "grants": ["/model"] } }),
            Some("/work/session-7"),
            &grants,
            None,
        );
        assert_eq!(
            out,
            json!({ "command": "ls", "fs_scope": { "root": "/work/session-7", "grants": ["/approved"] } })
        );
    }

    #[test]
    fn strips_caller_supplied_fs_scope_when_root_absent() {
        let args = json!({ "command": "ls", "fs_scope": { "root": "/etc", "grants": ["/model"] } });
        let out = inject("shell::exec", args, None, &[], None);
        assert_eq!(out, json!({ "command": "ls" }));
    }

    #[test]
    fn passthrough_for_filesystem_control_plane_functions() {
        let args = json!({ "path": "/Users/example/project" });
        let out = inject(
            "shell::filesystem::validate",
            args.clone(),
            Some("/work/session-7"),
            &["/approved".to_string()],
            None,
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
            None,
        );
        assert_eq!(out, args);
    }

    #[test]
    fn passthrough_when_args_not_an_object() {
        let args = json!(["ls", "-la"]);
        let out = inject(
            "shell::exec",
            args.clone(),
            Some("/work/session-7"),
            &[],
            None,
        );
        assert_eq!(out, args);

        let scalar = json!("raw");
        let out = inject(
            "coder::fs::read",
            scalar.clone(),
            Some("/work/session-7"),
            &[],
            None,
        );
        assert_eq!(out, scalar);
    }
}
