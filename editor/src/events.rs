//! `editor::changed` — the push channel that replaced polling.
//!
//! The page used to ask `editor::git::status` every three seconds. That was
//! wrong on every axis: it cost a git invocation per tick whether or not
//! anything had happened, it could not see a change made outside a git
//! repository, and it reported the settled state rather than the edit. The
//! platform already has push primitives, so this worker registers a trigger
//! type and fans out to whoever subscribed, exactly as the `github` worker
//! does for `github::called`.
//!
//! Subscribers are whatever bound the trigger — normally the console page, one
//! binding per open tab, GC'd when the tab closes. Emission is best-effort: a
//! slow or absent subscriber must never delay or fail the write that produced
//! the event.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType, TriggerAction};
use schemars::JsonSchema;
use serde::Serialize;

/// The trigger type a surface binds to watch the workspace change.
pub const CHANGED: &str = "editor::changed";

/// Previews are bounded: an event is a notification, not a file transfer. A
/// subscriber that wants the whole patch asks `editor::git::hunks` for it.
const MAX_PATCH_BYTES: usize = 16 * 1024;

/// What changed, and enough about it to render a row without a round trip.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ChangedEvent {
    /// Path relative to the workspace root.
    pub path: String,
    /// The function that caused it, e.g. `shell::fs::write`, so a surface can
    /// say who did what.
    pub cause: String,
    /// `created`, `modified`, `deleted`, or `unknown` when the cause does not
    /// say.
    pub kind: String,
    /// Lines added and removed against the file's previous content, when it
    /// could be computed.
    pub added: u32,
    pub removed: u32,
    /// Unified patch, truncated to `MAX_PATCH_BYTES`. Empty when there was no
    /// previous content to compare against.
    pub patch: String,
    /// True when `patch` was cut short.
    pub truncated: bool,
    /// Workspace root the path is relative to, so a surface that follows the
    /// agent can notice the root moved.
    pub root: String,
}

type Subscribers = Arc<Mutex<HashMap<String, String>>>;

/// Bound trigger ids to the function they want invoked.
#[derive(Clone, Default)]
pub struct SubscriberSet {
    inner: Subscribers,
}

impl SubscriberSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().map(|s| s.is_empty()).unwrap_or(true)
    }

    pub fn function_ids(&self) -> Vec<String> {
        self.inner
            .lock()
            .map(|s| s.values().cloned().collect())
            .unwrap_or_default()
    }

    fn add(&self, trigger_id: String, function_id: String) {
        if let Ok(mut s) = self.inner.lock() {
            s.insert(trigger_id, function_id);
        }
    }

    fn remove(&self, trigger_id: &str) {
        if let Ok(mut s) = self.inner.lock() {
            s.remove(trigger_id);
        }
    }
}

struct ChangedTriggerHandler {
    subscribers: SubscriberSet,
}

#[async_trait]
impl TriggerHandler for ChangedTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::info!(
            trigger_type = CHANGED,
            id = %config.id,
            function_id = %config.function_id,
            "change subscription registered"
        );
        self.subscribers.add(config.id, config.function_id);
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::info!(trigger_type = CHANGED, id = %config.id, "change subscription unregistered");
        self.subscribers.remove(&config.id);
        Ok(())
    }
}

/// A latch that is `true` the first time and never again — for a condition
/// worth one loud line and no more.
#[derive(Clone, Default)]
struct Once(Arc<AtomicBool>);

impl Once {
    fn first(&self) -> bool {
        !self.0.swap(true, Ordering::Relaxed)
    }
}

/// Fans `editor::changed` out to every current subscriber.
#[derive(Clone)]
pub struct ChangedEmitter {
    iii: Arc<IIIClient>,
    subscribers: SubscriberSet,
    /// Whether the "this event went nowhere" line has already been said.
    unsubscribed: Once,
}

impl ChangedEmitter {
    pub fn new(iii: Arc<IIIClient>, subscribers: SubscriberSet) -> Self {
        Self {
            iii,
            subscribers,
            unsubscribed: Once::default(),
        }
    }

    pub fn has_subscribers(&self) -> bool {
        !self.subscribers.is_empty()
    }

    /// Fire and forget. Delivery failures are logged and swallowed: the write
    /// that produced this event has already happened, and failing it because a
    /// browser tab went away would be absurd.
    pub async fn emit(&self, mut event: ChangedEvent) {
        let targets = self.subscribers.function_ids();
        if targets.is_empty() {
            // Loud once, then quiet. This is the only place a failed trigger
            // type registration is observable at all (see
            // `register_changed_trigger`), and "nobody has bound it yet" and
            // "nobody ever can" look identical from here — so it has to be
            // visible without a debug filter, and it must not log a line per
            // write for a workspace nobody is watching.
            if self.unsubscribed.first() {
                tracing::warn!(
                    path = %event.path,
                    trigger_type = CHANGED,
                    "editor::changed fired with nothing bound to it, so push updates are going \
                     nowhere. Either no surface has subscribed yet, or the trigger type never \
                     reached the engine."
                );
            } else {
                tracing::debug!(path = %event.path, "editor::changed: nobody subscribed, dropping");
            }
            return;
        }
        if event.patch.len() > MAX_PATCH_BYTES {
            let mut cut = MAX_PATCH_BYTES;
            while cut > 0 && !event.patch.is_char_boundary(cut) {
                cut -= 1;
            }
            event.patch.truncate(cut);
            event.truncated = true;
        }
        let payload = match serde_json::to_value(&event) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "editor::changed payload failed to serialize");
                return;
            }
        };
        for function_id in targets {
            if let Err(e) = self
                .iii
                .trigger(TriggerRequest {
                    function_id: function_id.clone(),
                    payload: payload.clone(),
                    action: Some(TriggerAction::Void),
                    timeout_ms: None,
                })
                .await
            {
                tracing::warn!(function_id = %function_id, error = %e, "editor::changed fan-out failed");
            } else {
                tracing::info!(
                    function_id = %function_id,
                    path = %event.path,
                    kind = %event.kind,
                    added = event.added,
                    removed = event.removed,
                    "editor::changed delivered"
                );
            }
        }
    }
}

/// Register the trigger type and return the emitter wired to its subscriber
/// set. Call before registering functions so the emitter can be threaded in.
pub fn register_changed_trigger(iii: &Arc<IIIClient>) -> ChangedEmitter {
    let subscribers = SubscriberSet::new();
    // `register_trigger_type` hands back a typed handle, not a `Result`: the SDK
    // queues the registration message and drops the send result on the floor,
    // so there is nothing here to check and nothing to retry. The log therefore
    // claims only what actually happened — the message was sent — because
    // "registered" would be a claim this worker cannot make.
    //
    // The failure it cannot see is a real one: without the type, every
    // `editor::changed` is undeliverable and the whole push surface is dead. The
    // loud signal for that lives in `ChangedEmitter::emit`, which warns the
    // first time an event has nowhere to go.
    let _handle = iii.register_trigger_type(RegisterTriggerType::new(
        CHANGED,
        "Fires when a file in the workspace changes, whoever changed it — \
         including an agent that never called this worker.",
        ChangedTriggerHandler {
            subscribers: subscribers.clone(),
        },
    ));
    tracing::info!(
        trigger_type = CHANGED,
        "sent the trigger type registration; delivery is confirmed by the first subscription"
    );
    ChangedEmitter::new(iii.clone(), subscribers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event() -> ChangedEvent {
        ChangedEvent {
            path: "a.rs".into(),
            cause: "shell::fs::write".into(),
            kind: "modified".into(),
            added: 1,
            removed: 0,
            patch: String::new(),
            truncated: false,
            root: "/repo".into(),
        }
    }

    #[test]
    fn an_empty_set_reports_empty() {
        assert!(SubscriberSet::new().is_empty());
    }

    #[test]
    fn register_then_unregister_tracks_the_binding() {
        let s = SubscriberSet::new();
        s.add("t1".into(), "iii::editor-ui::events::abc".into());
        assert!(!s.is_empty());
        assert_eq!(s.function_ids(), vec!["iii::editor-ui::events::abc"]);
        s.remove("t1");
        assert!(s.is_empty());
    }

    /// Two tabs bind separately and must both be delivered to.
    #[test]
    fn distinct_triggers_are_distinct_subscribers() {
        let s = SubscriberSet::new();
        s.add("t1".into(), "fn::a".into());
        s.add("t2".into(), "fn::b".into());
        let mut ids = s.function_ids();
        ids.sort();
        assert_eq!(ids, vec!["fn::a", "fn::b"]);
    }

    #[test]
    fn re_registering_the_same_trigger_does_not_duplicate() {
        let s = SubscriberSet::new();
        s.add("t1".into(), "fn::a".into());
        s.add("t1".into(), "fn::a".into());
        assert_eq!(s.function_ids().len(), 1);
    }

    /// The undeliverable-event warning has to be said once, not once per write:
    /// a workspace nobody is watching would otherwise log a line per edit.
    #[test]
    fn the_first_undeliverable_event_is_the_only_loud_one() {
        let once = Once::default();
        assert!(once.first(), "the first drop is worth a warning");
        assert!(!once.first());
        assert!(!once.first());
        // The latch is shared, not copied: a cloned emitter must not get a
        // second chance to shout.
        assert!(!once.clone().first());
    }

    #[test]
    fn the_event_serializes_with_every_field() {
        let v = serde_json::to_value(event()).unwrap();
        for key in [
            "path",
            "cause",
            "kind",
            "added",
            "removed",
            "patch",
            "truncated",
            "root",
        ] {
            assert!(v.get(key).is_some(), "{key} missing from the wire event");
        }
    }
}
