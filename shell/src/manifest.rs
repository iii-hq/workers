use serde_json::{json, Value};

pub fn build_manifest() -> Value {
    json!({
        "name": "iii-shell",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Unix shell execution worker for iii agents",
        "functions": [
            {
                "id": "shell::exec",
                "description": "Execute a command synchronously and return full stdout/stderr",
            },
            {
                "id": "shell::exec_bg",
                "description": "Spawn a command in the background and return job_id",
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
        assert_eq!(fns.len(), 5);
    }

    #[test]
    fn test_manifest_json_output() {
        let m = build_manifest();
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("shell::exec"));
        assert!(s.contains("shell::exec_bg"));
        assert!(s.contains("shell::kill"));
    }
}
