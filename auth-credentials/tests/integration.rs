//! Smoke tests that run without an iii engine connection.

#[test]
fn in_memory_store_constructs() {
    let _store = auth_credentials::InMemoryStore::new();
}

#[test]
fn credential_serializes_round_trip() {
    let cred = auth_credentials::Credential::ApiKey {
        key: "sk-test".into(),
    };
    let json = serde_json::to_string(&cred).expect("serialize");
    let back: auth_credentials::Credential = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(cred, back);
}
