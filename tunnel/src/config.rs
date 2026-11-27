use std::{
    collections::{BTreeMap, BTreeSet},
    net::IpAddr,
    path::{Path, PathBuf},
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

/// Built-in configuration entry id; also the stable `metadata.ui_form` family.
pub const DEFAULT_CONFIG_ID: &str = "tunnel";

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
///
/// The enum default stays `Cloudflared` because it is also the wire meaning of a
/// `status` response without `provider`. A new configuration defaults to
/// `Opentunnel` through [`Config::default`]; see [`fill_missing_provider`].
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
            provider: ProviderKind::Opentunnel,
            targets: BTreeMap::from([("webhooks".into(), "http://127.0.0.1:3112".into())]),
            proxy_protocol: BTreeSet::new(),
            cloudflared: "cloudflared".into(),
            state_path: PathBuf::from("data/tunnel/leases.json"),
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

/// Data directory of the `quick-tunnel` package this worker replaces.
pub const LEGACY_STATE_DIR: &str = "data/quick-tunnel";

/// On the first boot after replacing `quick-tunnel`, moves its data directory
/// to this worker's default one. The upgraded install keeps its leases, its
/// provider choice and its OpenTunnel identity, and with it its public URL.
/// Only the default `state_path` adopts; a custom one is the operator's.
/// Returns whether anything moved.
///
/// # Errors
///
/// Fails when a `quick-tunnel` worker still holds the legacy lease lock, or
/// when the directory cannot be moved.
pub fn adopt_legacy_state(state_path: &Path) -> std::io::Result<bool> {
    if state_path != Config::default().state_path {
        return Ok(false);
    }
    adopt_from(Path::new(LEGACY_STATE_DIR), state_path)
}

/// [`adopt_legacy_state`] from an explicit legacy directory.
///
/// # Errors
///
/// See [`adopt_legacy_state`].
pub fn adopt_from(legacy: &Path, state_path: &Path) -> std::io::Result<bool> {
    use fs2::FileExt;
    let target = crate::manager::state_parent(state_path);
    if target.exists() || !legacy.is_dir() {
        return Ok(false);
    }
    // A quick-tunnel worker still running holds this lock: moving its files
    // from under it would split the lease state between two processes.
    let lock = match std::fs::File::options()
        .read(true)
        .write(true)
        .open(legacy.join("leases.lock"))
    {
        Ok(file) => Some(file),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    if let Some(lock) = &lock {
        lock.try_lock_exclusive().map_err(|_| {
            std::io::Error::other(format!(
                "quick-tunnel is still running on {}; stop and remove it before starting tunnel",
                legacy.display()
            ))
        })?;
    }
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(legacy, target)?;
    Ok(true)
}

/// The provider for a stored configuration that names none.
///
/// Configurations written before `provider` existed leave it out, and so do
/// Compose-injected package defaults (the manifest leaves the key out on
/// purpose, see `package_defaults_leave_the_provider_to_boot_resolution`). A
/// worker that already ran on cloudflared (including `quick-tunnel` data
/// adopted by [`adopt_legacy_state`]) keeps it: switching would change its public URL
/// and could refuse targets OpenTunnel cannot carry. A fresh worker gets
/// OpenTunnel, which needs no binary.
///
/// The OpenTunnel state directory is created before the lease file on every
/// OpenTunnel boot, so it marks a worker that already chose OpenTunnel.
pub fn provider_for_unnamed(state_path: &Path) -> ProviderKind {
    let parent = crate::manager::state_parent(state_path);
    if parent.join("opentunnel").is_dir() || !state_path.exists() {
        ProviderKind::Opentunnel
    } else {
        ProviderKind::Cloudflared
    }
}

/// Writes the resolved provider into a raw configuration that omits it, before
/// it is parsed. Returns the provider it filled in, if any.
pub fn fill_missing_provider(value: &mut serde_json::Value) -> Option<ProviderKind> {
    let map = value.as_object_mut()?;
    if map.get("provider").is_some_and(|p| !p.is_null()) {
        return None;
    }
    let state_path = map
        .get("state_path")
        .and_then(serde_json::Value::as_str)
        .map_or_else(|| Config::default().state_path, PathBuf::from);
    let kind = provider_for_unnamed(&state_path);
    let name = match kind {
        ProviderKind::Cloudflared => "cloudflared",
        ProviderKind::Opentunnel => "opentunnel",
    };
    map.insert("provider".into(), serde_json::Value::from(name));
    Some(kind)
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
    fn adopt_moves_the_legacy_directory_with_its_identity() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("quick-tunnel");
        std::fs::create_dir_all(legacy.join("opentunnel/webhooks")).unwrap();
        std::fs::write(legacy.join("leases.json"), "[]").unwrap();
        std::fs::write(legacy.join("leases.lock"), "").unwrap();
        let state = dir.path().join("tunnel/leases.json");
        assert!(adopt_from(&legacy, &state).unwrap());
        assert!(!legacy.exists());
        assert!(state.exists());
        assert!(dir.path().join("tunnel/opentunnel/webhooks").is_dir());
    }

    #[test]
    fn adopt_keeps_an_existing_target_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("quick-tunnel");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(dir.path().join("tunnel")).unwrap();
        let state = dir.path().join("tunnel/leases.json");
        assert!(!adopt_from(&legacy, &state).unwrap());
        assert!(legacy.is_dir());
    }

    #[test]
    fn adopt_does_nothing_without_legacy_data() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("tunnel/leases.json");
        assert!(!adopt_from(&dir.path().join("quick-tunnel"), &state).unwrap());
        assert!(!dir.path().join("tunnel").exists());
    }

    #[test]
    fn adopt_refuses_while_quick_tunnel_holds_the_lock() {
        use fs2::FileExt;
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("quick-tunnel");
        std::fs::create_dir_all(&legacy).unwrap();
        let held = std::fs::File::create(legacy.join("leases.lock")).unwrap();
        held.lock_exclusive().unwrap();
        let state = dir.path().join("tunnel/leases.json");
        assert!(adopt_from(&legacy, &state).is_err());
        assert!(legacy.is_dir());
    }

    #[test]
    fn adopt_ignores_a_custom_state_path() {
        assert!(!adopt_legacy_state(Path::new("/nonexistent/custom/leases.json")).unwrap());
    }

    #[test]
    fn new_configs_default_to_opentunnel() {
        assert_eq!(Config::default().provider, ProviderKind::Opentunnel);
        assert!(Config::default().validate().is_ok());
    }

    #[test]
    fn missing_provider_is_opentunnel_on_a_fresh_install() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("leases.json");
        assert_eq!(provider_for_unnamed(&state), ProviderKind::Opentunnel);
    }

    #[test]
    fn missing_provider_keeps_cloudflared_for_a_worker_that_already_ran() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("leases.json");
        std::fs::write(&state, "[]").unwrap();
        assert_eq!(provider_for_unnamed(&state), ProviderKind::Cloudflared);
    }

    #[test]
    fn missing_provider_keeps_opentunnel_once_its_state_directory_exists() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("leases.json");
        std::fs::write(&state, "[]").unwrap();
        std::fs::create_dir(dir.path().join("opentunnel")).unwrap();
        assert_eq!(provider_for_unnamed(&state), ProviderKind::Opentunnel);
    }

    #[test]
    fn fill_missing_provider_uses_the_configured_state_path_and_keeps_an_explicit_one() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("leases.json");
        std::fs::write(&state, "[]").unwrap();
        let mut legacy = serde_json::json!({ "state_path": state });
        assert_eq!(
            fill_missing_provider(&mut legacy),
            Some(ProviderKind::Cloudflared)
        );
        let parsed: Config = serde_json::from_value(legacy).unwrap();
        assert_eq!(parsed.provider, ProviderKind::Cloudflared);
        let mut explicit = serde_json::json!({ "provider": "opentunnel", "state_path": state });
        assert_eq!(fill_missing_provider(&mut explicit), None);
        assert_eq!(explicit["provider"], "opentunnel");
    }

    #[test]
    fn stored_configs_without_provider_still_load() {
        let stored: Config = serde_yaml_ng::from_str(
            "targets:\n  webhooks: http://127.0.0.1:3112\ncloudflared: cloudflared\n",
        )
        .unwrap();
        assert_eq!(stored.provider, ProviderKind::Opentunnel);
        assert!(stored.validate().is_ok());
        let chosen: Config = serde_yaml_ng::from_str("provider: opentunnel\n").unwrap();
        assert_eq!(chosen.provider, ProviderKind::Opentunnel);
        assert!(serde_yaml_ng::from_str::<Config>("provider: ngrok\n").is_err());
    }

    // A `provider` in the package defaults would force every install, including
    // a cloudflared one adopted from quick-tunnel, onto that provider.
    #[test]
    fn package_defaults_leave_the_provider_to_boot_resolution() {
        let manifest: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(include_str!("../iii.worker.yaml")).unwrap();
        let config = manifest["config"].as_mapping().expect("config mapping");
        assert!(!config.contains_key("provider"));
        assert_eq!(
            manifest["config"]["state_path"].as_str().map(PathBuf::from),
            Some(Config::default().state_path)
        );
        // Absent keys fall back to their code defaults.
        let shipped: Config = serde_yaml_ng::from_value(manifest["config"].clone()).unwrap();
        assert_eq!(shipped.provider, ProviderKind::Opentunnel);
        assert!(shipped.validate().is_ok());
    }

    #[test]
    fn opentunnel_needs_http_origins_and_profile_ids() {
        assert!(
            opentunnel("webhooks", "http://127.0.0.1:3112")
                .validate()
                .is_ok()
        );
        assert!(opentunnel("mesh", "http://[::1]:49180").validate().is_ok());
        assert!(
            opentunnel("webhooks", "https://127.0.0.1:3112")
                .validate()
                .is_err()
        );
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
            provider: ProviderKind::Cloudflared,
            targets: BTreeMap::from([("Web_hooks".into(), "https://127.0.0.1:3112".into())]),
            ..Config::default()
        };
        assert!(https.validate().is_ok());
    }

    #[test]
    fn proxy_protocol_needs_opentunnel_listed_targets_and_one_mode_per_port() {
        assert!(Config::default().proxy_protocol.is_empty());
        let stored: Config = serde_yaml_ng::from_str(
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
