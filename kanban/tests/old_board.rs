//! A board written before pills existed still loads: the field is optional on
//! disk, so an older `board.json` is never invalid.

use iii_kanban::store::Ticket;

#[test]
fn a_ticket_without_pills_deserializes_to_none() {
    let json = r#"{
        "id": "0f1c3c2a-7d7e-4b2e-9c1e-0a1b2c3d4e5f",
        "key": "KAN-1",
        "title": "Water the plants",
        "status": "backlog",
        "priority": "low",
        "labels": ["home"],
        "created_at": "2026-10-01T10:00:00Z",
        "updated_at": "2026-10-01T10:00:00Z"
    }"#;
    let ticket: Ticket = serde_json::from_str(json).expect("an old ticket loads");
    assert!(ticket.pills.is_empty());
    assert_eq!(ticket.labels, vec!["home".to_string()]);
    // And it writes back without inventing anything beyond an empty list.
    let round = serde_json::to_value(&ticket).unwrap();
    assert_eq!(round["pills"], serde_json::json!([]));
}
