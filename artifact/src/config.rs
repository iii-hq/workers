//! Operator-facing runtime configuration.
//!
//! The authoritative value comes from the `configuration` worker at boot
//! (see [`crate::configuration`]); a `--config` YAML file, when passed, only
//! SEEDS the initial registration. Every field has a serde default so an empty
//! object yields a fully-populated config.
//!
//! Every field is read per call from the live snapshot EXCEPT `listen_addr`:
//! the share listener binds once at boot, so changing it takes a restart.

use std::net::SocketAddr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Longest share an operator may allow: 30 days, the quick-tunnel lease cap.
const HARD_MAX_SHARE_HOURS: u32 = 720;

/// Root config shape. Unknown keys are rejected so a typo'd field fails loudly
/// instead of silently running the default.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Loopback address the share listener binds, `ip:port`. Only `/s/<token>`
    /// is served there. Point the quick-tunnel target named by `tunnel_id` at
    /// it to publish shares. Must be a loopback IP; takes effect on restart.
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,

    /// quick-tunnel target id public shares are published through. The
    /// operator maps it to `http://<listen_addr>` in the quick-tunnel
    /// configuration; this worker never chooses an origin itself.
    #[serde(default = "default_tunnel_id")]
    pub tunnel_id: String,

    /// Lifetime of a share when the caller names none, in hours.
    #[serde(default = "default_share_hours")]
    pub default_share_hours: u32,

    /// Longest lifetime a caller may ask for, in hours (at most 720).
    #[serde(default = "default_max_share_hours")]
    pub max_share_hours: u32,

    /// Largest artifact content accepted per version, in bytes.
    #[serde(default = "default_max_content_bytes")]
    pub max_content_bytes: usize,

    /// Most versions one artifact keeps; an update past it is refused.
    #[serde(default = "default_max_versions")]
    pub max_versions: u32,

    /// Most records `artifact::list` returns in one response.
    #[serde(default = "default_max_list")]
    pub max_list: usize,

    /// How long `artifact::share` waits for a starting tunnel to report a
    /// public URL before answering without one, in milliseconds.
    #[serde(default = "default_tunnel_ready_timeout_ms")]
    pub tunnel_ready_timeout_ms: u64,

    /// Public DNS resolver (`ip:port`) asked whether a new tunnel hostname
    /// resolves before its link is handed out. Empty disables the check.
    /// Takes effect on restart.
    #[serde(default = "default_dns_check_resolver")]
    pub dns_check_resolver: String,

    /// Minutes the tunnel stays up after the last public share ends, so a
    /// new share soon after reuses the same, already-resolving address
    /// (at most 1440). 0 releases it at once.
    #[serde(default = "default_tunnel_grace_minutes")]
    pub tunnel_grace_minutes: u32,
}

fn default_listen_addr() -> String {
    "127.0.0.1:3120".to_string()
}

fn default_tunnel_id() -> String {
    "artifacts".to_string()
}

fn default_share_hours() -> u32 {
    24
}

fn default_max_share_hours() -> u32 {
    72
}

fn default_max_content_bytes() -> usize {
    2 * 1024 * 1024
}

fn default_max_versions() -> u32 {
    100
}

fn default_max_list() -> usize {
    200
}

fn default_tunnel_ready_timeout_ms() -> u64 {
    45_000
}

fn default_dns_check_resolver() -> String {
    "1.1.1.1:53".to_string()
}

fn default_tunnel_grace_minutes() -> u32 {
    30
}

/// Longest tunnel grace an operator may set: one day.
const MAX_TUNNEL_GRACE_MINUTES: u32 = 1440;

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            listen_addr: default_listen_addr(),
            tunnel_id: default_tunnel_id(),
            default_share_hours: default_share_hours(),
            max_share_hours: default_max_share_hours(),
            max_content_bytes: default_max_content_bytes(),
            max_versions: default_max_versions(),
            max_list: default_max_list(),
            tunnel_ready_timeout_ms: default_tunnel_ready_timeout_ms(),
            dns_check_resolver: default_dns_check_resolver(),
            tunnel_grace_minutes: default_tunnel_grace_minutes(),
        }
    }
}

impl WorkerConfig {
    /// Parse a seed config from YAML, expanding `${NAME}` against the process
    /// env FIRST, then deserializing.
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        let expanded = expand_env(yaml);
        let parsed: Self =
            serde_yaml_ng::from_str(&expanded).map_err(|e| format!("yaml parse: {e}"))?;
        parsed.validate()
    }

    /// Read and parse a YAML seed file (env-expanded).
    pub fn from_file(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::from_yaml(&raw)
    }

    /// Parse a config already env-expanded by the configuration worker.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let parsed: Self =
            serde_json::from_value(value.clone()).map_err(|e| format!("json parse: {e}"))?;
        parsed.validate()
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("WorkerConfig serializes")
    }

    /// The parsed listener address. Infallible on a validated config.
    pub fn listen_socket(&self) -> Result<SocketAddr, String> {
        let addr: SocketAddr = self
            .listen_addr
            .parse()
            .map_err(|_| format!("listen_addr `{}` is not ip:port", self.listen_addr))?;
        if !addr.ip().is_loopback() {
            return Err(format!(
                "listen_addr `{}` must be a loopback IP: shares are published only through \
                 the operator's quick-tunnel target",
                self.listen_addr
            ));
        }
        Ok(addr)
    }

    /// Reject values that parse but cannot mean anything.
    fn validate(self) -> Result<Self, String> {
        self.listen_socket()?;
        if !valid_tunnel_id(&self.tunnel_id) {
            return Err(format!(
                "tunnel_id `{}` must be 1..=63 lowercase letters, digits or `-`",
                self.tunnel_id
            ));
        }
        if self.max_share_hours == 0 || self.max_share_hours > HARD_MAX_SHARE_HOURS {
            return Err(format!(
                "max_share_hours must be between 1 and {HARD_MAX_SHARE_HOURS}"
            ));
        }
        if self.default_share_hours == 0 || self.default_share_hours > self.max_share_hours {
            return Err("default_share_hours must be between 1 and max_share_hours".to_string());
        }
        if self.max_content_bytes == 0 {
            return Err("max_content_bytes must be at least 1".to_string());
        }
        if self.max_versions == 0 {
            return Err("max_versions must be at least 1".to_string());
        }
        if self.max_list == 0 {
            return Err("max_list must be at least 1".to_string());
        }
        if self.tunnel_grace_minutes > MAX_TUNNEL_GRACE_MINUTES {
            return Err(format!(
                "tunnel_grace_minutes must be at most {MAX_TUNNEL_GRACE_MINUTES}"
            ));
        }
        crate::dns::Dns::new(&self.dns_check_resolver)?;
        Ok(self)
    }

    /// The JSON Schema registered with the `configuration` worker, with the
    /// shipped defaults attached as a top-level `example`.
    pub fn json_schema() -> Value {
        let root = schemars::schema_for!(WorkerConfig);
        let mut schema =
            serde_json::to_value(&root.schema).expect("WorkerConfig JSON Schema serializes");
        if let Some(obj) = schema.as_object_mut() {
            if !root.definitions.is_empty() {
                obj.insert(
                    "definitions".into(),
                    serde_json::to_value(&root.definitions).expect("definitions serialize"),
                );
            }
            obj.insert("example".into(), WorkerConfig::default().to_json());
        }
        schema
    }
}

fn valid_tunnel_id(id: &str) -> bool {
    (1..=63).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Expand `${NAME}` and `${NAME:default}` against the process env. An unset
/// variable with no default expands to the empty string, matching the
/// configuration worker's own expansion.
fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let spec = &after[..end];
                let (name, fallback) = match spec.split_once(':') {
                    Some((n, d)) => (n, Some(d)),
                    None => (spec, None),
                };
                match (std::env::var(name), fallback) {
                    (Ok(v), _) => out.push_str(&v),
                    (Err(_), Some(d)) => out.push_str(d),
                    (Err(_), None) => {
                        tracing::warn!(var = %name, "config references undefined env var")
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str("${");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_yaml_yields_defaults() {
        let cfg = WorkerConfig::from_yaml("{}").expect("empty object parses");
        assert_eq!(cfg, WorkerConfig::default());
    }

    #[test]
    fn unknown_field_is_rejected() {
        let err = WorkerConfig::from_yaml("max_lizt: 10\n").expect_err("typo must fail loudly");
        assert!(err.contains("max_lizt"), "{err}");
    }

    #[test]
    fn a_non_loopback_listener_is_rejected() {
        let err = WorkerConfig::from_yaml("listen_addr: 0.0.0.0:3120\n").expect_err("public bind");
        assert!(err.contains("loopback"), "{err}");
    }

    #[test]
    fn an_ephemeral_loopback_port_is_accepted() {
        let cfg = WorkerConfig::from_yaml("listen_addr: 127.0.0.1:0\n").expect("port 0");
        assert_eq!(cfg.listen_socket().expect("addr").port(), 0);
    }

    #[test]
    fn share_hours_must_fit_under_the_ceiling() {
        for bad in [
            "max_share_hours: 0\n",
            "max_share_hours: 721\n",
            "default_share_hours: 100\n",
            "default_share_hours: 0\n",
        ] {
            assert!(WorkerConfig::from_yaml(bad).is_err(), "{bad} should fail");
        }
    }

    #[test]
    fn a_malformed_tunnel_id_is_rejected() {
        let err = WorkerConfig::from_yaml("tunnel_id: Artifacts_1\n").expect_err("bad id");
        assert!(err.contains("tunnel_id"), "{err}");
    }

    #[test]
    fn zero_limits_are_rejected_on_both_parse_paths() {
        for bad in [
            "max_content_bytes: 0\n",
            "max_list: 0\n",
            "max_versions: 0\n",
        ] {
            assert!(WorkerConfig::from_yaml(bad).is_err(), "{bad} should fail");
        }
        assert!(WorkerConfig::from_json(&serde_json::json!({ "max_list": 0 })).is_err());
    }

    #[test]
    fn tunnel_grace_and_dns_resolver_are_validated() {
        assert!(WorkerConfig::from_yaml("tunnel_grace_minutes: 1441\n").is_err());
        assert!(WorkerConfig::from_yaml("dns_check_resolver: nope\n").is_err());
        let off = WorkerConfig::from_yaml("dns_check_resolver: ''\ntunnel_grace_minutes: 0\n")
            .expect("both may be turned off");
        assert_eq!(off.tunnel_grace_minutes, 0);
    }

    #[test]
    fn json_round_trips() {
        let cfg = WorkerConfig {
            max_list: 7,
            ..WorkerConfig::default()
        };
        assert_eq!(
            WorkerConfig::from_json(&cfg.to_json()).expect("parses"),
            cfg
        );
    }

    #[test]
    fn schema_carries_defaults_as_example() {
        let schema = WorkerConfig::json_schema();
        assert_eq!(schema["example"], WorkerConfig::default().to_json());
        assert!(schema["properties"]["tunnel_id"]["description"].is_string());
    }
}
