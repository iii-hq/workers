//! Persistence in iii-state (engine `state::*` functions, binary-worker.md
//! § 7): the router registration token, plus the long-lived GitHub OAuth
//! token the device-flow login stores ([`crate::login`]). The short-lived
//! Copilot bearer is deliberately NOT persisted — it lives ~25 minutes and
//! is cheap to re-exchange on boot.
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

pub const STATE_SCOPE: &str = "provider-github-copilot";
const TOKEN_KEY: &str = "registration_token";
pub const OAUTH_TOKEN_KEY: &str = "oauth_token";
const PROBE_RESULTS_KEY: &str = "probe_results";

/// Probe verdicts are re-checked after this long, so a plan upgrade (or a
/// model the operator newly enabled) is picked up without a manual reset.
const PROBE_TTL_SECS: i64 = 24 * 60 * 60;

/// Cached `upstream model id -> callable` verdicts plus when they were taken.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProbeResults {
    pub taken_at: i64,
    pub verdicts: std::collections::BTreeMap<String, bool>,
}

impl ProbeResults {
    fn expired(&self) -> bool {
        self.taken_at == 0 || crate::now_ms() / 1000 - self.taken_at > PROBE_TTL_SECS
    }
}

/// Load the cached verdicts, discarding an expired set.
pub async fn load_probe_results(iii: &IIIClient) -> ProbeResults {
    let Ok(value) = iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": STATE_SCOPE, "key": PROBE_RESULTS_KEY }),
            action: None,
            timeout_ms: None,
        })
        .await
    else {
        return ProbeResults::default();
    };
    let parsed: ProbeResults = serde_json::from_value(value).unwrap_or_default();
    if parsed.expired() {
        ProbeResults::default()
    } else {
        parsed
    }
}

pub async fn store_probe_results(iii: &IIIClient, results: &ProbeResults) {
    let payload = json!({
        "scope": STATE_SCOPE,
        "key": PROBE_RESULTS_KEY,
        "value": serde_json::to_value(results).unwrap_or(Value::Null),
    });
    if let Err(e) = iii
        .trigger(TriggerRequest {
            function_id: "state::set".into(),
            payload,
            action: None,
            timeout_ms: None,
        })
        .await
    {
        eprintln!("[provider-github-copilot] storing probe results failed ({e})");
    }
}

/// Record one model as uncallable so the next refresh drops it without
/// spending another probe (the self-healing path in `router_client`).
pub async fn record_unavailable(iii: &IIIClient, upstream_model: &str) {
    let mut results = load_probe_results(iii).await;
    if results.taken_at == 0 {
        results.taken_at = crate::now_ms() / 1000;
    }
    results.verdicts.insert(upstream_model.to_string(), false);
    store_probe_results(iii, &results).await;
}

pub async fn load_token(iii: &IIIClient) -> Option<String> {
    let value = iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": STATE_SCOPE, "key": TOKEN_KEY }),
            action: None,
            timeout_ms: None,
        })
        .await
        .ok()?;
    // An empty value is not a token: presenting one would be rejected as a
    // mismatch instead of registering fresh.
    value.as_str().filter(|s| !s.is_empty()).map(String::from)
}

pub async fn store_token(iii: &IIIClient, token: &str) -> Result<(), Error> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".into(),
        payload: json!({ "scope": STATE_SCOPE, "key": TOKEN_KEY, "value": Value::from(token) }),
        action: None,
        timeout_ms: None,
    })
    .await?;
    Ok(())
}
