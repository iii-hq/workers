use serde_json::{json, Value};

pub fn build_manifest() -> Value {
    json!({
        "name": "shell",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Unix shell execution worker for iii agents",
        "functions": [
            {
                "id": "shell::exec",
                "description": "Execute a command synchronously and return stdout/stderr (capped at max_output_bytes; truncation flagged per stream)",
            },
            {
                "id": "shell::exec_bg",
                "description": "Spawn a command in the background and return job_id",
            },
            {
                "id": "shell::classify_argv",
                "description": "Argv classifier for approval-gate (auto/deny/ask)",
            },
            {
                "id": "shell::kill",
                "description": "Kill a running background job",
            },
            {
                "id": "shell::status",
                "description": "Get status of a background job",
            },
            {
                "id": "shell::list",
                "description": "List all background jobs (running + recently completed)",
            },
            {
                "id": "shell::fs::ls",
                "description": "List directory contents on host or sandbox",
            },
            {
                "id": "shell::fs::stat",
                "description": "Stat a path on host or sandbox",
            },
            {
                "id": "shell::fs::mkdir",
                "description": "Create a directory on host or sandbox",
            },
            {
                "id": "shell::fs::rm",
                "description": "Remove a path on host or sandbox",
            },
            {
                "id": "shell::fs::chmod",
                "description": "Change permissions on host or sandbox",
            },
            {
                "id": "shell::fs::mv",
                "description": "Move/rename a path on host or sandbox",
            },
            {
                "id": "shell::fs::grep",
                "description": "Recursive regex search on host or sandbox",
            },
            {
                "id": "shell::fs::sed",
                "description": "Find-and-replace on host or sandbox",
            },
            {
                "id": "shell::fs::write",
                "description": "Stream a file to a host path or sandbox via StreamChannelRef",
            },
            {
                "id": "shell::fs::read",
                "description": "Stream a file from a host path or sandbox via StreamChannelRef",
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_has_required_fields() {
        let m = build_manifest();
        assert!(m.get("name").is_some());
        assert!(m.get("version").is_some());
        assert!(m.get("functions").is_some());
        let fns = m.get("functions").unwrap().as_array().unwrap();
        assert_eq!(fns.len(), 16);
    }

    #[test]
    fn test_manifest_json_output() {
        let m = build_manifest();
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("shell::exec"));
        assert!(s.contains("shell::exec_bg"));
        assert!(s.contains("shell::kill"));
        assert!(s.contains("shell::fs::read"));
        assert!(s.contains("shell::fs::write"));
        assert!(s.contains("shell::fs::grep"));
    }
}
