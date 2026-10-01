use serde_json::json;
use whatsapp::groups::{GroupRequest, GroupView, ParticipantsRequest, PrivacyRequest};
use whatsapp_rust::features::GroupMetadata;

#[test]
fn group_requests_reject_contact_addresses_before_session_lookup() {
    let request: GroupRequest = serde_json::from_value(json!({
        "session_id": "offline", "group_id": "+15550000001"
    }))
    .unwrap();
    assert!(request.group().is_err());
    let request: GroupRequest = serde_json::from_value(json!({
        "session_id": "offline", "group_id": "120363000000000001@g.us"
    }))
    .unwrap();
    assert!(request.group().is_ok());
}

#[test]
fn participant_mutations_require_distinct_nonempty_contact_targets() {
    for participants in [
        json!([]),
        json!(["120363000000000001@g.us"]),
        json!(["+15550000001", "15550000001@c.us"]),
    ] {
        let request: ParticipantsRequest = serde_json::from_value(json!({
            "session_id": "offline", "group_id": "120363000000000001@g.us",
            "participants": participants
        }))
        .unwrap();
        assert!(request.targets().is_err());
    }
    let request: ParticipantsRequest = serde_json::from_value(json!({
        "session_id": "offline", "group_id": "120363000000000001@g.us",
        "participants": ["+15550000001", "987654321@lid"]
    }))
    .unwrap();
    let targets = request.targets().unwrap();
    assert_eq!(targets[0].to_string(), "15550000001@s.whatsapp.net");
    assert_eq!(targets[1].to_string(), "987654321@lid");
}

#[test]
fn privacy_rejects_unknown_and_invalid_combinations_before_network() {
    for (category, value) in [("online", "contacts"), ("typo", "all"), ("last", "typo")] {
        let request: PrivacyRequest = serde_json::from_value(json!({
            "session_id": "offline", "category": category, "value": value
        }))
        .unwrap();
        assert!(request.setting().is_err());
    }
    let request: PrivacyRequest = serde_json::from_value(json!({
        "session_id": "offline", "category": "last", "value": "contacts"
    }))
    .unwrap();
    assert!(request.setting().is_ok());
}

#[test]
fn group_projection_preserves_absent_metadata_and_string_jids() {
    let metadata = GroupMetadata {
        id: "120363000000000001@g.us".parse().unwrap(),
        parent_group_jid: Some("120363000000000002@g.us".parse().unwrap()),
        is_announcement: true,
        ..Default::default()
    };
    let value = serde_json::to_value(GroupView::from(metadata)).unwrap();
    assert_eq!(value["id"], "120363000000000001@g.us");
    assert_eq!(value["parent_group_id"], "120363000000000002@g.us");
    assert!(value["subject"].is_null());
    assert!(value["ephemeral_expiration_seconds"].is_null());
    assert_eq!(value["announcement"], true);
}

#[test]
fn request_typos_are_not_silently_ignored() {
    assert!(
        serde_json::from_value::<GroupRequest>(json!({
            "session_id": "offline", "group_id": "120363000000000001@g.us",
            "particpants": ["+15550000001"]
        }))
        .is_err()
    );
}

#[tokio::test]
async fn registered_operations_reject_missing_and_stopped_sessions_without_connecting() {
    let directory = tempfile::TempDir::new().unwrap();
    let manager = whatsapp::sessions::SessionManager::open(whatsapp::config::Config {
        data_dir: directory.path().join("sessions"),
        ..Default::default()
    })
    .unwrap();
    manager.create("offline").await.unwrap();
    let mut registry = whatsapp::registry::Registry::default();
    whatsapp::groups::register(&mut registry);
    for session in ["offline", "missing"] {
        for (id, mut input) in [
            (
                "whatsapp::groups::get",
                json!({"group_id": "120363000000000001@g.us"}),
            ),
            (
                "whatsapp::contacts::check",
                json!({"jids": ["+15550000001"]}),
            ),
            (
                "whatsapp::profile::set_name",
                json!({"text": "Offline Test"}),
            ),
            ("whatsapp::privacy::get", json!({})),
            ("whatsapp::communities::list", json!({})),
        ] {
            input["session_id"] = json!(session);
            let error = registry
                .invoke(id, manager.clone(), input)
                .await
                .unwrap_err();
            assert!(!error.to_string().contains("unknown function"));
            assert!(!error.to_string().contains("timed out"));
        }
    }
    assert!(
        !directory
            .path()
            .join("sessions/offline/credentials.db")
            .exists()
    );
    assert_eq!(
        manager.get("offline").await.unwrap().status,
        whatsapp::sessions::SessionStatus::Stopped
    );
}

#[tokio::test]
async fn local_argument_errors_are_reported_before_connecting() {
    let directory = tempfile::TempDir::new().unwrap();
    let manager = whatsapp::sessions::SessionManager::open(whatsapp::config::Config {
        data_dir: directory.path().join("sessions"),
        ..Default::default()
    })
    .unwrap();
    let mut registry = whatsapp::registry::Registry::default();
    whatsapp::groups::register(&mut registry);
    for (id, mut input, expected) in [
        ("whatsapp::groups::create", json!({"subject":"  "}), "blank"),
        (
            "whatsapp::communities::create",
            json!({"name":"  "}),
            "blank",
        ),
        (
            "whatsapp::contacts::save",
            json!({"jid":"987654321@lid","full_name":"Example"}),
            "phone-number",
        ),
        (
            "whatsapp::contacts::remove",
            json!({"jid":"987654321@lid"}),
            "phone-number",
        ),
        (
            "whatsapp::profile::set_picture",
            json!({"jpeg_base64":"aGVsbG8="}),
            "JPEG",
        ),
        (
            "whatsapp::groups::set_picture",
            json!({"group_id":"120363000000000001@g.us","jpeg_base64":"aGVsbG8="}),
            "JPEG",
        ),
    ] {
        input["session_id"] = json!("missing");
        let error = registry
            .invoke(id, manager.clone(), input)
            .await
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{id}: {error}");
    }
}
