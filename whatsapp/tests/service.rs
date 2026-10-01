use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use serde_json::json;
use whatsapp::{
    service::{EventFilter, EventRouter},
    sessions::Event,
};

fn event() -> Event {
    Event {
        session_id: "work".into(),
        sequence: 1,
        kind: "message.received".into(),
        timestamp_ms: 1000,
        data: json!({}),
    }
}

#[test]
fn filters_match_both_session_and_exact_event_kind() {
    let filter: EventFilter =
        serde_json::from_value(json!({"session_id":"work", "kinds":["message.received"]})).unwrap();
    assert!(filter.matches(&event()));
    let mut other = event();
    other.session_id = "personal".into();
    assert!(!filter.matches(&other));
    other = event();
    other.kind = "message.sent".into();
    assert!(!filter.matches(&other));
    assert!(EventFilter::default().matches(&event()));
}

#[tokio::test]
async fn subscription_validation_and_unregistration_are_effective() {
    let router = EventRouter::default();
    let binding = TriggerConfig {
        id: "a".into(),
        function_id: "consumer::receive".into(),
        config: json!({"session_id":"work"}),
        metadata: Some(json!({"tenant":2})),
        namespace: Some("tenant-2".into()),
    };
    router.register_trigger(binding.clone()).await.unwrap();
    let matched = router.bindings_for(&event()).await;
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].namespace.as_deref(), Some("tenant-2"));
    assert_eq!(matched[0].metadata, Some(json!({"tenant":2})));
    let mut bad = binding.clone();
    bad.namespace = Some(" ".into());
    assert!(router.register_trigger(bad).await.is_err());
    let mut bad = binding.clone();
    bad.config = json!({"kinds":[""]});
    assert!(router.register_trigger(bad).await.is_err());
    let mut bad = binding.clone();
    bad.config = json!({"session":"typo"});
    assert!(router.register_trigger(bad).await.is_err());
    router.unregister_trigger(binding).await.unwrap();
    assert!(router.bindings_for(&event()).await.is_empty());
}
