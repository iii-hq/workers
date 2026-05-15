//! Validation for `__from_approval` on `shell::exec` / `shell::exec_bg`.

use serde_json::Value;

use crate::functions::types::ApprovalMarker;

pub(crate) fn marker_wellformed(marker: &ApprovalMarker) -> Result<(), String> {
    if marker.call_id.trim().is_empty() || marker.session_id.trim().is_empty() {
        return Err("__from_approval marker malformed".into());
    }
    Ok(())
}

/// Normalize `record.args` object (`command` + optional `args` tail) for argv binding.
fn normalized_command_args(stored_args: &Value) -> Result<(String, Option<Vec<String>>), String> {
    let obj = stored_args
        .as_object()
        .ok_or_else(|| "__from_approval approved record has invalid args shape".to_string())?;
    let cmd = obj
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| "__from_approval approved record args missing command".to_string())?
        .to_string();
    let tail = match obj.get("args") {
        None | Some(Value::Null) => None,
        Some(Value::Array(arr)) => {
            let mut v = Vec::with_capacity(arr.len());
            for x in arr {
                let s = x
                    .as_str()
                    .ok_or_else(|| "__from_approval record args.args must be strings".to_string())?
                    .to_string();
                v.push(s);
            }
            Some(v)
        }
        Some(_) => {
            return Err("__from_approval record args.args must be array or null".into());
        }
    };
    Ok((cmd, tail))
}

pub(crate) fn validate_approved_record_for_bypass(
    record: &Value,
    handler_function_id: &str,
    command: &str,
    args: &Option<Vec<String>>,
) -> Result<(), String> {
    let fid = record
        .get("function_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if fid != handler_function_id {
        return Err("__from_approval marker bound to different function_id".into());
    }
    let status = record.get("status").and_then(Value::as_str).unwrap_or("");
    if status != "approved" {
        return Err(format!(
            "__from_approval marker for non-approved record (status: {status})"
        ));
    }
    let stored_root = record.get("args").ok_or_else(|| {
        "__from_approval marker without valid pending approval record".to_string()
    })?;
    let (stored_cmd, stored_tail) = normalized_command_args(stored_root)?;
    if stored_cmd != command {
        return Err("__from_approval marker argv mismatch with approved call".into());
    }
    match (&stored_tail, args) {
        (None, None) => Ok(()),
        (Some(a), Some(b)) if a == b => Ok(()),
        _ => Err("__from_approval marker argv mismatch with approved call".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_mismatched_command() {
        let rec = json!({
            "function_id": "shell::exec",
            "status": "approved",
            "args": {"command": "netstat", "args": ["-an"]}
        });
        let err = validate_approved_record_for_bypass(
            &rec,
            "shell::exec",
            "cat",
            &Some(vec!["/etc/passwd".into()]),
        )
        .expect_err("argv mismatch");
        assert!(err.contains("mismatch"));
    }

    #[test]
    fn rejects_wrong_function_id() {
        let rec = json!({
            "function_id": "shell::exec",
            "status": "approved",
            "args": {"command": "echo"}
        });
        let err = validate_approved_record_for_bypass(&rec, "shell::exec_bg", "echo", &None)
            .expect_err("fid mismatch");
        assert!(err.contains("different function_id"));
    }

    #[test]
    fn rejects_non_approved_status() {
        let rec = json!({
            "function_id": "shell::exec",
            "status": "executed",
            "args": {"command": "echo"}
        });
        let err = validate_approved_record_for_bypass(&rec, "shell::exec", "echo", &None)
            .expect_err("status");
        assert!(err.contains("non-approved"));
    }

    #[test]
    fn accepts_matching_payload() {
        let rec = json!({
            "function_id": "shell::exec",
            "status": "approved",
            "args": {"command": "echo", "args": ["hi"]}
        });
        validate_approved_record_for_bypass(
            &rec,
            "shell::exec",
            "echo",
            &Some(vec!["hi".into()]),
        )
        .unwrap();
    }
}
