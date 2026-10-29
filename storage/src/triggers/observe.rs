//! `storage::on-object-created` and `storage::on-object-deleted`: observable
//! functions next to the `storage::object-created` / `storage::object-deleted`
//! trigger types (iii 0.25). Each observer is filed in the same
//! [`TriggerRegistry`] the bindings live in, so one fan-out serves both and
//! an observer receives exactly the payload a binding receives (`obs.call`).

use crate::handlers::schema_of;
use crate::triggers::handler::WiredBuckets;
use crate::triggers::normalize::EventKind;
use crate::triggers::object_created::TriggerConfig as CreatedConfig;
use crate::triggers::object_deleted::TriggerConfig as DeletedConfig;
use crate::triggers::registry::{ObjectObserver, TriggerRegistry};
use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::observe_codes;
use iii_sdk::{ObserveHandler, ObserveRejection, Observer};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

pub const ON_OBJECT_CREATED: &str = "storage::on-object-created";
pub const ON_OBJECT_DELETED: &str = "storage::on-object-deleted";

pub const ON_OBJECT_CREATED_DESCRIPTION: &str =
    "Fires when an object is written to a configured bucket whose notifications source is wired \
     up. Observe it with args { bucket, event_types?, handler_timeout_ms? } (the \
     storage::object-created binding config); every observer of that bucket receives \
     { bucket, key, size, etag, content_type, created_at, raw_event_id }. A target may return \
     { ack: false } to have the upstream message redelivered. Only the storage worker fires it: \
     invoking it directly is refused.";

pub const ON_OBJECT_DELETED_DESCRIPTION: &str =
    "Fires when an object is removed from a configured bucket whose notifications source is \
     wired up. Observe it with args { bucket, handler_timeout_ms? } (the storage::object-deleted \
     binding config); every observer of that bucket receives { bucket, key, deleted_at, \
     raw_event_id }. A target may return { ack: false } to have the upstream message \
     redelivered. Only the storage worker fires it: invoking it directly is refused.";

/// The observable function id for `kind`.
pub fn function_id(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Created => ON_OBJECT_CREATED,
        EventKind::Deleted => ON_OBJECT_DELETED,
    }
}

#[async_trait]
impl ObjectObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    async fn deliver(&self, payload: &Value, timeout: Duration) -> Result<Value, Error> {
        self.call_with_timeout::<Value>(payload, timeout).await
    }
}

/// Parses observer args exactly as the trigger type parses a binding config
/// for `kind`: returns the bucket and the handler timeout.
pub fn parse_args(kind: EventKind, args: &Value) -> Result<(String, u64), ObserveRejection> {
    let invalid = |e: serde_json::Error| {
        ObserveRejection::new(
            observe_codes::INVALID_ARGS,
            format!("{}: {e}", function_id(kind)),
        )
    };
    match kind {
        EventKind::Created => {
            let cfg: CreatedConfig = serde_json::from_value(args.clone()).map_err(invalid)?;
            Ok((cfg.bucket, cfg.handler_timeout_ms))
        }
        EventKind::Deleted => {
            let cfg: DeletedConfig = serde_json::from_value(args.clone()).map_err(invalid)?;
            Ok((cfg.bucket, cfg.handler_timeout_ms))
        }
    }
}

/// The `ObserveHandler` of one `storage::on-object-*` function: files each
/// observer in the shared registry under its bucket and kind, and takes it
/// out when the engine removes it. A bucket without a wired notifications
/// source is refused, as the trigger type refuses such a binding.
pub struct OnObjectHandler {
    pub kind: EventKind,
    pub registry: Arc<TriggerRegistry>,
    pub wired_buckets: WiredBuckets,
    pub reconfigure_gate: Arc<RwLock<()>>,
}

impl OnObjectHandler {
    /// Files `observer` (with `args`) in the registry, or refuses it.
    pub async fn add(
        &self,
        observer: Arc<dyn ObjectObserver>,
        args: &Value,
    ) -> Result<(), ObserveRejection> {
        let _gate = self.reconfigure_gate.read().await;
        let (bucket, handler_timeout_ms) = parse_args(self.kind, args)?;
        if !self.wired_buckets.read().await.contains(&bucket) {
            return Err(ObserveRejection::new(
                observe_codes::INVALID_ARGS,
                format!(
                    "bucket `{bucket}` has no notifications source configured; add `notifications:` under the bucket in worker config"
                ),
            ));
        }
        self.registry
            .add_observer(bucket, self.kind, observer, handler_timeout_ms);
        Ok(())
    }

    pub fn remove(&self, id: &str) {
        self.registry.remove_observer(id);
    }
}

#[async_trait]
impl ObserveHandler for OnObjectHandler {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        // Checked by the engine against args_format; parsed again here so a
        // config the trigger type would refuse is refused the same way.
        self.add(Arc::new(obs.clone()), obs.args_value()).await?;
        tracing::info!(
            observer = %obs.id(),
            target = %obs.target(),
            "{} observer added",
            function_id(self.kind)
        );
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.remove(obs.id());
        tracing::info!(observer = %obs.id(), "{} observer removed", function_id(self.kind));
    }
}

/// The request schema `kind`'s observable function registers: the event
/// payload its observers receive, never `AnyValue`.
pub fn request_schema(kind: EventKind) -> Value {
    match kind {
        EventKind::Created => schema_of::<crate::triggers::object_created::Event>(),
        EventKind::Deleted => schema_of::<crate::triggers::object_deleted::Event>(),
    }
}

/// The response schema of every `storage::on-object-*` function: its
/// handler refuses every invocation and returns `()` on success.
pub fn response_schema() -> Value {
    schema_of::<()>()
}

/// The handler of a `storage::on-object-*` function: events only come from
/// this worker's own fan-out, so every invocation is refused with FORBIDDEN,
/// whatever its input.
pub fn refuse_invocation(kind: EventKind) -> Error {
    let id = function_id(kind);
    Error::Remote {
        code: observe_codes::FORBIDDEN.to_string(),
        message: format!(
            "{id} is an event source: observe it \
             (iii.observe({{ function_id: '{id}', args: {{ bucket }} }}, target)); \
             only the storage worker fires its events"
        ),
        stacktrace: None,
    }
}
