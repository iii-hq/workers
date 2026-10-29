//! The four observable functions this worker provides. Each is private (only
//! this worker calls it), and its handler returns the event it is called
//! with: that event is what observers observe.
//!
//! - `ios-simulator::on-device-changed` — a simulator appeared, disappeared
//!   or changed state (booted, shut down, renamed), whoever caused it: this
//!   worker, Xcode, or `simctl` in a terminal.
//! - `ios-simulator::on-media-changed` — a screenshot or recording was saved
//!   or deleted.
//! - `ios-simulator::on-frame-event` — one live-view frame of a watched
//!   simulator, delivered straight to its observers (as
//!   `browser::on-frame-event` does): the live view depends on no other
//!   worker.
//! - `ios-simulator::on-activity` — a caller drove a simulator (a gesture, a
//!   button, typing, an app, a screenshot, `ios-simulator::open`), so live
//!   previews can surface it while an agent works.
//!
//! The device, media and activity functions take no args: observers select
//! calls with filters on `input.tenant` and `input.udid`. Frames take args
//! `{ tenant?, udid? }`: each observer is filed under its tenant and udid
//! (`*` for one it omits), and the bridge builds a frame only when an
//! observer is filed under one of its four keys. An observer that names no
//! tenant sees every tenant (the operator's console); a shared deployment's
//! `rbac-proxy` observer-registration hook stamps the tenant on observers
//! made by tenants (`args.tenant` for frames, an `input.tenant` filter for
//! the rest). Calls are fire-and-forget.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{Call, CallAction, CallRequest, IIIClient, Match, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const ON_DEVICE_CHANGED_FN: &str = "ios-simulator::on-device-changed";
pub const ON_MEDIA_CHANGED_FN: &str = "ios-simulator::on-media-changed";
pub const ON_FRAME_EVENT_FN: &str = "ios-simulator::on-frame-event";
pub const ON_ACTIVITY_FN: &str = "ios-simulator::on-activity";

/// The args of `ios-simulator::on-frame-event`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrameArgs {
    /// Only this tenant's frames. Omit for every tenant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
    /// Only this simulator's frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udid: Option<String>,
}

/// Stands for an omitted tenant or udid in a key. Tenants are slugs and
/// udids are UUIDs, so neither is ever `*` or holds `|`.
const ANY: &str = "*";

fn key(tenant: &str, udid: &str) -> String {
    format!("{tenant}|{udid}")
}

/// The key an observer of frames is filed under.
pub fn observer_key(args: &FrameArgs) -> String {
    key(
        args.tenant.as_deref().unwrap_or(ANY),
        args.udid.as_deref().unwrap_or(ANY),
    )
}

/// The keys a frame of `tenant`'s simulator `udid` matches: four lookups,
/// whatever the number of observers.
pub fn frame_keys(tenant: &str, udid: &str) -> Vec<String> {
    vec![
        key(tenant, udid),
        key(tenant, ANY),
        key(ANY, udid),
        key(ANY, ANY),
    ]
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

pub struct Events {
    iii: Arc<IIIClient>,
}

impl Events {
    /// Register the four functions. Run before any function can emit.
    pub fn register(iii: &Arc<IIIClient>) -> Arc<Self> {
        iii.register_function(
            ON_DEVICE_CHANGED_FN,
            RegisterFunction::new(|event: DeviceChangedEvent| Ok::<_, Error>(event))
                .description(
                    "A simulator was added, removed, booted or shut down (by this worker, Xcode or simctl). \
                     No args; filter on input.tenant and input.udid.",
                )
                .private(),
        );
        iii.register_function(
            ON_MEDIA_CHANGED_FN,
            RegisterFunction::new(|event: MediaChangedEvent| Ok::<_, Error>(event))
                .description(
                    "A simulator screenshot or recording was saved or deleted. \
                     No args; filter on input.tenant and input.udid.",
                )
                .private(),
        );
        iii.register_function(
            ON_ACTIVITY_FN,
            RegisterFunction::new(|event: ActivityEvent| Ok::<_, Error>(event))
                .description(
                    "A caller drove a simulator (gesture, button, typing, app, screenshot, ios-simulator::open). \
                     No args; filter on input.tenant and input.udid.",
                )
                .private(),
        );
        // Set before registering, so it files every observer.
        iii.on_observer_match_for(
            ON_FRAME_EVENT_FN,
            Match::keys(
                |obs| {
                    let args = obs.args_value();
                    key(
                        args["tenant"].as_str().unwrap_or(ANY),
                        args["udid"].as_str().unwrap_or(ANY),
                    )
                    .into()
                },
                |call| {
                    let input = &call.input;
                    frame_keys(
                        input["tenant"].as_str().unwrap_or_default(),
                        input["udid"].as_str().unwrap_or_default(),
                    )
                    .into()
                },
            ),
        );
        iii.register_function(
            ON_FRAME_EVENT_FN,
            RegisterFunction::new(|event: FrameEvent| Ok::<_, Error>(event))
                .description(
                    "A live-view frame of a simulator someone is watching (ios-simulator::watch). \
                     Args: { tenant?, udid? }. Observe with max_in_flight: 1 so a slow viewer drops frames.",
                )
                .args_format::<FrameArgs>()
                .private(),
        );
        Arc::new(Self { iii: iii.clone() })
    }

    pub fn device(&self, event: DeviceChangedEvent) {
        self.emit(ON_DEVICE_CHANGED_FN, &event);
    }

    pub fn media(&self, event: MediaChangedEvent) {
        self.emit(ON_MEDIA_CHANGED_FN, &event);
    }

    /// Whether a frame of `tenant`'s simulator `udid` would reach an
    /// observer. The bridge builds a frame only then.
    pub fn frame_observed(&self, tenant: &str, udid: &str) -> bool {
        if !self.iii.is_observed(ON_FRAME_EVENT_FN) {
            return false;
        }
        let call = Call {
            input: json!({ "tenant": tenant, "udid": udid }),
            ..Call::default()
        };
        self.iii.is_observed_for(ON_FRAME_EVENT_FN, &call)
    }

    pub fn frame(&self, event: FrameEvent) {
        self.emit(ON_FRAME_EVENT_FN, &event);
    }

    pub fn activity(&self, event: ActivityEvent) {
        self.emit(ON_ACTIVITY_FN, &event);
    }

    /// One fire-and-forget call of `function_id`; iii delivers it to the
    /// observers that want it.
    fn emit<T: Serialize>(&self, function_id: &'static str, event: &T) {
        // Before serializing: an event nobody observes is never encoded.
        if !self.iii.is_observed(function_id) {
            return;
        }
        let Ok(payload) = serde_json::to_value(event) else {
            return;
        };
        let iii = self.iii.clone();
        tokio::spawn(async move {
            let request = CallRequest {
                function_id: function_id.to_string(),
                payload,
                action: Some(CallAction::Void),
                timeout_ms: None,
            };
            if let Err(e) = iii.call(request).await {
                tracing::warn!(function_id, error = %e, "event call failed");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(tenant: Option<&str>, udid: Option<&str>) -> FrameArgs {
        FrameArgs {
            tenant: tenant.map(str::to_string),
            udid: udid.map(str::to_string),
        }
    }

    /// A frame reaches an observer when the observer's key is among the
    /// frame's: the tenant and udid it named, `*` for one it omitted.
    #[test]
    fn frame_keys_narrow_by_tenant_and_udid() {
        let wants = |args: &FrameArgs, tenant: &str, udid: &str| {
            frame_keys(tenant, udid).contains(&observer_key(args))
        };
        assert!(wants(&args(None, None), "acme", "U1"));
        let acme = args(Some("acme"), None);
        assert!(wants(&acme, "acme", "U1"));
        assert!(!wants(&acme, "globex", "U1"));
        let one = args(Some("acme"), Some("U1"));
        assert!(wants(&one, "acme", "U1"));
        assert!(!wants(&one, "acme", "U2"));
        let any_tenant = args(None, Some("U1"));
        assert!(wants(&any_tenant, "globex", "U1"));
        assert!(!wants(&any_tenant, "globex", "U2"));
    }

    #[test]
    fn misspelled_args_are_refused() {
        assert!(serde_json::from_value::<FrameArgs>(serde_json::json!({ "tenat": "x" })).is_err());
    }

    /// The args schema the engine validates observers against.
    #[test]
    fn frame_args_schema_is_closed() {
        let schema = serde_json::to_value(schemars::schema_for!(FrameArgs)).unwrap();
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["properties"]["tenant"].is_object());
        assert!(schema["properties"]["udid"].is_object());
        assert!(schema.get("required").is_none());
    }
}
