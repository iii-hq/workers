//! `tmux::on-changed` — the live feed behind the console page.
//!
//! A private function whose handler returns the [`Overview`] it is called
//! with: its observers observe the overview. Each observer receives the full
//! overview right after it is accepted, and again whenever anything in it
//! changes. The poll loop only runs while the function is observed, and
//! mutations through this worker poke it so their effect shows at once.
//!
//! Calls are fire-and-forget (`CallAction::Void`): iii delivers each one to
//! the observers whose filters match, and the engine applies each observer's
//! metadata (the harness's `__binding`) and target namespace. The overview a
//! new observer receives first is a call of the function with metadata
//! `{ for_observer: <observer id> }`, which the match hook gives to that
//! observer alone; iii delivers it like every other call, so a target
//! receives every overview in the shape its observer's `deliver` names.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::{Call, CallAction, CallRequest, IIIClient, Match, Observer, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::{Mutex, Notify};

use crate::collector::Collector;
use crate::model::Overview;

pub const ON_CHANGED_FN: &str = "tmux::on-changed";

/// The poll loop that calls [`ON_CHANGED_FN`] while it is observed.
pub struct Monitor {
    iii: IIIClient,
    collector: Arc<Collector>,
    wake: Notify,
    /// The fingerprint of the overview the observers last received, `None`
    /// while nothing observes the function. Shared by the poll loop and the
    /// first overview a new observer receives, so a state an observer
    /// already has is not sent again.
    last: Mutex<Option<Value>>,
}

/// The observer a call of [`ON_CHANGED_FN`] is for: its metadata's
/// `for_observer`. `None` for a call every observer receives.
fn for_observer(call: &Call) -> Option<&str> {
    call.metadata
        .as_ref()
        .and_then(|metadata| metadata["for_observer"].as_str())
}

/// The match hook of [`ON_CHANGED_FN`]: a call for one observer goes to it
/// alone, every other call to every observer.
fn wants(observer_id: &str, call: &Call) -> bool {
    for_observer(call).is_none_or(|id| id == observer_id)
}

/// The overview without its timestamps, so an unchanged state is not re-sent.
fn fingerprint(overview: &Overview) -> Option<Value> {
    let mut value = serde_json::to_value(overview).ok()?;
    value["sessions"]["generated_at_ms"] = Value::Null;
    value["ports"]["generated_at_ms"] = Value::Null;
    Some(value)
}

impl Monitor {
    pub fn new(iii: IIIClient, collector: Arc<Collector>) -> Arc<Self> {
        Arc::new(Self {
            iii,
            collector,
            wake: Notify::new(),
            last: Mutex::new(None),
        })
    }

    /// Re-collect now (after a mutation) instead of on the next tick.
    pub fn poke(&self) {
        self.wake.notify_one();
    }

    fn observed(&self) -> bool {
        self.iii.is_observed(ON_CHANGED_FN)
    }

    /// One call of [`ON_CHANGED_FN`]; iii delivers it to every observer.
    async fn emit(&self, payload: Value) {
        let request = CallRequest {
            function_id: ON_CHANGED_FN.to_string(),
            payload,
            action: Some(CallAction::Void),
            timeout_ms: None,
        };
        if let Err(error) = self.iii.call(request).await {
            tracing::debug!(%error, "tmux::on-changed call failed");
        }
    }

    /// The current overview, to `observer` alone, once it is active: a call
    /// of [`ON_CHANGED_FN`] with metadata `{ for_observer }`, which the match
    /// hook gives to that observer only. A rejected observer gets nothing.
    /// When it is the first observer since the function was idle, the
    /// overview it received is what the poll loop compares the next one with.
    async fn send_current(&self, observer: Observer) {
        if let Err(error) = observer.accepted().await {
            tracing::debug!(observer_id = %observer.id(), %error, "observer not accepted");
            return;
        }
        {
            let mut last = self.last.lock().await;
            let overview = self.collector.overview().await;
            if let Ok(payload) = serde_json::to_value(&overview) {
                let request = CallRequest::new(ON_CHANGED_FN, payload)
                    .metadata(json!({ "for_observer": observer.id() }))
                    .action(CallAction::Void);
                match self.iii.call(request).await {
                    Ok(_) if last.is_none() => *last = fingerprint(&overview),
                    Ok(_) => {}
                    Err(error) => {
                        tracing::debug!(observer_id = %observer.id(), %error, "first overview not delivered");
                    }
                }
            }
        }
        self.poke();
    }

    /// The poll loop. Sleeps while nothing observes [`ON_CHANGED_FN`].
    pub fn spawn(self: &Arc<Self>) {
        let monitor = self.clone();
        tokio::spawn(async move {
            loop {
                let wake = monitor.wake.notified();
                tokio::pin!(wake);
                wake.as_mut().enable();
                {
                    let mut last = monitor.last.lock().await;
                    if !monitor.observed() {
                        *last = None;
                        drop(last);
                        wake.await;
                        continue;
                    }
                    let overview = monitor.collector.overview().await;
                    let print = fingerprint(&overview);
                    if print.is_some() && print != *last {
                        if let Ok(payload) = serde_json::to_value(&overview) {
                            monitor.emit(payload).await;
                        }
                        *last = print;
                    }
                }
                let interval = Duration::from_millis(monitor.collector.config().poll_interval_ms);
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {}
                    _ = wake => {}
                }
            }
        });
    }
}

/// Registers [`ON_CHANGED_FN`]. Its added hook sends the new observer the
/// current overview and wakes the poll loop; its match hook gives that
/// overview to the new observer alone.
pub fn register(iii: &IIIClient, monitor: Arc<Monitor>) {
    iii.on_observer_added_for(ON_CHANGED_FN, move |observer: Observer| {
        let monitor = monitor.clone();
        async move {
            tracing::debug!(observer_id = %observer.id(), "live feed observed");
            // The observer becomes active once this hook returns; the task
            // waits for that before calling the function for it.
            tokio::spawn(async move { monitor.send_current(observer).await });
            Ok(())
        }
    });
    iii.on_observer_match_for(
        ON_CHANGED_FN,
        Match::predicate(|observer, call| wants(observer.id(), call)),
    );
    iii.register_function(
        ON_CHANGED_FN,
        RegisterFunction::new(|overview: Overview| Ok::<_, Error>(overview))
            .description(
                "Live tmux overview: the full state (sessions, windows, panes with agent/compose/port \
                 status, compose stacks and listening ports), sent to each observer when it is added \
                 and called again whenever it changes. Takes no args.",
            )
            .private(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PortsSnapshot, SessionsSnapshot, TmuxInfo};

    fn overview(at: u64) -> Overview {
        Overview {
            sessions: SessionsSnapshot {
                tmux: TmuxInfo::default(),
                sessions: vec![],
                generated_at_ms: at,
            },
            ports: PortsSnapshot {
                supported: true,
                error: None,
                composes: vec![],
                listeners: vec![],
                generated_at_ms: at,
            },
        }
    }

    #[test]
    fn a_call_for_one_observer_goes_to_it_alone() {
        let ordinary = Call::default();
        assert_eq!(for_observer(&ordinary), None);
        assert!(wants("page", &ordinary) && wants("other", &ordinary));
        let for_page = Call {
            metadata: Some(json!({ "for_observer": "page" })),
            ..Call::default()
        };
        assert_eq!(for_observer(&for_page), Some("page"));
        assert!(wants("page", &for_page));
        assert!(!wants("other", &for_page));
        let other_metadata = Call {
            metadata: Some(json!({ "__binding": "b1" })),
            ..Call::default()
        };
        assert!(wants("page", &other_metadata));
    }

    #[test]
    fn timestamps_alone_are_not_a_change() {
        assert_eq!(fingerprint(&overview(1)), fingerprint(&overview(2)));
        let mut changed = overview(1);
        changed.ports.supported = false;
        assert_ne!(fingerprint(&overview(1)), fingerprint(&changed));
    }
}
