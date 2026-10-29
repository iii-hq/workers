//! `email::new-mail` / `email::on-new-mail` (iii 0.25): the observable
//! function refuses direct invocation, and an observer filed through its
//! hooks receives the payload a binding of the same (account, folder) gets.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use email::triggers::dispatcher::deliver_to_observers;
use email::triggers::new_mail::{
    codes, refuse_invocation, Handler, ObserveNewMail, NEW_MAIL_TRIGGER_TYPE, ON_NEW_MAIL_FN,
};
use email::triggers::registry::{NewMailObserver, TriggerRegistry};
use email::triggers::{Event, NewMailEvent};
use iii_sdk::errors::Error;
use iii_sdk::iii::IntoSyncHandler;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use serde_json::{json, Value};

/// An observer that records what it is handed, or fails with `fail_with`.
struct Recording {
    id: String,
    fail_with: Option<&'static str>,
    received: Mutex<Vec<(Value, Duration)>>,
}

impl Recording {
    fn new(id: &str) -> Arc<Self> {
        Arc::new(Self {
            id: id.into(),
            fail_with: None,
            received: Mutex::new(Vec::new()),
        })
    }

    fn failing(id: &str, code: &'static str) -> Arc<Self> {
        Arc::new(Self {
            id: id.into(),
            fail_with: Some(code),
            received: Mutex::new(Vec::new()),
        })
    }

    fn payloads(&self) -> Vec<Value> {
        self.received
            .lock()
            .unwrap()
            .iter()
            .map(|(p, _)| p.clone())
            .collect()
    }
}

#[async_trait]
impl NewMailObserver for Recording {
    fn id(&self) -> &str {
        &self.id
    }

    async fn deliver(&self, payload: Value, timeout: Duration) -> Result<(), Error> {
        if let Some(code) = self.fail_with {
            return Err(Error::Remote {
                code: code.into(),
                message: "not sent".into(),
                stacktrace: None,
            });
        }
        self.received.lock().unwrap().push((payload, timeout));
        Ok(())
    }
}

fn binding(id: &str, function_id: &str, config: Value) -> TriggerConfig {
    TriggerConfig {
        id: id.into(),
        function_id: function_id.into(),
        config,
        metadata: None,
        namespace: None,
    }
}

fn event(account: &str, folder: &str, uid: u32) -> Event {
    Event {
        account: account.into(),
        folder: folder.into(),
        uid,
        message_id: Some(format!("<{uid}@example.com>")),
        from: Some("a@example.com".into()),
        subject: Some("hello".into()),
        snippet: Some("hi there".into()),
        ts: None,
    }
}

#[test]
fn on_new_mail_refuses_direct_invocation() {
    assert_eq!(NEW_MAIL_TRIGGER_TYPE, "email::new-mail");
    assert_eq!(ON_NEW_MAIL_FN, "email::on-new-mail");
    for payload in [
        event("support", "INBOX", 1).new_mail_event(),
        serde_json::from_value::<NewMailEvent>(
            json!({ "account": "support", "folder": "INBOX", "uid": 1 }),
        )
        .expect("the optional fields may be absent"),
    ] {
        let error = refuse_invocation(payload).unwrap_err();
        assert!(
            matches!(&error, Error::Remote { code, message, .. }
                if code == codes::FORBIDDEN && message.contains(ON_NEW_MAIL_FN)),
            "{error:?}"
        );
    }
}

/// The request / response schemas `RegisterFunction::new` extracts from a
/// sync handler, as the engine publishes them.
fn formats_of<F: IntoSyncHandler<M>, M>(_handler: &F) -> (Option<Value>, Option<Value>) {
    (F::request_format(), F::response_format())
}

fn is_typed(schema: &Value) -> bool {
    [
        "type",
        "properties",
        "$ref",
        "allOf",
        "anyOf",
        "oneOf",
        "enum",
    ]
    .iter()
    .any(|k| schema.get(k).is_some())
}

/// `email::on-new-mail` publishes the event observers receive as its
/// request schema, never an untyped (AnyValue / empty) one.
#[test]
fn on_new_mail_request_schema_is_the_typed_event() {
    let (request, response) = formats_of(&refuse_invocation);
    let request = request.expect("a request schema");
    assert!(is_typed(&request), "untyped request schema: {request}");
    let mut fields: Vec<&String> = request["properties"]
        .as_object()
        .expect("request properties")
        .keys()
        .collect();
    fields.sort();
    assert_eq!(
        fields,
        [
            "account",
            "folder",
            "from",
            "message_id",
            "snippet",
            "subject",
            "ts",
            "uid"
        ]
    );
    let mut required: Vec<&str> = request["required"]
        .as_array()
        .expect("required fields")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    required.sort();
    assert_eq!(required, ["account", "folder", "uid"]);
    let response = response.expect("a response schema");
    assert!(is_typed(&response), "untyped response schema: {response}");
}

#[test]
fn the_payload_is_the_typed_event() {
    let ev = event("support", "INBOX", 7);
    let back: NewMailEvent = serde_json::from_value(ev.payload()).expect("payload parses");
    assert_eq!(back, ev.new_mail_event());
}

#[tokio::test]
async fn an_observer_receives_the_event_a_binding_receives() {
    let registry = Arc::new(TriggerRegistry::new());
    let handler = Handler {
        registry: registry.clone(),
    };
    handler
        .register_trigger(binding(
            "trig-1",
            "h::on_mail",
            json!({ "account": "support" }),
        ))
        .await
        .unwrap();
    let observe = ObserveNewMail {
        registry: registry.clone(),
    };
    let observer = Recording::new("obs-1");
    observe
        .admit(
            &json!({ "account": "support", "handler_timeout_ms": 5000 }),
            observer.clone(),
        )
        .unwrap();
    let elsewhere = Recording::new("obs-2");
    observe
        .admit(
            &json!({ "account": "support", "folder": "Archive" }),
            elsewhere.clone(),
        )
        .unwrap();

    // The binding and the observer are filed under the same key; the binding
    // set is untouched by the observer.
    let subs = registry.subscribers_for("support", "INBOX");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].function_id, "h::on_mail");
    let observers = registry.observers_for("support", "INBOX");
    assert_eq!(observers.len(), 1);
    assert_eq!(observers[0].handler_timeout_ms, 5000);
    assert_eq!(registry.observer_count(), 2);

    // The dispatcher sends `event.payload()` to bindings and the same value
    // to observers.
    let ev = event("support", "INBOX", 42);
    let payload = ev.payload();
    deliver_to_observers(observers, &payload).await;
    assert_eq!(observer.payloads(), vec![payload.clone()]);
    assert_eq!(
        observer.received.lock().unwrap()[0].1,
        Duration::from_millis(5000)
    );
    assert_eq!(payload["uid"], 42);
    assert_eq!(payload["account"], "support");
    assert_eq!(payload["folder"], "INBOX");
    assert_eq!(payload["subject"], "hello");
    assert!(elsewhere.payloads().is_empty());
}

#[test]
fn observer_args_follow_the_binding_config() {
    let registry = Arc::new(TriggerRegistry::new());
    let observe = ObserveNewMail {
        registry: registry.clone(),
    };
    for bad in [
        json!({}),
        json!({ "account": 7 }),
        json!({ "account": "a", "handler_timeout_ms": "soon" }),
    ] {
        let rejection = observe.admit(&bad, Recording::new("bad")).unwrap_err();
        assert_eq!(rejection.code, codes::CONFIG_ERROR, "{bad}");
    }
    assert_eq!(registry.observer_count(), 0);

    // Defaults match the binding config: INBOX, 30 s.
    observe
        .admit(&json!({ "account": "a" }), Recording::new("o1"))
        .unwrap();
    let observers = registry.observers_for("a", "INBOX");
    assert_eq!(observers.len(), 1);
    assert_eq!(observers[0].handler_timeout_ms, 30_000);

    // Re-adding an id moves it; removing it leaves nothing behind.
    observe
        .admit(
            &json!({ "account": "a", "folder": "Sent" }),
            Recording::new("o1"),
        )
        .unwrap();
    assert!(registry.observers_for("a", "INBOX").is_empty());
    assert_eq!(registry.observers_for("a", "Sent").len(), 1);
    assert!(registry.remove_observer("o1"));
    assert!(!registry.remove_observer("o1"));
    assert_eq!(registry.observer_count(), 0);
}

#[tokio::test]
async fn a_skipped_or_failing_observer_does_not_stop_the_others() {
    let registry = Arc::new(TriggerRegistry::new());
    let observe = ObserveNewMail {
        registry: registry.clone(),
    };
    let ok = Recording::new("ok");
    for observer in [
        Recording::failing("skipped", "FILTERS_NOT_MATCHED"),
        Recording::failing("broken", "TARGET_FAILED"),
        ok.clone(),
    ] {
        observe
            .admit(&json!({ "account": "support" }), observer)
            .unwrap();
    }
    let payload = event("support", "INBOX", 1).payload();
    deliver_to_observers(registry.observers_for("support", "INBOX"), &payload).await;
    assert_eq!(ok.payloads(), vec![payload]);
}
