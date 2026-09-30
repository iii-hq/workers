//! The four trigger types this worker provides.
//!
//! - `ios-simulator::device-changed` — a simulator appeared, disappeared or
//!   changed state (booted, shut down, renamed), whoever caused it: this
//!   worker, Xcode, or `simctl` in a terminal.
//! - `ios-simulator::media-changed` — a screenshot or recording was saved or
//!   deleted.
//! - `ios-simulator::frame-event` — one live-view frame of a watched
//!   simulator, delivered straight to the bindings (as `browser::frame-event`
//!   does): the live view depends on no other worker.
//! - `ios-simulator::activity` — a caller drove a simulator (a gesture, a
//!   button, typing, an app, a screenshot, `ios-simulator::open`), so live
//!   previews can surface it while an agent works.
//!
//! Config `{ tenant?, udid? }` narrows a binding. A binding without `tenant`
//! sees every tenant (the operator's console); a shared deployment's
//! `rbac-proxy` trigger-registration hook stamps `tenant` on bindings made by
//! tenants. Delivery is fire-and-forget in the binding's namespace.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{TriggerAction, TriggerRequest, TriggerRequestWithMetadata};
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const DEVICE_CHANGED: &str = "ios-simulator::device-changed";
pub const MEDIA_CHANGED: &str = "ios-simulator::media-changed";
pub const FRAME_EVENT: &str = "ios-simulator::frame-event";
pub const ACTIVITY: &str = "ios-simulator::activity";

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingConfig {
    /// Only this tenant's events. Omit for every tenant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
    /// Only this simulator's events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeviceChangedEvent {
    pub tenant: String,
    pub udid: String,
    pub name: String,
    /// The state now; `Deleted` when the simulator is gone.
    pub state: String,
    /// The state before this change; empty when the simulator is new.
    pub previous_state: String,
    /// `added`, `removed` or `updated`.
    pub change: String,
    /// False when the boot came from a surface already showing the
    /// simulator (the console page), so live previews skip it.
    pub preview: bool,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MediaChangedEvent {
    pub tenant: String,
    pub udid: String,
    /// Media name, `<udid>/<file>`.
    pub name: String,
    /// `screenshot` or `recording`.
    pub kind: String,
    /// `added` or `removed`.
    pub change: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FrameEvent {
    pub tenant: String,
    pub udid: String,
    /// Base64 JPEG.
    pub data: String,
    /// Frame pixels.
    pub width: u32,
    pub height: u32,
    /// Device pixels: the coordinate space of touch.
    pub device_width: u32,
    pub device_height: u32,
    /// Monotonic per bridge; drop anything not newer than what you show.
    pub seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ActivityEvent {
    pub tenant: String,
    pub udid: String,
    /// The function that drove it; `ios-simulator::open` asks to be shown.
    pub function: String,
    pub timestamp: i64,
}

type Bindings = Arc<RwLock<HashMap<String, (TriggerConfig, BindingConfig)>>>;

struct BindingTable(Bindings);

#[async_trait]
impl TriggerHandler for BindingTable {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let raw = if config.config.is_null() {
            serde_json::json!({})
        } else {
            config.config.clone()
        };
        let filter: BindingConfig = serde_json::from_value(raw)
            .map_err(|e| Error::Handler(format!("invalid ios-simulator trigger config: {e}")))?;
        self.0
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(config.id.clone(), (config, filter));
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.0
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&config.id);
        Ok(())
    }
}

pub fn matches(filter: &BindingConfig, tenant: &str, udid: &str) -> bool {
    filter.tenant.as_deref().is_none_or(|t| t == tenant)
        && filter.udid.as_deref().is_none_or(|u| u == udid)
}

pub struct Events {
    iii: Arc<IIIClient>,
    device: Bindings,
    media: Bindings,
    frame: Bindings,
    activity: Bindings,
}

impl Events {
    /// Register the trigger types. Run before any function can emit.
    pub fn register(iii: &Arc<IIIClient>) -> Arc<Self> {
        let events = Arc::new(Self {
            iii: iii.clone(),
            device: Bindings::default(),
            media: Bindings::default(),
            frame: Bindings::default(),
            activity: Bindings::default(),
        });
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                DEVICE_CHANGED,
                "A simulator was added, removed, booted or shut down (by this worker, Xcode or simctl). Config: { tenant?, udid? }.",
                BindingTable(events.device.clone()),
            )
            .trigger_request_format::<BindingConfig>()
            .call_request_format::<DeviceChangedEvent>(),
        );
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                MEDIA_CHANGED,
                "A simulator screenshot or recording was saved or deleted. Config: { tenant?, udid? }.",
                BindingTable(events.media.clone()),
            )
            .trigger_request_format::<BindingConfig>()
            .call_request_format::<MediaChangedEvent>(),
        );
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                FRAME_EVENT,
                "A live-view frame of a simulator someone is watching (ios-simulator::watch). Config: { tenant?, udid? }.",
                BindingTable(events.frame.clone()),
            )
            .trigger_request_format::<BindingConfig>()
            .call_request_format::<FrameEvent>(),
        );
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                ACTIVITY,
                "A caller drove a simulator (gesture, button, typing, app, screenshot, ios-simulator::open). Config: { tenant?, udid? }.",
                BindingTable(events.activity.clone()),
            )
            .trigger_request_format::<BindingConfig>()
            .call_request_format::<ActivityEvent>(),
        );
        events
    }

    pub fn device(&self, event: DeviceChangedEvent) {
        self.deliver(&self.device, &event.tenant, &event.udid, &event);
    }

    pub fn media(&self, event: MediaChangedEvent) {
        self.deliver(&self.media, &event.tenant, &event.udid, &event);
    }

    pub fn frame(&self, event: FrameEvent) {
        self.deliver(&self.frame, &event.tenant, &event.udid, &event);
    }

    pub fn activity(&self, event: ActivityEvent) {
        self.deliver(&self.activity, &event.tenant, &event.udid, &event);
    }

    fn deliver<T: Serialize>(&self, table: &Bindings, tenant: &str, udid: &str, event: &T) {
        let targets: Vec<TriggerConfig> = table
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .filter(|(_, filter)| matches(filter, tenant, udid))
            .map(|(binding, _)| binding.clone())
            .collect();
        if targets.is_empty() {
            return;
        }
        // After the binding check: a frame nobody watches is never encoded.
        let Ok(payload) = serde_json::to_value(event) else {
            return;
        };
        let iii = self.iii.clone();
        tokio::spawn(async move {
            for binding in targets {
                let mut request: TriggerRequestWithMetadata = TriggerRequest {
                    function_id: binding.function_id.clone(),
                    payload: payload.clone(),
                    action: Some(TriggerAction::Void),
                    timeout_ms: None,
                }
                .into();
                if let Some(namespace) = &binding.namespace {
                    request = request.namespace(namespace.clone());
                }
                if let Err(e) = iii.trigger(request).await {
                    tracing::warn!(function_id = %binding.function_id, error = %e, "event delivery failed");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_narrow_by_tenant_and_udid() {
        let any = BindingConfig::default();
        assert!(matches(&any, "acme", "U1"));
        let acme = BindingConfig {
            tenant: Some("acme".into()),
            udid: None,
        };
        assert!(matches(&acme, "acme", "U1"));
        assert!(!matches(&acme, "globex", "U1"));
        let one = BindingConfig {
            tenant: Some("acme".into()),
            udid: Some("U1".into()),
        };
        assert!(!matches(&one, "acme", "U2"));
    }

    #[test]
    fn misspelled_filter_keys_are_refused() {
        assert!(
            serde_json::from_value::<BindingConfig>(serde_json::json!({ "tenat": "x" })).is_err()
        );
    }
}
