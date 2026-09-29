use std::collections::BTreeSet;

use github::webhooks::{
    listeners::{listens, orphaned, stale},
    Data, EventFilter, Subscriber, Watch, WatchRequest, WatchState,
};
use serde_json::json;

fn watch(id: &str, number: u64) -> Watch {
    let request: WatchRequest = serde_json::from_value(json!({
        "watch_id": id, "repo": "owner/repo", "number": number,
        "expires_at": "2099-01-01T00:00:00Z"
    }))
    .unwrap();
    Watch {
        spec: request.validate().unwrap(),
        status: WatchState::Active,
        snapshot: Default::default(),
        lease_id: Some(format!("lease-{id}")),
        error: None,
        seen: BTreeSet::new(),
        orphaned_at: None,
    }
}
fn by_watch(id: &str) -> EventFilter {
    EventFilter {
        watch_id: Some(id.into()),
        ..Default::default()
    }
}
fn subscriber(id: &str, filter: EventFilter) -> Subscriber {
    Subscriber {
        id: id.into(),
        function_id: "harness::trigger::deliver".into(),
        filter,
        metadata: None,
        namespace: None,
    }
}
fn data() -> Data {
    let mut data = Data::default();
    data.watches.insert("chat-a".into(), watch("chat-a", 1));
    data.watches.insert("chat-b".into(), watch("chat-b", 2));
    data
}

#[test]
fn last_listener_leaving_orphans_only_its_watch() {
    let data = data();
    // chat-a's binding was already removed; chat-b still listens.
    let mut data = data;
    data.subscribers
        .insert("b".into(), subscriber("b", by_watch("chat-b")));
    assert_eq!(orphaned(&data, &[by_watch("chat-a")]), ["chat-a"]);
}

#[test]
fn another_listener_keeps_the_watch_alive() {
    let mut data = data();
    data.subscribers
        .insert("other".into(), subscriber("other", by_watch("chat-a")));
    assert!(orphaned(&data, &[by_watch("chat-a")]).is_empty());
    // A repo/number binding (no watch_id) also listens to that PR's watches.
    let mut data = self::data();
    let pr = EventFilter {
        repo: Some("OWNER/repo".into()),
        number: Some(1),
        categories: Some(BTreeSet::from([github::webhooks::Category::Ci])),
        ..Default::default()
    };
    data.subscribers.insert("pr".into(), subscriber("pr", pr));
    assert!(orphaned(&data, &[by_watch("chat-a")]).is_empty());
}

#[test]
fn stopped_or_never_listened_watches_are_not_selected() {
    let mut data = data();
    data.watches.get_mut("chat-a").unwrap().status = WatchState::Stopped;
    assert!(orphaned(&data, &[by_watch("chat-a")]).is_empty());
    // Removing a binding for chat-a never touches chat-b, which nobody armed.
    assert!(!orphaned(&data, &[by_watch("chat-a")]).contains(&"chat-b".to_string()));
}

#[test]
fn an_unfiltered_binding_listens_to_every_watch() {
    let data = data();
    let all = EventFilter::default();
    let w = data.watches.get("chat-b").unwrap();
    assert!(listens(&all, "chat-b", w));
    let mut orphans = orphaned(&data, &[all]);
    orphans.sort();
    assert_eq!(orphans, ["chat-a", "chat-b"]);
}

#[test]
fn stale_bindings_are_known_locally_but_absent_from_the_engine() {
    let known = BTreeSet::from(["live".to_string(), "gone".to_string()]);
    let engine = BTreeSet::from(["live".to_string(), "registered-meanwhile".to_string()]);
    assert_eq!(stale(&known, &engine), ["gone"]);
}
