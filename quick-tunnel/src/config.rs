use std::{
    collections::{BTreeMap, BTreeSet},
    net::IpAddr,
    path::PathBuf,
};

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

/// Which tunnel service publishes the targets. One per worker instance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// cloudflared Quick Tunnel child processes (operator-provided binary).
    #[default]
    Cloudflared,
    /// OpenTunnel, in-process: TLS ends in this worker, the hostname is stable.
    Opentunnel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub provider: ProviderKind,
    /// Operator allowlist. Only literal loopback IP HTTP(S) origins are accepted
    /// (`http://` only, and DNS-label ids, with provider `opentunnel`).
    pub targets: BTreeMap<String, String>,
    /// Target ids whose local service reads a PROXY protocol v2 header: the first
    /// bytes of every connection then carry the visitor's address. Provider
    /// `opentunnel` only.
    pub proxy_protocol: BTreeSet<String>,
    /// Prerequisite executable; never downloaded by this worker. Ignored by
    /// provider `opentunnel`.
    pub cloudflared: String,
    pub state_path: PathBuf,
    pub startup_timeout_ms: u64,
    /// Additional process attempts after the first, per lease cohort.
    pub max_retries: u32,
    pub retry_initial_ms: u64,
    pub retry_max_ms: u64,
    /// After the fast retry budget is spent, keep trying once per cooldown while
    /// leases remain, so a transient outage cannot park a leased tunnel forever.
    /// 0 keeps `failed` terminal until leases end or the worker restarts.
    pub failed_cooldown_ms: u64,
    /// A generation continuously ready for this long restores the retry budget.
    /// Crash-after-connect loops shorter than this still consume it. 0 disables.
    pub retry_budget_reset_ms: u64,
    pub max_leases: usize,
    pub max_lease_seconds: i64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            provider: ProviderKind::Cloudflared,
            targets: BTreeMap::from([("webhooks".into(), "http://127.0.0.1:3112".into())]),
            proxy_protocol: BTreeSet::new(),
            cloudflared: "cloudflared".into(),
            state_path: PathBuf::from("data/quick-tunnel/leases.json"),
            startup_timeout_ms: 30_000,
            max_retries: 3,
            retry_initial_ms: 1_000,
            retry_max_ms: 30_000,
            failed_cooldown_ms: 300_000,
            retry_budget_reset_ms: 600_000,
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
            // The id names the OpenTunnel profile; the worker hands plaintext to the target.
            if self.provider == ProviderKind::Opentunnel
                && (u.scheme() != "http" || !opentunnel::protocol::names::is_valid_profile(id))
            {
                return Err(
                    "opentunnel targets must be http:// origins with lowercase DNS-label ids"
                        .into(),
                );
            }
        }
        if self
            .proxy_protocol
            .iter()
            .any(|id| !self.targets.contains_key(id))
            || (!self.proxy_protocol.is_empty() && self.provider != ProviderKind::Opentunnel)
        {
            return Err(
                "proxy_protocol needs provider opentunnel and ids listed in targets".into(),
            );
        }
        // A local listener reads the header or it does not: every target on its port must agree.
        let port = |origin: &String| {
            Url::parse(origin)
                .ok()
                .and_then(|u| u.port_or_known_default())
        };
        let (listed, unlisted): (Vec<_>, Vec<_>) = self
            .targets
            .iter()
            .partition(|(id, _)| self.proxy_protocol.contains(*id));
        if listed
            .iter()
            .any(|(_, a)| unlisted.iter().any(|(_, b)| port(a) == port(b)))
        {
            return Err("targets on the same port must all be in proxy_protocol or none".into());
        }
        if self.cloudflared.trim().is_empty()
            || (self.cloudflared.contains('/') && !PathBuf::from(&self.cloudflared).is_absolute())
            || self.state_path.as_os_str().is_empty()
            || !(50..=300_000).contains(&self.startup_timeout_ms)
            || self.max_retries > 10
            || self.retry_initial_ms == 0
            || self.retry_initial_ms > self.retry_max_ms
            || self.retry_max_ms > 300_000
            || (self.failed_cooldown_ms != 0 && self.failed_cooldown_ms < self.retry_max_ms)
            || self.failed_cooldown_ms > 3_600_000
            || self.retry_budget_reset_ms > 86_400_000
            || !(1..=100_000).contains(&self.max_leases)
            || !(1..=2_592_000).contains(&self.max_lease_seconds)
        {
            return Err("invalid executable, state path or lifecycle limits".into());
        }
        Ok(())
    }

    /// "v2" when every connection to this target starts with a PROXY protocol v2 header.
    pub fn proxy_protocol_of(&self, id: &str) -> Option<String> {
        self.proxy_protocol.contains(id).then(|| "v2".to_owned())
    }
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opentunnel(id: &str, origin: &str) -> Config {
        Config {
            provider: ProviderKind::Opentunnel,
            targets: BTreeMap::from([(id.into(), origin.into())]),
            ..Config::default()
        }
    }

    #[test]
    fn provider_defaults_to_cloudflared_and_stored_configs_without_it_load() {
        assert_eq!(Config::default().provider, ProviderKind::Cloudflared);
        let stored: Config = serde_yaml::from_str(
            "targets:\n  webhooks: http://127.0.0.1:3112\ncloudflared: cloudflared\n",
        )
        .unwrap();
        assert_eq!(stored.provider, ProviderKind::Cloudflared);
        assert!(stored.validate().is_ok());
        let chosen: Config = serde_yaml::from_str("provider: opentunnel\n").unwrap();
        assert_eq!(chosen.provider, ProviderKind::Opentunnel);
        assert!(serde_yaml::from_str::<Config>("provider: ngrok\n").is_err());
    }

    #[test]
    fn opentunnel_needs_http_origins_and_profile_ids() {
        assert!(opentunnel("webhooks", "http://127.0.0.1:3112")
            .validate()
            .is_ok());
        assert!(opentunnel("mesh", "http://[::1]:49180").validate().is_ok());
        assert!(opentunnel("webhooks", "https://127.0.0.1:3112")
            .validate()
            .is_err());
        for id in [
            "Webhooks",
            "web_hooks",
            "web.hooks",
            "-webhooks",
            "webhooks-",
            "a:b",
        ] {
            assert!(
                opentunnel(id, "http://127.0.0.1:3112").validate().is_err(),
                "{id}"
            );
        }
        // cloudflared keeps accepting what it accepted before.
        let https = Config {
            targets: BTreeMap::from([("Web_hooks".into(), "https://127.0.0.1:3112".into())]),
            ..Config::default()
        };
        assert!(https.validate().is_ok());
    }

    #[test]
    fn proxy_protocol_needs_opentunnel_listed_targets_and_one_mode_per_port() {
        assert!(Config::default().proxy_protocol.is_empty());
        let stored: Config = serde_yaml::from_str(
            "provider: opentunnel\ntargets:\n  mesh: http://127.0.0.1:49180\n",
        )
        .unwrap();
        assert!(stored.proxy_protocol.is_empty());
        let listed = |provider, targets: &[(&str, &str)], ids: &[&str]| Config {
            provider,
            targets: targets
                .iter()
                .map(|(id, origin)| (id.to_string(), origin.to_string()))
                .collect(),
            proxy_protocol: ids.iter().map(|id| id.to_string()).collect(),
            ..Config::default()
        };
        let mesh = listed(
            ProviderKind::Opentunnel,
            &[("mesh", "http://127.0.0.1:49180")],
            &["mesh"],
        );
        assert!(mesh.validate().is_ok());
        assert_eq!(mesh.proxy_protocol_of("mesh").as_deref(), Some("v2"));
        assert_eq!(mesh.proxy_protocol_of("webhooks"), None);
        // The documented setup: one listed and one unlisted target, on different ports.
        let documented = listed(
            ProviderKind::Opentunnel,
            &[
                ("webhooks", "http://127.0.0.1:3112"),
                ("mesh", "http://127.0.0.1:49180"),
            ],
            &["mesh"],
        );
        assert_eq!(documented.validate(), Ok(()));
        let refused = "proxy_protocol needs provider opentunnel and ids listed in targets";
        let ghost = listed(
            ProviderKind::Opentunnel,
            &[("mesh", "http://127.0.0.1:49180")],
            &["ghost"],
        );
        assert_eq!(ghost.validate().unwrap_err(), refused);
        let cloudflared = listed(
            ProviderKind::Cloudflared,
            &[("webhooks", "http://127.0.0.1:3112")],
            &["webhooks"],
        );
        assert_eq!(cloudflared.validate().unwrap_err(), refused);
        let shared = [
            ("mesh", "http://127.0.0.1:49180"),
            ("mesh2", "http://[::1]:49180"),
        ];
        assert_eq!(
            listed(ProviderKind::Opentunnel, &shared, &["mesh"])
                .validate()
                .unwrap_err(),
            "targets on the same port must all be in proxy_protocol or none"
        );
        assert!(
            listed(ProviderKind::Opentunnel, &shared, &["mesh", "mesh2"])
                .validate()
                .is_ok()
        );
    }
}
