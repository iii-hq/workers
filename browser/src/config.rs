//! Config for the browser worker. Numeric caps are HARD ceilings — a caller
//! may ask for less per call, never more. Buffer sizes, timeouts, and the
//! scheme allowlist are read per call, so they hot-reload. `executable`,
//! `headless`, and the viewport are read at session start, so a change
//! applies to sessions started after it — running sessions keep the browser
//! they were launched with.

use std::collections::BTreeMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub type SharedConfig = Arc<ArcSwap<WorkerConfig>>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SecurityMode {
    #[default]
    Safe,
    Compat,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ScraplingDefaults {
    pub impersonate: String,
    pub headless: bool,
    pub network_idle: bool,
    pub proxy: String,
    pub include_html: bool,
}

impl Default for ScraplingDefaults {
    fn default() -> Self {
        Self {
            impersonate: "chrome".to_string(),
            headless: true,
            network_idle: false,
            proxy: String::new(),
            include_html: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ScraplingConfig {
    pub security_mode: SecurityMode,
    pub chromium_executable: String,
    pub allow_loopback: bool,
    pub defaults: ScraplingDefaults,
    pub max_bulk_concurrency: u64,
    pub max_sessions: u64,
    pub session_idle_timeout_s: u64,
    pub adaptive_storage_path: String,
    pub adaptive_max_bytes: u64,
    /// Append the browser::* scraping guidance to agent system prompts.
    /// Hot-applies: flipping it in the console binds/unbinds the
    /// pre-generate hook live, no restart (same knob the Python scrapling
    /// worker and the fp/web workers carry).
    pub inject_guidance: bool,
}

impl Default for ScraplingConfig {
    fn default() -> Self {
        Self {
            security_mode: SecurityMode::Safe,
            chromium_executable: String::new(),
            allow_loopback: false,
            defaults: ScraplingDefaults::default(),
            max_bulk_concurrency: 5,
            max_sessions: 8,
            session_idle_timeout_s: 900,
            adaptive_storage_path: "./data/scrapling/elements.db".to_string(),
            adaptive_max_bytes: 268_435_456,
            inject_guidance: true,
        }
    }
}

impl ScraplingConfig {
    pub fn startup_snapshot(&self) -> ScraplingStartupConfig {
        ScraplingStartupConfig {
            max_sessions: self.max_sessions,
            session_idle_timeout_s: self.session_idle_timeout_s,
            adaptive_storage_path: iii_worker_paths::resolve_path(&self.adaptive_storage_path)
                .to_string_lossy()
                .into_owned(),
        }
    }

    pub fn adaptive_quota(&self) -> Option<u64> {
        (self.security_mode == SecurityMode::Safe).then_some(self.adaptive_max_bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScraplingStartupConfig {
    pub max_sessions: u64,
    pub session_idle_timeout_s: u64,
    pub adaptive_storage_path: String,
}

pub const fn scrapling_compat_supported() -> bool {
    cfg!(all(
        feature = "scrapling-compat",
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OriginPolicyDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OriginPolicy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access: Option<OriginPolicyDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloads: Option<OriginPolicyDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploads: Option<OriginPolicyDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scripting: Option<OriginPolicyDecision>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedOriginPolicy {
    pub access: bool,
    pub downloads: bool,
    pub uploads: bool,
    pub scripting: bool,
}

impl Default for ResolvedOriginPolicy {
    fn default() -> Self {
        Self {
            access: true,
            downloads: true,
            uploads: true,
            scripting: true,
        }
    }
}

impl OriginPolicy {
    fn resolve(&self) -> ResolvedOriginPolicy {
        let allowed = |decision| decision != Some(OriginPolicyDecision::Deny);
        ResolvedOriginPolicy {
            access: allowed(self.access),
            downloads: allowed(self.downloads),
            uploads: allowed(self.uploads),
            scripting: allowed(self.scripting),
        }
    }
}

fn configured_origin_policy_for<'a>(
    config: &'a WorkerConfig,
    raw_url: &str,
) -> Option<&'a OriginPolicy> {
    let policies = config.origin_policies.as_ref()?;
    let parsed = url::Url::parse(raw_url).ok()?;
    let origin = parsed.origin().ascii_serialization();
    let host = parsed.host_str();
    let mut host_match = None;

    for (key, policy) in policies {
        if let Ok(key_url) = url::Url::parse(key) {
            if key_url.host_str().is_some() {
                if origin != "null" && key_url.origin().ascii_serialization() == origin {
                    return Some(policy);
                }
                continue;
            }
        }
        if host.is_some_and(|request_host| key.eq_ignore_ascii_case(request_host)) {
            host_match = Some(policy);
        }
    }

    host_match
}

pub fn origin_policy_for(config: &WorkerConfig, raw_url: &str) -> ResolvedOriginPolicy {
    configured_origin_policy_for(config, raw_url)
        .or(config.default_origin_policy.as_ref())
        .map(OriginPolicy::resolve)
        .unwrap_or_default()
}

pub fn origin_policy_config_key_for(config: &WorkerConfig, raw_url: &str) -> &'static str {
    if configured_origin_policy_for(config, raw_url).is_some() {
        "origin_policies"
    } else {
        "default_origin_policy"
    }
}

pub fn origin_label(raw_url: &str) -> String {
    let Ok(parsed) = url::Url::parse(raw_url) else {
        return raw_url.to_string();
    };
    let origin = parsed.origin().ascii_serialization();
    if origin == "null" {
        raw_url.to_string()
    } else {
        origin
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WorkerConfig {
    /// Absolute path to a Chromium/Chrome executable. Empty string
    /// auto-detects a system install (Chrome, Chromium, Edge).
    pub executable: String,
    /// Chromium profile directory. Empty string is ephemeral (fresh profile
    /// per session); set a path to persist cookies, localStorage, and logins
    /// across sessions. All sessions share the one profile.
    pub user_data_dir: String,
    /// Launch sessions headless. Set false to watch a real window locally.
    pub headless: bool,
    /// Maximum concurrently running sessions; `sessions::start` beyond this
    /// fails until one stops.
    pub max_sessions: u64,
    /// Per-session console ring-buffer capacity (entries).
    pub console_buffer: u64,
    /// Per-session network ring-buffer capacity (entries).
    pub network_buffer: u64,
    /// Viewport width for new sessions (pixels).
    pub viewport_width: u32,
    /// Viewport height for new sessions (pixels).
    pub viewport_height: u32,
    /// Timeout used when the caller omits `timeout_ms` (navigation and
    /// evaluate, which take a caller `timeout_ms`).
    pub default_timeout_ms: u64,
    /// Hard ceiling on per-call timeout; caller `timeout_ms` is clamped DOWN
    /// to this.
    pub max_timeout_ms: u64,
    /// Stop sessions idle longer than this (ms). 0 disables the sweep.
    pub idle_stop_ms: u64,
    /// JPEG quality for `browser::screenshot` (1-100).
    pub screenshot_quality: u64,
    /// URL schemes `browser::navigate` accepts.
    ///
    /// `file` ships enabled so a local document can be opened and rendered —
    /// the path `document::ocr` takes to read a scanned PDF, which has nowhere
    /// else to get pixels from. Note what that permits: unlike the workers that
    /// read files directly, navigation is not checked against a session's
    /// filesystem scope, so any caller that reaches `browser::navigate` can
    /// open any file this process can read. Narrow the list on a shared or
    /// multi-tenant machine.
    pub allowed_schemes: Vec<String>,
    /// Maximum nodes serialized by `browser::snapshot` before truncation.
    pub max_snapshot_nodes: u64,
    /// Fallback access policy when no exact-origin or bare-host entry matches.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_origin_policy: Option<OriginPolicy>,
    /// Exact-origin and bare-host policies. Exact origins take precedence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_policies: Option<BTreeMap<String, OriginPolicy>>,
    /// Expose the session's visited-page list.
    pub allow_history_access: bool,
    /// Let callers import cookies with `browser::cookies::set`.
    pub allow_cookie_import: bool,
    /// Allow `browser::sessions::attach` to connect to an already-running
    /// browser and adopt its tabs. Off by default: attaching reaches the
    /// user's real profile with its logged-in sessions, so it is opt-in.
    pub allow_attach: bool,
    /// Native Scrapling compatibility settings. These are isolated from the
    /// interactive browser runtime above.
    pub scrapling: ScraplingConfig,
    /// Internal compatibility projection for routes that have not yet moved
    /// to the private Scrapling runtime. It is populated from `scrapling` and
    /// is deliberately absent from the public configuration schema/wire.
    #[serde(skip)]
    #[schemars(skip)]
    pub allow_loopback: bool,
    /// Internal compatibility projection; see `allow_loopback`.
    #[serde(skip)]
    #[schemars(skip)]
    pub max_bulk_concurrency: u64,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            executable: String::new(),
            user_data_dir: String::new(),
            headless: true,
            max_sessions: 4,
            console_buffer: 500,
            network_buffer: 500,
            viewport_width: 1280,
            viewport_height: 800,
            default_timeout_ms: 30_000,
            max_timeout_ms: 120_000,
            idle_stop_ms: 300_000,
            screenshot_quality: 60,
            allowed_schemes: vec!["http".to_string(), "https".to_string(), "file".to_string()],
            max_snapshot_nodes: 2_000,
            default_origin_policy: None,
            origin_policies: None,
            allow_history_access: true,
            allow_cookie_import: true,
            allow_attach: false,
            scrapling: ScraplingConfig::default(),
            allow_loopback: false,
            max_bulk_concurrency: 5,
        }
    }
}

impl WorkerConfig {
    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(WorkerConfig))
            .expect("WorkerConfig schema serializes")
    }

    /// Parse from a JSON object; missing keys fall back to defaults
    /// (`#[serde(default)]`). The configuration worker may store the value
    /// under a `browser` wrapper or flat — accept both.
    pub fn from_json(v: &serde_json::Value) -> Result<WorkerConfig, String> {
        let inner = v.get("browser").unwrap_or(v);
        let mut config: WorkerConfig = serde_json::from_value(inner.clone())
            .map_err(|e| format!("invalid browser config: {e}"))?;
        config.validate()?;
        config.sync_scrapling_projection();
        Ok(config)
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("WorkerConfig serializes")
    }

    pub fn into_shared(mut self) -> SharedConfig {
        self.sync_scrapling_projection();
        Arc::new(ArcSwap::from_pointee(self))
    }

    pub fn validate(&self) -> Result<(), String> {
        self.validate_with_compat_support(scrapling_compat_supported())
    }

    fn validate_with_compat_support(&self, compat_supported: bool) -> Result<(), String> {
        if self.scrapling.security_mode == SecurityMode::Compat && !compat_supported {
            return Err(
                "browser.scrapling.security_mode=compat is unsupported on this target; compat requires Tier-1 Linux x86_64 or aarch64"
                    .to_string(),
            );
        }
        Ok(())
    }

    fn sync_scrapling_projection(&mut self) {
        self.allow_loopback = self.scrapling.allow_loopback;
        self.max_bulk_concurrency = self.scrapling.max_bulk_concurrency;
    }

    /// Clamp a caller-supplied timeout to the configured ceiling, defaulting
    /// when omitted.
    pub fn clamp_timeout(&self, requested: Option<u64>) -> u64 {
        requested
            .unwrap_or(self.default_timeout_ms)
            .min(self.max_timeout_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let c = WorkerConfig::default();
        assert_eq!(c.executable, "");
        assert_eq!(c.user_data_dir, "");
        assert!(c.headless);
        assert_eq!(c.max_sessions, 4);
        assert_eq!(c.console_buffer, 500);
        assert_eq!(c.network_buffer, 500);
        assert_eq!(c.viewport_width, 1280);
        assert_eq!(c.viewport_height, 800);
        assert_eq!(c.default_timeout_ms, 30_000);
        assert_eq!(c.max_timeout_ms, 120_000);
        assert_eq!(c.idle_stop_ms, 300_000);
        assert_eq!(c.screenshot_quality, 60);
        assert_eq!(c.allowed_schemes, vec!["http", "https", "file"]);
        assert_eq!(c.max_snapshot_nodes, 2_000);
        assert!(c.default_origin_policy.is_none());
        assert!(c.origin_policies.is_none());
        assert!(c.allow_history_access);
        assert!(c.allow_cookie_import);
        assert!(!c.allow_attach);
        assert_eq!(c.scrapling.security_mode, SecurityMode::Safe);
        assert_eq!(c.scrapling.chromium_executable, "");
        assert!(!c.scrapling.allow_loopback);
        assert_eq!(c.scrapling.defaults.impersonate, "chrome");
        assert!(c.scrapling.defaults.headless);
        assert!(!c.scrapling.defaults.network_idle);
        assert_eq!(c.scrapling.defaults.proxy, "");
        assert!(!c.scrapling.defaults.include_html);
        assert_eq!(c.scrapling.max_bulk_concurrency, 5);
        assert_eq!(c.scrapling.max_sessions, 8);
        assert_eq!(c.scrapling.session_idle_timeout_s, 900);
        assert_eq!(
            c.scrapling.adaptive_storage_path,
            "./data/scrapling/elements.db"
        );
        assert_eq!(c.scrapling.adaptive_max_bytes, 268_435_456);
        assert_eq!(c.scrapling.adaptive_quota(), Some(268_435_456));
    }

    #[test]
    fn compat_keeps_the_oracles_unbounded_adaptive_storage() {
        let config = ScraplingConfig {
            security_mode: SecurityMode::Compat,
            ..ScraplingConfig::default()
        };
        assert_eq!(config.adaptive_quota(), None);
    }

    #[test]
    fn json_roundtrip() {
        let c = WorkerConfig {
            headless: false,
            max_sessions: 2,
            ..WorkerConfig::default()
        };
        let back = WorkerConfig::from_json(&c.to_json()).unwrap();
        assert!(!back.headless);
        assert_eq!(back.max_sessions, 2);
    }

    #[test]
    fn from_json_fills_missing_with_defaults() {
        let v = serde_json::json!({ "max_sessions": 8 });
        let c = WorkerConfig::from_json(&v).unwrap();
        assert_eq!(c.max_sessions, 8);
        assert_eq!(c.console_buffer, 500);
    }

    #[test]
    fn from_json_accepts_wrapped_value() {
        let v = serde_json::json!({ "browser": { "headless": false } });
        let c = WorkerConfig::from_json(&v).unwrap();
        assert!(!c.headless);
    }

    #[test]
    fn from_json_parses_origin_and_surface_policies() {
        let config = WorkerConfig::from_json(&serde_json::json!({
            "default_origin_policy": {"access": "deny"},
            "origin_policies": {
                "example.com": {"access": "allow", "uploads": "deny"}
            },
            "allow_history_access": false,
            "allow_cookie_import": false
        }))
        .unwrap();

        let policy = origin_policy_for(&config, "https://example.com/path");
        assert!(policy.access);
        assert!(!policy.uploads);
        assert!(!config.allow_history_access);
        assert!(!config.allow_cookie_import);
    }

    #[test]
    fn nested_scrapling_values_parse_without_changing_interactive_values() {
        let value = serde_json::json!({
            "browser": {
                "headless": false,
                "max_sessions": 3,
                "scrapling": {
                    "allow_loopback": true,
                    "max_bulk_concurrency": 2,
                    "max_sessions": 7,
                    "session_idle_timeout_s": 45,
                    "adaptive_storage_path": "/tmp/scrapling-test.db",
                    "adaptive_max_bytes": 1024,
                    "defaults": {
                        "impersonate": "firefox",
                        "headless": false,
                        "network_idle": true,
                        "proxy": "http://proxy.test:8080",
                        "include_html": true
                    }
                }
            }
        });

        let config = WorkerConfig::from_json(&value).unwrap();
        assert!(!config.headless);
        assert_eq!(config.max_sessions, 3);
        assert!(config.scrapling.allow_loopback);
        assert_eq!(config.scrapling.max_bulk_concurrency, 2);
        assert_eq!(config.scrapling.max_sessions, 7);
        assert_eq!(config.scrapling.session_idle_timeout_s, 45);
        assert_eq!(
            config.scrapling.adaptive_storage_path,
            "/tmp/scrapling-test.db"
        );
        assert_eq!(config.scrapling.adaptive_max_bytes, 1024);
        assert_eq!(config.scrapling.defaults.impersonate, "firefox");
        assert!(!config.scrapling.defaults.headless);
        assert!(config.scrapling.defaults.network_idle);
        assert_eq!(config.scrapling.defaults.proxy, "http://proxy.test:8080");
        assert!(config.scrapling.defaults.include_html);
        assert!(config.allow_loopback);
        assert_eq!(config.max_bulk_concurrency, 2);
    }

    #[test]
    fn serialized_config_exposes_scrapling_settings_only_in_nested_block() {
        let value = WorkerConfig::default().to_json();
        assert!(value.get("scrapling").is_some());
        assert!(value.get("allow_loopback").is_none());
        assert!(value.get("max_bulk_concurrency").is_none());
    }

    #[test]
    fn startup_snapshot_owns_frozen_session_and_adaptive_values() {
        let mut config = WorkerConfig::default();
        config.scrapling.max_sessions = 6;
        config.scrapling.session_idle_timeout_s = 123;
        config.scrapling.adaptive_storage_path = "/tmp/first.db".to_string();

        let snapshot = config.scrapling.startup_snapshot();
        config.scrapling.max_sessions = 9;
        config.scrapling.session_idle_timeout_s = 456;
        config.scrapling.adaptive_storage_path = "/tmp/second.db".to_string();

        assert_eq!(snapshot.max_sessions, 6);
        assert_eq!(snapshot.session_idle_timeout_s, 123);
        assert_eq!(snapshot.adaptive_storage_path, "/tmp/first.db");
    }

    #[test]
    fn compat_is_explicitly_rejected_when_target_is_not_tier_one() {
        let mut config = WorkerConfig::default();
        config.scrapling.security_mode = SecurityMode::Compat;
        let error = config.validate_with_compat_support(false).unwrap_err();
        assert_eq!(
            error,
            "browser.scrapling.security_mode=compat is unsupported on this target; compat requires Tier-1 Linux x86_64 or aarch64"
        );
    }

    #[test]
    fn compat_validation_tracks_the_compiled_target() {
        let result = WorkerConfig::from_json(&serde_json::json!({
            "scrapling": {"security_mode": "compat"}
        }));
        if scrapling_compat_supported() {
            assert_eq!(
                result.unwrap().scrapling.security_mode,
                SecurityMode::Compat
            );
        } else {
            assert_eq!(
                result.unwrap_err(),
                "browser.scrapling.security_mode=compat is unsupported on this target; compat requires Tier-1 Linux x86_64 or aarch64"
            );
        }
    }

    #[test]
    fn safe_mode_is_accepted_on_every_target() {
        WorkerConfig::default()
            .validate_with_compat_support(false)
            .unwrap();
    }

    #[test]
    fn unknown_security_mode_is_rejected() {
        let error = WorkerConfig::from_json(&serde_json::json!({
            "scrapling": {"security_mode": "unsafe"}
        }))
        .unwrap_err();
        assert!(error.contains("unknown variant `unsafe`"), "{error}");
    }

    #[test]
    fn clamp_timeout_defaults_and_ceils() {
        let c = WorkerConfig::default();
        assert_eq!(c.clamp_timeout(None), 30_000);
        assert_eq!(c.clamp_timeout(Some(5_000)), 5_000);
        assert_eq!(c.clamp_timeout(Some(600_000)), 120_000);
    }

    fn policy(access: OriginPolicyDecision) -> OriginPolicy {
        OriginPolicy {
            access: Some(access),
            ..OriginPolicy::default()
        }
    }

    #[test]
    fn origin_policy_exact_origin_precedes_host_and_default() {
        let config = WorkerConfig {
            default_origin_policy: Some(policy(OriginPolicyDecision::Deny)),
            origin_policies: Some(BTreeMap::from([
                (
                    "app.example.com".to_string(),
                    policy(OriginPolicyDecision::Deny),
                ),
                (
                    "https://app.example.com:8443".to_string(),
                    policy(OriginPolicyDecision::Allow),
                ),
            ])),
            ..WorkerConfig::default()
        };

        assert!(origin_policy_for(&config, "https://app.example.com:8443/a").access);
        assert!(!origin_policy_for(&config, "https://app.example.com/a").access);
        assert!(!origin_policy_for(&config, "https://other.example.com/a").access);
    }

    #[test]
    fn origin_policy_exact_origin_includes_port_and_scheme() {
        let config = WorkerConfig {
            origin_policies: Some(BTreeMap::from([(
                "https://app.example.com:8443".to_string(),
                policy(OriginPolicyDecision::Deny),
            )])),
            ..WorkerConfig::default()
        };

        assert!(!origin_policy_for(&config, "https://app.example.com:8443/a").access);
        assert!(origin_policy_for(&config, "https://app.example.com/a").access);
        assert!(origin_policy_for(&config, "http://app.example.com:8443/a").access);
    }

    #[test]
    fn origin_policy_bare_host_matches_any_scheme_or_port() {
        let config = WorkerConfig {
            origin_policies: Some(BTreeMap::from([(
                "app.example.com".to_string(),
                policy(OriginPolicyDecision::Deny),
            )])),
            ..WorkerConfig::default()
        };

        assert!(!origin_policy_for(&config, "https://app.example.com/a").access);
        assert!(!origin_policy_for(&config, "http://app.example.com:8080/a").access);
    }

    #[test]
    fn origin_policy_bare_host_is_case_insensitive() {
        let config = WorkerConfig {
            origin_policies: Some(BTreeMap::from([(
                "App.Example.com".to_string(),
                policy(OriginPolicyDecision::Deny),
            )])),
            ..WorkerConfig::default()
        };

        assert!(!origin_policy_for(&config, "https://app.example.com/a").access);
    }

    #[test]
    fn origin_policy_origin_drops_an_explicit_default_port() {
        let config = WorkerConfig {
            origin_policies: Some(BTreeMap::from([(
                "https://app.example.com:443".to_string(),
                policy(OriginPolicyDecision::Deny),
            )])),
            ..WorkerConfig::default()
        };

        assert!(!origin_policy_for(&config, "https://app.example.com/a").access);
    }

    #[test]
    fn origin_policy_unparseable_url_falls_back_to_default() {
        let config = WorkerConfig {
            default_origin_policy: Some(OriginPolicy {
                uploads: Some(OriginPolicyDecision::Deny),
                ..OriginPolicy::default()
            }),
            origin_policies: Some(BTreeMap::from([(
                "not a url".to_string(),
                OriginPolicy::default(),
            )])),
            ..WorkerConfig::default()
        };

        let resolved = origin_policy_for(&config, "not a url");
        assert!(resolved.access);
        assert!(resolved.downloads);
        assert!(!resolved.uploads);
        assert!(resolved.scripting);
    }

    #[test]
    fn omitted_policy_fields_default_to_allow() {
        let config = WorkerConfig {
            default_origin_policy: Some(OriginPolicy {
                access: Some(OriginPolicyDecision::Deny),
                ..OriginPolicy::default()
            }),
            ..WorkerConfig::default()
        };

        assert_eq!(
            origin_policy_for(&config, "https://example.com"),
            ResolvedOriginPolicy {
                access: false,
                downloads: true,
                uploads: true,
                scripting: true,
            }
        );
    }

    #[test]
    fn schema_lists_fields() {
        let s = WorkerConfig::json_schema();
        let props = &s["properties"];
        assert!(props.get("executable").is_some());
        assert!(props.get("headless").is_some());
        assert!(props.get("allowed_schemes").is_some());
        assert!(props.get("default_origin_policy").is_some());
        assert!(props.get("origin_policies").is_some());
        assert!(props.get("allow_history_access").is_some());
        assert!(props.get("allow_cookie_import").is_some());
        let scrapling = &props["scrapling"];
        assert_eq!(
            scrapling["default"],
            serde_json::to_value(WorkerConfig::default().scrapling).unwrap()
        );
        assert_eq!(
            scrapling["allOf"][0]["$ref"],
            "#/definitions/ScraplingConfig"
        );
        let scrapling_properties = &s["definitions"]["ScraplingConfig"]["properties"];
        let names: std::collections::BTreeSet<_> = scrapling_properties
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            names,
            std::collections::BTreeSet::from([
                "adaptive_max_bytes",
                "adaptive_storage_path",
                "allow_loopback",
                "chromium_executable",
                "defaults",
                "inject_guidance",
                "max_bulk_concurrency",
                "max_sessions",
                "security_mode",
                "session_idle_timeout_s",
            ])
        );
        assert!(props.get("allow_loopback").is_none());
        assert!(props.get("max_bulk_concurrency").is_none());
    }
}
