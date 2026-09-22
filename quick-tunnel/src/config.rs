use std::{collections::BTreeMap, net::IpAddr, path::PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

/// Built-in configuration entry id; also the stable `metadata.ui_form` family.
pub const DEFAULT_CONFIG_ID: &str = "quick-tunnel";

/// The configuration entry this worker owns: `III_CONFIG_NAME` when a
/// supervisor set it (trimmed, non-empty), else the built-in name. Cached for
/// the process lifetime because `EntrySpec` needs a `'static` identity.
pub fn config_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        std::env::var("III_CONFIG_NAME")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_CONFIG_ID.to_string())
    })
    .as_str()
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Operator allowlist. Only literal loopback IP HTTP(S) origins are accepted.
    pub targets: BTreeMap<String, String>,
    /// Prerequisite executable; never downloaded by this worker.
    pub cloudflared: String,
    pub state_path: PathBuf,
    pub startup_timeout_ms: u64,
    /// Additional process attempts after the first, per lease cohort.
    pub max_retries: u32,
    pub retry_initial_ms: u64,
    pub retry_max_ms: u64,
    pub max_leases: usize,
    pub max_lease_seconds: i64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            targets: BTreeMap::from([("webhooks".into(), "http://127.0.0.1:3112".into())]),
            cloudflared: "cloudflared".into(),
            state_path: PathBuf::from("data/quick-tunnel/leases.json"),
            startup_timeout_ms: 30_000,
            max_retries: 3,
            retry_initial_ms: 1_000,
            retry_max_ms: 30_000,
            max_leases: 1024,
            max_lease_seconds: 2_592_000,
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if self.targets.is_empty() || self.targets.len() > 32 {
            return Err("targets must contain 1..=32 entries".into());
        }
        for (id, origin) in &self.targets {
            if !valid_id(id) {
                return Err("invalid target identifier".into());
            }
            let u = Url::parse(origin).map_err(|_| "invalid target URL")?;
            let loopback = match u.host() {
                Some(url::Host::Ipv4(ip)) => IpAddr::V4(ip).is_loopback(),
                Some(url::Host::Ipv6(ip)) => IpAddr::V6(ip).is_loopback(),
                _ => false,
            };
            if !loopback
                || !matches!(u.scheme(), "http" | "https")
                || !u.username().is_empty()
                || u.password().is_some()
                || u.query().is_some()
                || u.fragment().is_some()
                || u.path() != "/"
                || u.port_or_known_default() == Some(0)
            {
                return Err("targets must be HTTP(S) literal loopback origins without credentials, path, query or fragment".into());
            }
        }
        if self.cloudflared.trim().is_empty()
            || (self.cloudflared.contains('/') && !PathBuf::from(&self.cloudflared).is_absolute())
            || self.state_path.as_os_str().is_empty()
            || !(50..=300_000).contains(&self.startup_timeout_ms)
            || self.max_retries > 10
            || self.retry_initial_ms == 0
            || self.retry_initial_ms > self.retry_max_ms
            || self.retry_max_ms > 300_000
            || !(1..=100_000).contains(&self.max_leases)
            || !(1..=2_592_000).contains(&self.max_lease_seconds)
        {
            return Err("invalid executable, state path or lifecycle limits".into());
        }
        Ok(())
    }
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
}
