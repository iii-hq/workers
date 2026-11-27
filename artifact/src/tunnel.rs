//! The quick-tunnel client: acquire, renew and release this worker's lease on
//! the operator-configured target, and read the tunnel's public URL.
//!
//! The worker never names an origin. The operator maps `tunnel_id` to the
//! share listener in the quick-tunnel configuration; the provider behind it
//! (cloudflared or opentunnel) is the operator's choice too.
//!
//! `Tunnel::Fake` is the engine-free test seam.

use std::sync::Arc;

use iii_sdk::{CallRequest, IIIClient};
use serde_json::{Value, json};

use crate::util::rfc3339;

/// Stable consumer identity for every lease this worker takes.
pub const CONSUMER_ID: &str = "artifact";

const TUNNEL_TIMEOUT_MS: u64 = 15_000;

/// What the tunnel looks like right now.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TunnelSnapshot {
    /// `ready`, `starting`, `reconnecting`, `failed`, … as quick-tunnel says.
    pub status: String,
    /// The public base URL, present only while `ready`.
    pub public_url: Option<String>,
    /// Sanitized diagnostics on failure.
    pub error: Option<String>,
}

impl TunnelSnapshot {
    pub fn is_ready(&self) -> bool {
        self.status == "ready" && self.public_url.is_some()
    }

    pub fn is_failed(&self) -> bool {
        self.status == "failed"
    }

    /// Read a quick-tunnel response, flat or with a nested `snapshot`.
    fn from_value(value: &Value) -> Self {
        let source = value.get("snapshot").unwrap_or(value);
        let field = |name: &str| {
            source
                .get(name)
                .or_else(|| value.get(name))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let status = field("status").unwrap_or_else(|| "unknown".to_string());
        let public_url = field("public_url").filter(|_| status == "ready");
        Self {
            status,
            public_url,
            error: field("error"),
        }
    }
}

/// A scripted tunnel for tests: records calls and answers from its fields.
#[derive(Debug, Default)]
pub struct FakeTunnel {
    /// Public URL reported once acquired; `None` keeps it `starting`.
    pub url: Option<String>,
    /// When set, every call fails with this message.
    pub fail: Option<String>,
    /// `(tunnel_id, expires_at)` of every acquire.
    pub acquired: Vec<(String, i64)>,
    /// Lease ids released.
    pub released: Vec<String>,
    held: bool,
}

pub enum Tunnel {
    Bus(Arc<IIIClient>),
    Fake(std::sync::Mutex<FakeTunnel>),
}

impl Tunnel {
    pub fn fake(url: Option<&str>) -> Self {
        Tunnel::Fake(std::sync::Mutex::new(FakeTunnel {
            url: url.map(str::to_string),
            ..FakeTunnel::default()
        }))
    }

    /// Run `f` over the fake's state. Panics on a bus tunnel: test-only.
    pub fn with_fake<R>(&self, f: impl FnOnce(&mut FakeTunnel) -> R) -> R {
        match self {
            Tunnel::Fake(fake) => f(&mut fake.lock().expect("fake tunnel lock")),
            Tunnel::Bus(_) => panic!("with_fake called on a bus tunnel"),
        }
    }

    async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value, String> {
        iii.call(
            CallRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms: Some(TUNNEL_TIMEOUT_MS),
            }
            .namespace("default"),
        )
        .await
        .map_err(|e| format!("{function_id} failed: {e}"))
    }

    /// Acquire or renew the lease until `expires_at` (unix seconds). Returns
    /// the lease id and the tunnel snapshot at that moment.
    pub async fn acquire(
        &self,
        tunnel_id: &str,
        expires_at: i64,
    ) -> Result<(String, TunnelSnapshot), String> {
        match self {
            Tunnel::Bus(iii) => {
                let value = Self::call(
                    iii,
                    "quick-tunnel::acquire",
                    json!({
                        "consumer_id": CONSUMER_ID,
                        "tunnel_id": tunnel_id,
                        "expires_at": rfc3339(expires_at),
                    }),
                )
                .await?;
                let lease_id = value
                    .get("lease_id")
                    .and_then(Value::as_str)
                    .ok_or("quick-tunnel::acquire answered without a lease_id")?
                    .to_string();
                Ok((lease_id, TunnelSnapshot::from_value(&value)))
            }
            Tunnel::Fake(fake) => {
                let mut fake = fake.lock().expect("fake tunnel lock");
                if let Some(err) = &fake.fail {
                    return Err(err.clone());
                }
                fake.acquired.push((tunnel_id.to_string(), expires_at));
                fake.held = true;
                Ok((format!("lease-{tunnel_id}"), fake.snapshot()))
            }
        }
    }

    pub async fn status(&self, tunnel_id: &str) -> Result<TunnelSnapshot, String> {
        match self {
            Tunnel::Bus(iii) => {
                let value = Self::call(
                    iii,
                    "quick-tunnel::status",
                    json!({ "tunnel_id": tunnel_id }),
                )
                .await?;
                Ok(TunnelSnapshot::from_value(&value))
            }
            Tunnel::Fake(fake) => {
                let fake = fake.lock().expect("fake tunnel lock");
                match &fake.fail {
                    Some(err) => Err(err.clone()),
                    None => Ok(fake.snapshot()),
                }
            }
        }
    }

    pub async fn release(&self, lease_id: &str) -> Result<(), String> {
        match self {
            Tunnel::Bus(iii) => Self::call(
                iii,
                "quick-tunnel::release",
                json!({ "lease_id": lease_id }),
            )
            .await
            .map(|_| ()),
            Tunnel::Fake(fake) => {
                let mut fake = fake.lock().expect("fake tunnel lock");
                fake.released.push(lease_id.to_string());
                fake.held = false;
                Ok(())
            }
        }
    }
}

impl FakeTunnel {
    fn snapshot(&self) -> TunnelSnapshot {
        match (&self.url, self.held) {
            (Some(url), true) => TunnelSnapshot {
                status: "ready".to_string(),
                public_url: Some(url.clone()),
                error: None,
            },
            (None, true) => TunnelSnapshot {
                status: "starting".to_string(),
                ..TunnelSnapshot::default()
            },
            (_, false) => TunnelSnapshot {
                status: "stopped".to_string(),
                ..TunnelSnapshot::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_reads_the_flat_status_shape() {
        let snap = TunnelSnapshot::from_value(&json!({
            "status": "ready",
            "public_url": "https://a.trycloudflare.com",
            "tunnel_id": "artifacts",
        }));
        assert!(snap.is_ready());
        assert_eq!(
            snap.public_url.as_deref(),
            Some("https://a.trycloudflare.com")
        );
    }

    #[test]
    fn snapshot_reads_a_nested_snapshot() {
        let snap = TunnelSnapshot::from_value(&json!({
            "lease_id": "l1",
            "snapshot": { "status": "starting", "public_url": null },
        }));
        assert_eq!(snap.status, "starting");
        assert!(!snap.is_ready());
    }

    /// A URL reported outside `ready` belongs to a dead or stale generation.
    #[test]
    fn a_url_outside_ready_is_dropped() {
        let snap = TunnelSnapshot::from_value(&json!({
            "status": "reconnecting",
            "public_url": "https://old.trycloudflare.com",
        }));
        assert_eq!(snap.public_url, None);
    }
}
