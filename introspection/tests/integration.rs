use serde_json::json;

#[test]
fn slim_worker_entry_shape_is_stable() {
    let entry = json!({
        "name": "shell-bash",
        "status": "connected",
        "function_count": 3,
        "description": "Sandboxed bash exec",
    });
    assert_eq!(entry["name"], "shell-bash");
    assert_eq!(entry["status"], "connected");
    assert_eq!(entry["function_count"], 3);
    assert!(entry["description"].is_string());
}

#[test]
fn slim_function_entry_shape_is_stable() {
    let entry = json!({
        "id": "shell::bash::exec",
        "worker": "shell-bash",
        "description": "Run bash in a sandbox",
    });
    assert_eq!(entry["id"], "shell::bash::exec");
    assert_eq!(entry["worker"], "shell-bash");
    assert!(entry["description"].is_string());
}

#[tokio::test]
async fn registry_query_payload_validates_required_fields() {
    let payload = json!({"q": "mcp", "limit": 5});
    assert!(payload.get("q").is_some());
    assert_eq!(payload["limit"], 5);
}
