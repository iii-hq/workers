//! `sentinel::on-group-changed` and `sentinel::on-investigation-changed` —
//! the surface siblings and the console watch.
//!
//! Each is a private function whose handler returns the event it is called
//! with: only this worker calls it, anyone may observe it, and observers
//! select calls with filters on its input (`input.group_id`, `input.op`).
//! The worker calls them only while something observes them.
//!
//! The event is a doorbell, not a record: an observer re-reads the group it
//! names rather than trusting the payload, which is what keeps a page correct
//! when two events cross. Occurrence events are coalesced per group over a
//! second, because an incident with a thousand occurrences must not become a
//! thousand deliveries — while a state change goes out immediately, since
//! that is the one an alert cares about.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::{CallAction, CallRequest, IIIClient, RegisterFunction};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::{GroupChangedEventV1, GroupChangedOpV1, GroupStatusV1, InvestigationChangedEventV1};

pub const ON_GROUP_CHANGED_FN: &str = "sentinel::on-group-changed";
pub const ON_GROUP_CHANGED_DESC: &str = "Called when an error group is created, takes a new \
     occurrence, or changes state. No args; filter on input.group_id or input.op (created, \
     occurrence or status). Occurrence events are coalesced per group over one second; re-read \
     the group rather than trusting the event.";
pub const ON_INVESTIGATION_CHANGED_FN: &str = "sentinel::on-investigation-changed";
pub const ON_INVESTIGATION_CHANGED_DESC: &str = "Called when an investigation is created, records \
     a diagnosis, or finishes. No args; filter on input.group_id, input.investigation_id or \
     input.op.";

/// How long occurrence events for one group are gathered before delivery.
const COALESCE: Duration = Duration::from_secs(1);

/// Registers both event functions: private, no args, each handler returning
/// the event it is called with.
pub fn register_functions(iii: &IIIClient) {
    iii.register_function(
        ON_GROUP_CHANGED_FN,
        RegisterFunction::new(|event: GroupChangedEventV1| Ok::<_, Error>(event))
            .description(ON_GROUP_CHANGED_DESC)
            .private(),
    );
    iii.register_function(
        ON_INVESTIGATION_CHANGED_FN,
        RegisterFunction::new(|event: InvestigationChangedEventV1| Ok::<_, Error>(event))
            .description(ON_INVESTIGATION_CHANGED_DESC)
            .private(),
    );
}

/// Calls the event functions, coalescing the noisy kind.
pub struct Emitter {
    iii: Arc<IIIClient>,
    /// Occurrence events waiting out their coalescing window, newest payload
    /// per group.
    pending: Arc<Mutex<HashMap<String, GroupChangedEventV1>>>,
}

impl Emitter {
    pub fn new(iii: Arc<IIIClient>) -> Arc<Self> {
        let emitter = Arc::new(Self {
            iii,
            pending: Arc::new(Mutex::new(HashMap::new())),
        });
        let flusher = emitter.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(COALESCE);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                flusher.flush().await;
            }
        });
        emitter
    }

    /// Send a group event, coalescing the noisy kind. Nothing is gathered or
    /// sent while nobody observes the function.
    pub async fn group_changed(&self, event: GroupChangedEventV1) {
        if !self.iii.is_observed(ON_GROUP_CHANGED_FN) {
            return;
        }
        if event.op == GroupChangedOpV1::Occurrence {
            self.pending
                .lock()
                .await
                .insert(event.group_id.clone(), event);
            return;
        }
        // A creation or a state change supersedes anything gathered for that
        // group: the later event carries the newer count anyway.
        self.pending.lock().await.remove(&event.group_id);
        self.call(ON_GROUP_CHANGED_FN, &event).await;
    }

    pub async fn investigation_changed(&self, event: InvestigationChangedEventV1) {
        if !self.iii.is_observed(ON_INVESTIGATION_CHANGED_FN) {
            return;
        }
        self.call(ON_INVESTIGATION_CHANGED_FN, &event).await;
    }

    async fn flush(&self) {
        let ready: Vec<GroupChangedEventV1> = {
            let mut pending = self.pending.lock().await;
            pending.drain().map(|(_, event)| event).collect()
        };
        if ready.is_empty() || !self.iii.is_observed(ON_GROUP_CHANGED_FN) {
            return;
        }
        for event in ready {
            self.call(ON_GROUP_CHANGED_FN, &event).await;
        }
    }

    async fn call<T: Serialize>(&self, function_id: &'static str, event: &T) {
        let Ok(payload) = serde_json::to_value(event) else {
            return;
        };
        let request = CallRequest {
            function_id: function_id.to_string(),
            payload,
            // Fire and forget: an observer that is slow or broken must not
            // hold up the pipeline that noticed the error.
            action: Some(CallAction::Void),
            timeout_ms: None,
        };
        if let Err(error) = self.iii.call(request).await {
            tracing::warn!(function_id, %error, "sentinel event call failed");
        }
    }
}

/// Build the event for a group the ingest just touched.
pub fn group_event(
    op: GroupChangedOpV1,
    group_id: &str,
    status: GroupStatusV1,
    occurrence_count: u64,
    reason: Option<crate::GroupChangeReasonV1>,
) -> GroupChangedEventV1 {
    GroupChangedEventV1 {
        op,
        group_id: group_id.to_string(),
        status,
        previous_status: None,
        occurrence_count,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `IIIClient::new` builds local state only: no engine, no observers.
    fn offline() -> Arc<IIIClient> {
        let iii = Arc::new(IIIClient::new("ws://127.0.0.1:1"));
        register_functions(&iii);
        iii
    }

    #[tokio::test]
    async fn nothing_is_gathered_while_nobody_observes() {
        let iii = offline();
        let emitter = Emitter::new(iii.clone());
        assert!(!iii.is_observed(ON_GROUP_CHANGED_FN));

        emitter
            .group_changed(group_event(
                GroupChangedOpV1::Occurrence,
                "grp_1",
                GroupStatusV1::New,
                3,
                None,
            ))
            .await;
        assert!(emitter.pending.lock().await.is_empty());
    }

    #[test]
    fn the_group_event_is_the_documented_doorbell() {
        let event = group_event(
            GroupChangedOpV1::Created,
            "grp_1",
            GroupStatusV1::New,
            1,
            None,
        );
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            json!({
                "op": "created",
                "group_id": "grp_1",
                "status": "new",
                "occurrence_count": 1,
            })
        );
    }

    #[test]
    fn descriptions_name_the_filterable_fields() {
        assert!(ON_GROUP_CHANGED_DESC.contains("input.group_id"));
        assert!(ON_GROUP_CHANGED_DESC.contains("input.op"));
        assert!(ON_INVESTIGATION_CHANGED_DESC.contains("input.group_id"));
    }
}
