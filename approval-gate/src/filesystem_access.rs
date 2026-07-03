//! Pure `filesystem_access_request` parsing + the first half of the `approval::filesystem-access-watch`
//! decision ladder. No I/O — mirrors
//! `decision.rs`: exhaustively unit tested in isolation, the async
//! orchestration in `functions/filesystem_access_watch.rs` calls straight through.
//!
//! Ladder steps 1-4 live here (is_error / code / request parse / scope-root
//! guard); steps 5-8 need state + harness RPCs and live in
//! `functions/filesystem_access_watch.rs`.

use crate::types::{AccessRequest, HookInput, HookResult};

/// The jail-scope rejection codes a `filesystem_access_request` can ride on.
pub const JAIL_SCOPE_CODES: [&str; 4] = ["S215", "S220", "C215", "C218"];

const ACCESS_REQUEST_MARKER: &str = "filesystem_access_request=";

/// The minimal shape parsed out of the `filesystem_access_request=` tail. Extra JSON keys
/// (e.g. newer request metadata) are ignored — `serde_json` skips unknown
/// fields by default.
#[derive(Debug, Clone, serde::Deserialize)]
struct RawFilesystemAccessRequest {
    v: u32,
    requested_root: String,
    #[serde(default)]
    attempted_path: String,
    #[serde(default)]
    error_code: Option<String>,
}

/// `rfind("filesystem_access_request=")` then JSON-parse from just after `=` to
/// end-of-string. Requires `v == 1` and a non-empty absolute root. A marker
/// marker that appears anywhere earlier than the true tail is rejected
/// because the JSON parse consumes the WHOLE remainder — junk after a
/// false-positive `filesystem_access_request=` substring fails to parse as the hint shape.
fn parse_tail(message: &str, error_code: Option<&str>) -> Option<AccessRequest> {
    let idx = message.rfind(ACCESS_REQUEST_MARKER)?;
    let json_str = message[idx + ACCESS_REQUEST_MARKER.len()..].trim();
    let raw: RawFilesystemAccessRequest = serde_json::from_str(json_str).ok()?;
    if raw.v != 1 {
        return None;
    }
    if raw.requested_root.is_empty() || !raw.requested_root.starts_with('/') {
        return None;
    }
    let error_code = error_code.or(raw.error_code.as_deref())?;
    if !JAIL_SCOPE_CODES.contains(&error_code) {
        return None;
    }
    Some(AccessRequest {
        requested_root: raw.requested_root,
        attempted_path: raw.attempted_path,
        error_code: error_code.to_string(),
    })
}

/// Try the error message first, then the first text content block
/// Falls back from the structured error message to the first text content block.
fn parse_filesystem_access_request(result: &HookResult) -> Option<AccessRequest> {
    let error = result.details.error.as_ref()?;
    if let Some(hint) = parse_tail(&error.message, error.code.as_deref()) {
        return Some(hint);
    }
    let first_text = result.content.first()?;
    parse_tail(&first_text.text, error.code.as_deref())
}

/// Component-safe prefix containment: `/a/b` contains itself and `/a/b/c`,
/// but NOT `/a/bc`. Both `root` and `candidate` are expected to be
/// canonical absolute paths.
pub fn dir_contains(root: &str, candidate: &str) -> bool {
    if candidate == root {
        return true;
    }
    let prefix = if root.ends_with('/') {
        root.to_string()
    } else {
        format!("{root}/")
    };
    candidate.starts_with(&prefix)
}

/// Ladder steps 1-4's verdict.
#[derive(Debug, Clone, PartialEq)]
pub enum HintOutcome {
    /// Not an error, wrong/absent code, or no parseable hint — continue
    /// silently, the error is delivered to the model unchanged.
    NoHint,
    /// The requested root is the session workspace itself (or under it) — a
    /// harness stamp hole, not something to prompt about. The caller logs
    /// at ERROR and continues.
    WorkspaceItself(AccessRequest),
    /// Eligible for the rest of the ladder (steps 5-8).
    Candidate(AccessRequest),
}

/// Steps 1-4: does this post_trigger envelope carry a jail-scope
/// `filesystem_access_request` worth acting on?
pub fn extract_hint(input: &HookInput) -> HintOutcome {
    let Some(result) = &input.result else {
        return HintOutcome::NoHint;
    };
    if !result.is_error {
        return HintOutcome::NoHint;
    }
    let Some(error) = &result.details.error else {
        return HintOutcome::NoHint;
    };
    if let Some(code) = error.code.as_deref() {
        if !JAIL_SCOPE_CODES.contains(&code) {
            return HintOutcome::NoHint;
        }
    }
    let Some(hint) = parse_filesystem_access_request(result) else {
        return HintOutcome::NoHint;
    };

    if let Some(metadata) = &input.metadata {
        let scope_root = metadata
            .get("fs_scope")
            .and_then(|v| v.as_object())
            .and_then(|scope| scope.get("root"))
            .and_then(|v| v.as_str());
        if let Some(root) = scope_root {
            if dir_contains(root, &hint.requested_root) {
                return HintOutcome::WorkspaceItself(hint);
            }
        }
    }

    HintOutcome::Candidate(hint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::HookCall;
    use serde_json::{json, Value};

    fn hint_message(requested_root: &str) -> String {
        format!(
            "shell::fs::read rejected: outside jail filesystem_access_request={{\"v\":1,\"requested_root\":\"{requested_root}\",\"attempted_path\":\"/raw/path\",\"error_code\":\"S215\"}}"
        )
    }

    fn result_with(
        is_error: bool,
        code: Option<&str>,
        message: Option<&str>,
        content: Vec<Value>,
    ) -> Value {
        json!({
            "function_call_id": "c_1",
            "function_id": "shell::fs::read",
            "content": content,
            "is_error": is_error,
            "details": {
                "error": code.map(|c| json!({ "code": c, "message": message.unwrap_or_default() })),
            },
        })
    }

    fn text_block(text: &str) -> Value {
        json!({ "type": "text", "text": text })
    }

    fn input_with(result: Option<Value>, metadata: Option<Value>) -> HookInput {
        let mut base = json!({
            "session_id": "s_1",
            "turn_id": "t_1",
            "depth": 0,
            "call": { "id": "c_1", "function_id": "shell::fs::read", "arguments": {} },
        });
        if let Some(result) = result {
            base["result"] = result;
        }
        if let Some(metadata) = metadata {
            base["metadata"] = metadata;
        }
        serde_json::from_value(base).unwrap()
    }

    // --- rung 1: not an error ---

    #[test]
    fn not_an_error_is_no_hint() {
        let result = result_with(false, Some("S215"), Some(&hint_message("/a/b")), vec![]);
        assert_eq!(
            extract_hint(&input_with(Some(result), None)),
            HintOutcome::NoHint
        );
    }

    #[test]
    fn missing_result_is_no_hint() {
        assert_eq!(extract_hint(&input_with(None, None)), HintOutcome::NoHint);
    }

    // --- rung 2: wrong / absent code ---

    #[test]
    fn wrong_code_is_no_hint() {
        let result = result_with(true, Some("S499"), Some(&hint_message("/a/b")), vec![]);
        assert_eq!(
            extract_hint(&input_with(Some(result), None)),
            HintOutcome::NoHint
        );
    }

    #[test]
    fn missing_error_details_is_no_hint() {
        let result = result_with(true, None, None, vec![]);
        assert_eq!(
            extract_hint(&input_with(Some(result), None)),
            HintOutcome::NoHint
        );
    }

    #[test]
    fn every_jail_scope_code_is_accepted() {
        for code in JAIL_SCOPE_CODES {
            let message = format!(
                "rejected filesystem_access_request={{\"v\":1,\"requested_root\":\"/a/b\",\"attempted_path\":\"/a/b/c\",\"error_code\":\"{code}\"}}"
            );
            let result = result_with(true, Some(code), Some(&message), vec![]);
            assert!(matches!(
                extract_hint(&input_with(Some(result), None)),
                HintOutcome::Candidate(_)
            ));
        }
    }

    #[test]
    fn null_error_code_uses_filesystem_access_request_code() {
        let message = "remote error (S215): outside jail filesystem_access_request={\"v\":1,\"requested_root\":\"/a/b\",\"attempted_path\":\"/a/b/c\",\"error_code\":\"S215\"}";
        let result = json!({
            "function_call_id": "c_1",
            "function_id": "shell::fs::read",
            "content": [],
            "is_error": true,
            "details": {
                "error": { "code": null, "message": message },
            },
        });

        let HintOutcome::Candidate(hint) = extract_hint(&input_with(Some(result), None)) else {
            panic!("expected null code to fall back to the filesystem_access_request code");
        };
        assert_eq!(hint.error_code, "S215");
    }

    // --- rung 3: hint parsing ---

    #[test]
    fn no_hint_marker_is_no_hint() {
        let result = result_with(true, Some("S215"), Some("outside the jail, sorry"), vec![]);
        assert_eq!(
            extract_hint(&input_with(Some(result), None)),
            HintOutcome::NoHint
        );
    }

    #[test]
    fn tail_extraction_ignores_junk_before_the_marker() {
        let message =
            "some prefix filesystem_access_request= is not json, and neither is this filesystem_access_request={\"v\":1,\"requested_root\":\"/a/b\",\"attempted_path\":\"/a/b/c\"}";
        let result = result_with(true, Some("S215"), Some(message), vec![]);
        let HintOutcome::Candidate(hint) = extract_hint(&input_with(Some(result), None)) else {
            panic!("expected a candidate");
        };
        assert_eq!(hint.requested_root, "/a/b");
    }

    #[test]
    fn hint_mid_message_is_rejected_when_not_last() {
        // rfind locks onto the LAST marker; if trailing junk follows the
        // real hint, the JSON parse of "hint json + junk" fails.
        let message =
            "filesystem_access_request={\"v\":1,\"requested_root\":\"/a/b\",\"attempted_path\":\"/a/b/c\"} trailing junk after";
        let result = result_with(true, Some("S215"), Some(message), vec![]);
        assert_eq!(
            extract_hint(&input_with(Some(result), None)),
            HintOutcome::NoHint
        );
    }

    #[test]
    fn wrong_version_is_rejected() {
        let message = "filesystem_access_request={\"v\":2,\"requested_root\":\"/a/b\",\"attempted_path\":\"/a/b/c\"}";
        let result = result_with(true, Some("S215"), Some(message), vec![]);
        assert_eq!(
            extract_hint(&input_with(Some(result), None)),
            HintOutcome::NoHint
        );
    }

    #[test]
    fn relative_requested_root_is_rejected() {
        let message = "filesystem_access_request={\"v\":1,\"requested_root\":\"relative/path\",\"attempted_path\":\"x\"}";
        let result = result_with(true, Some("S215"), Some(message), vec![]);
        assert_eq!(
            extract_hint(&input_with(Some(result), None)),
            HintOutcome::NoHint
        );
    }

    #[test]
    fn empty_requested_root_is_rejected() {
        let message =
            "filesystem_access_request={\"v\":1,\"requested_root\":\"\",\"attempted_path\":\"x\"}";
        let result = result_with(true, Some("S215"), Some(message), vec![]);
        assert_eq!(
            extract_hint(&input_with(Some(result), None)),
            HintOutcome::NoHint
        );
    }

    #[test]
    fn falls_back_to_first_text_content_block() {
        let result = result_with(
            true,
            Some("S215"),
            Some("no marker here"),
            vec![text_block(&hint_message("/a/b"))],
        );
        let HintOutcome::Candidate(hint) = extract_hint(&input_with(Some(result), None)) else {
            panic!("expected a candidate");
        };
        assert_eq!(hint.requested_root, "/a/b");
        assert_eq!(hint.error_code, "S215");
    }

    // --- rung 4: workspace guard ---

    #[test]
    fn workspace_itself_is_flagged_not_continued_silently() {
        let result = result_with(true, Some("S215"), Some(&hint_message("/ws")), vec![]);
        let input = input_with(Some(result), Some(json!({ "fs_scope": { "root": "/ws" } })));
        assert!(matches!(
            extract_hint(&input),
            HintOutcome::WorkspaceItself(_)
        ));
    }

    #[test]
    fn requested_root_under_workspace_is_workspace_itself() {
        let result = result_with(true, Some("S215"), Some(&hint_message("/ws/sub")), vec![]);
        let input = input_with(Some(result), Some(json!({ "fs_scope": { "root": "/ws" } })));
        assert!(matches!(
            extract_hint(&input),
            HintOutcome::WorkspaceItself(_)
        ));
    }

    #[test]
    fn sibling_directory_with_shared_prefix_is_not_the_workspace() {
        // /ws/sub-sibling must NOT be treated as under /ws/sub.
        let result = result_with(
            true,
            Some("S215"),
            Some(&hint_message("/ws-sibling")),
            vec![],
        );
        let input = input_with(Some(result), Some(json!({ "fs_scope": { "root": "/ws" } })));
        assert!(matches!(extract_hint(&input), HintOutcome::Candidate(_)));
    }

    #[test]
    fn no_fs_scope_metadata_never_guards() {
        let result = result_with(true, Some("S215"), Some(&hint_message("/a/b")), vec![]);
        assert!(matches!(
            extract_hint(&input_with(Some(result), None)),
            HintOutcome::Candidate(_)
        ));
    }

    // --- dir_contains: the component-safe boundary ---

    #[test]
    fn dir_contains_is_component_safe() {
        assert!(dir_contains("/a/b", "/a/b"));
        assert!(dir_contains("/a/b", "/a/b/c"));
        assert!(!dir_contains("/a/b", "/a/bc"));
        assert!(!dir_contains("/a/b", "/a/b-sibling"));
        assert!(dir_contains("/a/b/", "/a/b/c"));
    }

    // Sanity: HookCall is still usable via HookInput.call after these
    // additive changes (regression guard for the type surface).
    #[test]
    fn call_field_still_round_trips() {
        let input = input_with(None, None);
        let call: HookCall = input.call.unwrap();
        assert_eq!(call.function_id, "shell::fs::read");
    }
}
