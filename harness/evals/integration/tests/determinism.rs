//! Deterministic report bytes (spec § Verification): grading the same
//! evidence twice must produce byte-identical canonical JSON.

use harness_integration::canonical::canonical_json_pretty;
use harness_integration::grader::{grade, Evidence};
use harness_integration::types::scenario::InvariantSpecV1;
use serde_json::json;

#[test]
fn grading_twice_yields_identical_bytes() {
    let evidence = Evidence {
        session_id: "s_1".into(),
        turn_id: Some("t_1".into()),
        send_response: Some(json!({ "session_id": "s_1", "turn_id": "t_1", "accepted": true })),
        status: json!({ "status": "completed", "pending_function_calls": [], "children": [] }),
        transcript: vec![json!({
            "entry_id": "e1",
            "message": { "role": "user", "content": [{ "type": "text", "text": "hi" }], "timestamp": 1 }
        })],
        generations_consumed: 1,
        generations_total: 1,
        recorder_events: vec![],
    };
    let specs: Vec<InvariantSpecV1> = serde_json::from_value(json!([
        { "id": "send.flags", "parameters": { "merged": false, "queued": false, "deduplicated": false } },
        { "id": "transcript.no_duplicates", "parameters": {} },
        { "id": "status.terminal", "parameters": { "status": "completed", "pending_calls": 0, "queued_messages": 0 } },
        { "id": "router.generations_consumed", "parameters": { "count": 1 } }
    ]))
    .unwrap();

    let first = canonical_json_pretty(&serde_json::to_value(grade(&specs, &evidence)).unwrap());
    let second = canonical_json_pretty(&serde_json::to_value(grade(&specs, &evidence)).unwrap());
    assert_eq!(first, second);
}
