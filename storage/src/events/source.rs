//! Registers `storage::on-object-created` and `storage::on-object-deleted`
//! with their observer hooks.
//!
//! Each observer names one bucket in its args. `onObserverAdded` admits it
//! only when the bucket's notifications source is wired up (`CONFIG_ERROR`
//! otherwise), `onObserverMatch` files it under its bucket, and
//! `onObserverDeliver` calls the target of every matching observer whose
//! filters match the event, each bounded by its `handler_timeout_ms`, and
//! answers `{ acked }`: true when every target returned `null` or
//! `{ ack: true }`. The notifications source acks the upstream message on
//! `acked` and leaves it for redelivery otherwise.

use super::normalize::EventKind;
use super::object_created::{
    ObjectCreatedEvent, ON_OBJECT_CREATED_DESCRIPTION, ON_OBJECT_CREATED_FN,
};
use super::object_deleted::{
    ObjectDeletedEvent, ON_OBJECT_DELETED_DESCRIPTION, ON_OBJECT_DELETED_FN,
};
use iii_sdk::errors::Error;
use iii_sdk::{Call, CallRequest, IIIClient, Match, ObserveRejection, Observer, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Live set of buckets whose notifications source is wired. An observer of
/// a bucket missing from it is rejected instead of silently never receiving
/// anything.
pub type WiredBuckets = Arc<RwLock<HashSet<String>>>;

/// How long a target may take to ack one event when its observer does not
/// say.
pub const DEFAULT_HANDLER_TIMEOUT_MS: u64 = 60_000;

/// Added to the longest handler timeout to bound one whole call of an object
/// function.
const DELIVERY_SLACK_MS: u64 = 5_000;

/// An observer's args.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObjectArgs {
    /// Worker bucket name (a key of `buckets:` in the configuration) whose
    /// notifications source is wired up.
    pub bucket: String,
    /// How long the target may take to ack one event, in milliseconds.
    /// Default 60000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handler_timeout_ms: Option<u64>,
}

impl ObjectArgs {
    pub fn handler_timeout_ms(&self) -> u64 {
        self.handler_timeout_ms
            .unwrap_or(DEFAULT_HANDLER_TIMEOUT_MS)
    }
}

/// What a call of either function returns to its caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Delivery {
    /// Every matching target acked (or none matched).
    pub acked: bool,
}

/// The function an event of `kind` is a call of.
pub fn function_id(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Created => ON_OBJECT_CREATED_FN,
        EventKind::Deleted => ON_OBJECT_DELETED_FN,
    }
}

/// Sets the hooks of both functions by id, then registers them: private
/// (only this worker calls them), with `args_format` [`ObjectArgs`].
pub fn register(iii: &IIIClient, wired_buckets: WiredBuckets, reconfigure_gate: Arc<RwLock<()>>) {
    for id in [ON_OBJECT_CREATED_FN, ON_OBJECT_DELETED_FN] {
        let (wired, gate) = (wired_buckets.clone(), reconfigure_gate.clone());
        iii.on_observer_added_for(id, move |obs: Observer| {
            let (wired, gate) = (wired.clone(), gate.clone());
            async move { admit(&obs, &wired, &gate).await }
        });
        iii.on_observer_match_for(
            id,
            Match::keys(
                |obs| obs.args_value()["bucket"].as_str().into(),
                |call| call.input["bucket"].as_str().into(),
            ),
        );
        let engine = iii.clone();
        iii.on_observer_deliver_for(id, move |call: Call, observers: Vec<Observer>| {
            let engine = engine.clone();
            async move { Ok::<Value, Error>(deliver(&engine, &call, observers).await) }
        });
    }
    iii.register_function(
        ON_OBJECT_CREATED_FN,
        RegisterFunction::new(|event: ObjectCreatedEvent| Ok::<_, Error>(event))
            .description(ON_OBJECT_CREATED_DESCRIPTION)
            .args_format::<ObjectArgs>()
            .private(),
    );
    iii.register_function(
        ON_OBJECT_DELETED_FN,
        RegisterFunction::new(|event: ObjectDeletedEvent| Ok::<_, Error>(event))
            .description(ON_OBJECT_DELETED_DESCRIPTION)
            .args_format::<ObjectArgs>()
            .private(),
    );
}

/// `onObserverAdded`: the bucket must have a wired notifications source. The
/// reconfigure gate keeps a configuration publish from changing the wired
/// set mid-check.
async fn admit(
    obs: &Observer,
    wired: &WiredBuckets,
    gate: &RwLock<()>,
) -> Result<(), ObserveRejection> {
    let args: ObjectArgs = obs.args()?;
    let _gate = gate.read().await;
    if !wired.read().await.contains(&args.bucket) {
        return Err(ObserveRejection::new(
            "CONFIG_ERROR",
            format!(
                "bucket `{}` has no notifications source configured; add `notifications:` under the bucket in worker config",
                args.bucket
            ),
        ));
    }
    tracing::info!(
        observer = obs.id(),
        function = obs.function_id(),
        bucket = %args.bucket,
        "object event observer added"
    );
    Ok(())
}

/// `onObserverDeliver`: calls the target of every matching observer whose
/// filters match, concurrently, and answers whether all of them acked.
async fn deliver(iii: &IIIClient, call: &Call, observers: Vec<Observer>) -> Value {
    let deliveries = observers
        .into_iter()
        .filter(|obs| obs.matches(call))
        .map(|obs| {
            let payload = call.input.clone();
            async move { deliver_to(iii, &obs, payload).await }
        });
    let acked = futures_util::future::join_all(deliveries)
        .await
        .into_iter()
        .all(|acked| acked);
    serde_json::to_value(Delivery { acked }).unwrap_or(Value::Null)
}

async fn deliver_to(iii: &IIIClient, obs: &Observer, payload: Value) -> bool {
    let timeout_ms = obs
        .args::<ObjectArgs>()
        .map(|args| args.handler_timeout_ms())
        .unwrap_or(DEFAULT_HANDLER_TIMEOUT_MS);
    let request = CallRequest::observer(obs, payload).timeout(Duration::from_millis(timeout_ms));
    // The engine enforces the same timeout; the local bound protects against
    // a stuck connection.
    let bound = Duration::from_millis(timeout_ms.saturating_add(1_000));
    match tokio::time::timeout(bound, iii.call(request)).await {
        Ok(Ok(value)) => is_ack(&value),
        Ok(Err(error)) => {
            tracing::warn!(
                observer = obs.id(),
                target = obs.target(),
                error = %error,
                "object event target failed; not acking"
            );
            false
        }
        Err(_) => {
            tracing::warn!(
                observer = obs.id(),
                target = obs.target(),
                timeout_ms,
                "object event target timed out; not acking"
            );
            false
        }
    }
}

/// A target acks with `null` (a function that returns nothing) or anything
/// without `ack: false`; `{ ack: false }` asks for redelivery.
pub fn is_ack(value: &Value) -> bool {
    value.is_null() || value.get("ack").and_then(Value::as_bool).unwrap_or(true)
}

/// How long one call of `function_id` for `bucket` may take: the longest
/// handler timeout among the bucket's observers (their targets run
/// concurrently), plus slack.
pub fn delivery_timeout_ms(iii: &IIIClient, function_id: &str, bucket: &str) -> u64 {
    iii.observers(function_id)
        .iter()
        .filter_map(|obs| obs.args::<ObjectArgs>().ok())
        .filter(|args| args.bucket == bucket)
        .map(|args| args.handler_timeout_ms())
        .max()
        .unwrap_or(DEFAULT_HANDLER_TIMEOUT_MS)
        .saturating_add(DELIVERY_SLACK_MS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn args_default_the_handler_timeout_and_refuse_unknown_fields() {
        let args: ObjectArgs = serde_json::from_value(json!({ "bucket": "uploads" })).unwrap();
        assert_eq!(args.handler_timeout_ms(), DEFAULT_HANDLER_TIMEOUT_MS);
        let args: ObjectArgs =
            serde_json::from_value(json!({ "bucket": "uploads", "handler_timeout_ms": 5000 }))
                .unwrap();
        assert_eq!(args.handler_timeout_ms(), 5_000);
        assert!(serde_json::from_value::<ObjectArgs>(
            json!({ "bucket": "uploads", "bucket_name": "x" })
        )
        .is_err());
        assert!(serde_json::from_value::<ObjectArgs>(json!({})).is_err());
    }

    #[test]
    fn null_and_unflagged_results_ack() {
        assert!(is_ack(&Value::Null));
        assert!(is_ack(&json!({})));
        assert!(is_ack(&json!({ "ack": true })));
        assert!(is_ack(&json!("done")));
        assert!(!is_ack(&json!({ "ack": false })));
    }

    #[test]
    fn each_kind_is_its_own_function() {
        assert_eq!(
            function_id(EventKind::Created),
            "storage::on-object-created"
        );
        assert_eq!(
            function_id(EventKind::Deleted),
            "storage::on-object-deleted"
        );
    }
}
