use std::collections::{HashMap, HashSet};
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use image::codecs::png::{CompressionType, FilterType as PngFilterType, PngEncoder};
use image::imageops::FilterType;
use image::{GenericImageView, ImageEncoder};
use serde_json::{json, Map, Value};

use crate::config::{SecurityMode, WorkerConfig};
use crate::scrapling::cdp::{CdpClient, CdpEvent, CdpSession, EventReceiver};
use crate::scrapling::egress_gate::EgressGate;
use crate::scrapling::page::PageData;
use crate::ssrf::SsrfPolicy;

/// The frozen Tier-1 Chromium builds compat mode certifies against: the
/// x86_64 Chrome-for-Testing 148 build and the aarch64 Playwright chromium
/// build 1223 (same 148 milestone; Playwright snapshots report patch .0).
/// Both are pinned by sha256 in oracle/manifest.json and fetched by
/// scripts/fetch_chromium_artifacts.sh.
pub(crate) const CERTIFIED_CHROME_VERSIONS: &[&str] = &["148.0.7778.96", "148.0.7778.0"];
const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36";
const GOOGLE_REFERER: &str = "https://www.google.com/";
const MAX_TILE_WIDTH: u32 = 1024;
const MAX_TILE_HEIGHT: u32 = 1536;
const MAX_TILES: usize = 6;

const DEFAULT_ARGS: &[&str] = &[
    "--no-pings",
    "--no-first-run",
    "--disable-infobars",
    "--disable-breakpad",
    "--no-service-autorun",
    "--homepage=about:blank",
    "--password-store=basic",
    "--disable-hang-monitor",
    "--no-default-browser-check",
    "--disable-session-crashed-bubble",
    "--disable-search-engine-choice-screen",
];

const PLAYWRIGHT_ARGS: &[&str] = &[
    "--disable-field-trial-config",
    "--disable-background-networking",
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-back-forward-cache",
    "--disable-breakpad",
    "--disable-client-side-phishing-detection",
    "--disable-component-extensions-with-background-pages",
    "--no-default-browser-check",
    "--disable-dev-shm-usage",
    "--disable-edgeupdater",
    "--disable-features=AvoidUnnecessaryBeforeUnloadCheckSync,BoundaryEventDispatchTracksNodeRemoval,DestroyProfileOnBrowserClose,DialMediaRouteProvider,GlobalMediaControls,HttpsUpgrades,LensOverlay,MediaRouter,PaintHolding,ThirdPartyStoragePartitioning,Translate,AutoDeElevate,RenderDocument,OptimizationHints,msForceBrowserSignIn,msEdgeUpdateLaunchServicesPreferredVersion",
    "--enable-features=CDPScreenshotNewSurface",
    "--allow-pre-commit-input",
    "--disable-hang-monitor",
    "--disable-ipc-flooding-protection",
    "--disable-prompt-on-repost",
    "--disable-renderer-backgrounding",
    "--force-color-profile=srgb",
    "--metrics-recording-only",
    "--no-first-run",
    "--password-store=basic",
    "--use-mock-keychain",
    "--no-service-autorun",
    "--export-tagged-pdf",
    "--disable-search-engine-choice-screen",
    "--unsafely-disable-devtools-self-xss-warnings",
    "--edge-skip-compat-layer-relaunch",
    "--disable-infobars",
    "--disable-sync",
    "--enable-unsafe-swiftshader",
];

const PATCHRIGHT_ARGS: &[&str] = &[
    "--disable-field-trial-config",
    "--disable-background-networking",
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-breakpad",
    "--no-default-browser-check",
    "--disable-dev-shm-usage",
    "--disable-edgeupdater",
    "--disable-features=AvoidUnnecessaryBeforeUnloadCheckSync,BoundaryEventDispatchTracksNodeRemoval,DestroyProfileOnBrowserClose,DialMediaRouteProvider,GlobalMediaControls,HttpsUpgrades,LensOverlay,MediaRouter,PaintHolding,ThirdPartyStoragePartitioning,Translate,AutoDeElevate,RenderDocument,OptimizationHints,msForceBrowserSignIn,msEdgeUpdateLaunchServicesPreferredVersion",
    "--enable-features=CDPScreenshotNewSurface",
    "--disable-hang-monitor",
    "--disable-prompt-on-repost",
    "--disable-renderer-backgrounding",
    "--force-color-profile=srgb",
    "--no-first-run",
    "--password-store=basic",
    "--use-mock-keychain",
    "--no-service-autorun",
    "--export-tagged-pdf",
    "--disable-search-engine-choice-screen",
    "--edge-skip-compat-layer-relaunch",
    "--disable-infobars",
    "--disable-search-engine-choice-screen",
    "--disable-sync",
    "--disable-blink-features=AutomationControlled",
];

const STEALTH_ARGS: &[&str] = &[
    "--test-type",
    "--lang=en-US",
    "--mute-audio",
    "--disable-sync",
    "--hide-scrollbars",
    "--disable-logging",
    "--start-maximized",
    "--enable-async-dns",
    "--accept-lang=en-US",
    "--use-mock-keychain",
    "--disable-translate",
    "--disable-voice-input",
    "--window-position=0,0",
    "--disable-wake-on-wifi",
    "--ignore-gpu-blocklist",
    "--enable-tcp-fast-open",
    "--enable-web-bluetooth",
    "--disable-cloud-import",
    "--disable-print-preview",
    "--disable-dev-shm-usage",
    "--metrics-recording-only",
    "--disable-crash-reporter",
    "--disable-partial-raster",
    "--disable-gesture-typing",
    "--disable-checker-imaging",
    "--disable-prompt-on-repost",
    "--force-color-profile=srgb",
    "--font-render-hinting=none",
    "--aggressive-cache-discard",
    "--disable-cookie-encryption",
    "--disable-domain-reliability",
    "--disable-threaded-animation",
    "--disable-threaded-scrolling",
    "--enable-simple-cache-backend",
    "--disable-background-networking",
    "--enable-surface-synchronization",
    "--disable-image-animation-resync",
    "--disable-renderer-backgrounding",
    "--disable-ipc-flooding-protection",
    "--prerender-from-omnibox=disabled",
    "--safebrowsing-disable-auto-update",
    "--disable-offer-upload-credit-cards",
    "--disable-background-timer-throttling",
    "--disable-new-content-rendering-timeout",
    "--run-all-compositor-stages-before-draw",
    "--disable-client-side-phishing-detection",
    "--disable-backgrounding-occluded-windows",
    "--disable-layer-tree-host-memory-pressure",
    "--autoplay-policy=user-gesture-required",
    "--disable-offer-store-unmasked-wallet-cards",
    "--disable-blink-features=AutomationControlled",
    "--disable-component-extensions-with-background-pages",
    "--enable-features=NetworkService,NetworkServiceInProcess,TrustTokens,TrustTokensAlwaysAllowIssuance",
    "--blink-settings=primaryHoverType=2,availableHoverTypes=2,primaryPointerType=4,availablePointerTypes=4",
    "--disable-features=AudioServiceOutOfProcess,TranslateUI,BlinkGenPropertyTrees",
];

// CPython 3.12 set order with PYTHONHASHSEED=0, retained as an independent
// frozen transcript assertion for the default stealth launch.
#[cfg(test)]
const STEALTH_DEFAULT_ARGS: &[&str] = &[
    "--homepage=about:blank",
    "--disable-threaded-animation",
    "--disable-offer-store-unmasked-wallet-cards",
    "--disable-component-extensions-with-background-pages",
    "--enable-async-dns",
    "--disable-new-content-rendering-timeout",
    "--disable-background-networking",
    "--disable-sync",
    "--no-service-autorun",
    "--disable-checker-imaging",
    "--enable-tcp-fast-open",
    "--enable-surface-synchronization",
    "--disable-offer-upload-credit-cards",
    "--run-all-compositor-stages-before-draw",
    "--autoplay-policy=user-gesture-required",
    "--enable-features=NetworkService,NetworkServiceInProcess,TrustTokens,TrustTokensAlwaysAllowIssuance",
    "--blink-settings=primaryHoverType=2,availableHoverTypes=2,primaryPointerType=4,availablePointerTypes=4",
    "--accept-lang=en-US",
    "--use-mock-keychain",
    "--disable-ipc-flooding-protection",
    "--no-first-run",
    "--disable-threaded-scrolling",
    "--safebrowsing-disable-auto-update",
    "--disable-infobars",
    "--enable-web-bluetooth",
    "--disable-domain-reliability",
    "--disable-client-side-phishing-detection",
    "--hide-scrollbars",
    "--enable-simple-cache-backend",
    "--disable-features=AudioServiceOutOfProcess,TranslateUI,BlinkGenPropertyTrees",
    "--prerender-from-omnibox=disabled",
    "--test-type",
    "--disable-dev-shm-usage",
    "--metrics-recording-only",
    "--disable-session-crashed-bubble",
    "--mute-audio",
    "--aggressive-cache-discard",
    "--lang=en-US",
    "--disable-voice-input",
    "--disable-blink-features=AutomationControlled",
    "--disable-breakpad",
    "--disable-cookie-encryption",
    "--disable-background-timer-throttling",
    "--disable-gesture-typing",
    "--window-position=0,0",
    "--disable-hang-monitor",
    "--disable-crash-reporter",
    "--disable-logging",
    "--disable-image-animation-resync",
    "--force-color-profile=srgb",
    "--disable-layer-tree-host-memory-pressure",
    "--ignore-gpu-blocklist",
    "--disable-renderer-backgrounding",
    "--disable-print-preview",
    "--start-maximized",
    "--disable-wake-on-wifi",
    "--disable-partial-raster",
    "--disable-prompt-on-repost",
    "--no-default-browser-check",
    "--password-store=basic",
    "--no-pings",
    "--disable-backgrounding-occluded-windows",
    "--disable-translate",
    "--disable-cloud-import",
    "--disable-search-engine-choice-screen",
    "--font-render-hinting=none",
];

const BLOCKED_RESOURCE_TYPES: &[&str] = &[
    "Font",
    "Image",
    "Media",
    "TextTrack",
    "WebSocket",
    "Stylesheet",
];

static AD_DOMAINS: OnceLock<HashSet<&'static str>> = OnceLock::new();
type ProxyConfig = (Option<String>, Option<(String, String)>);

#[derive(Clone, Debug)]
pub struct RawBrowserOptions {
    pub headless: bool,
    pub network_idle: bool,
    pub load_dom: bool,
    pub timeout: Duration,
    pub wait: Duration,
    pub wait_selector: Option<String>,
    pub wait_selector_state: String,
    pub disable_resources: bool,
    pub block_ads: bool,
    pub blocked_domains: Vec<String>,
    pub proxy: Option<String>,
    pub proxy_auth: Option<(String, String)>,
    pub useragent: Option<String>,
    pub cookies: Vec<Value>,
    pub extra_headers: Map<String, Value>,
    pub google_search: bool,
    pub capture_xhr: Option<String>,
    pub locale: Option<String>,
    pub timezone_id: Option<String>,
    pub dns_over_https: bool,
    pub extra_flags: Vec<String>,
    pub retries: usize,
    pub retry_delay: Duration,
    pub cdp_url: Option<String>,
    pub real_chrome: bool,
    pub block_webrtc: bool,
    pub hide_canvas: bool,
    pub allow_webgl: bool,
    pub solve_cloudflare: bool,
}

/// Upper clamp for `retry_delay` (seconds). `Duration::from_secs_f64` PANICS
/// on values it cannot represent, and the schema declares a bare number, so
/// an unclamped `{"retry_delay": 1e300}` would panic inside the handler and
/// hang the caller (the HTTP tier clamps for exactly this reason).
const MAX_RETRY_DELAY_SECS: f64 = 60.0;

impl RawBrowserOptions {
    pub fn from_payload(payload: &Value) -> Result<Self, String> {
        let mut timeout = float_field(payload, "timeout", 30_000.0)?;
        let wait = float_field(payload, "wait", 0.0)?;
        let retry_delay = float_field(payload, "retry_delay", 1.0)?.min(MAX_RETRY_DELAY_SECS);
        let retries = bounded_int_field(payload, "retries", 3, 1, 10)?;
        let _max_pages = bounded_int_field(payload, "max_pages", 1, 1, 50)?;
        let solve_cloudflare = bool_field(payload, "solve_cloudflare", false)?;
        if solve_cloudflare {
            timeout = timeout.max(60_000.0);
        }
        let wait_selector_state = string_field(payload, "wait_selector_state", "attached")?;
        if !matches!(
            wait_selector_state.as_str(),
            "attached" | "detached" | "visible" | "hidden"
        ) {
            return Err(format!(
                "Invalid argument type: Invalid enum value '{wait_selector_state}' - at `$.wait_selector_state`"
            ));
        }
        let (proxy, proxy_auth) = proxy_field(payload)?;
        Ok(Self {
            headless: bool_field(payload, "headless", true)?,
            network_idle: bool_field(payload, "network_idle", false)?,
            load_dom: bool_field(payload, "load_dom", true)?,
            timeout: Duration::from_millis(timeout as u64),
            wait: Duration::from_millis(wait as u64),
            wait_selector: optional_string_field(payload, "wait_selector")?,
            wait_selector_state,
            disable_resources: bool_field(payload, "disable_resources", false)?,
            block_ads: bool_field(payload, "block_ads", false)?,
            blocked_domains: string_list_field(payload, "blocked_domains")?,
            proxy,
            proxy_auth,
            useragent: optional_string_field(payload, "useragent")?
                .filter(|value| !value.is_empty()),
            cookies: cookies(payload.get("cookies"))?,
            extra_headers: string_map_field(payload, "extra_headers")?,
            google_search: bool_field(payload, "google_search", true)?,
            capture_xhr: optional_string_field(payload, "capture_xhr")?
                .filter(|value| !value.is_empty()),
            locale: optional_string_field(payload, "locale")?.filter(|value| !value.is_empty()),
            timezone_id: optional_string_field(payload, "timezone_id")?
                .filter(|value| !value.is_empty()),
            dns_over_https: bool_field(payload, "dns_over_https", false)?,
            extra_flags: string_list_field(payload, "extra_flags")?,
            retries: retries as usize,
            retry_delay: Duration::from_secs_f64(retry_delay),
            cdp_url: cdp_url(payload)?,
            real_chrome: bool_field(payload, "real_chrome", false)?,
            block_webrtc: bool_field(payload, "block_webrtc", false)?,
            hide_canvas: bool_field(payload, "hide_canvas", false)?,
            allow_webgl: bool_field(payload, "allow_webgl", true)?,
            solve_cloudflare,
        })
    }

    /// Clamp `timeout` and `wait` to the configured ceiling. The deleted
    /// python-shaped tier clamped to `max_timeout_ms` and the interactive
    /// tier still does; without this a caller-supplied `wait` holds a
    /// Chromium page (or wedges a session actor) for an arbitrary time.
    pub fn clamp_durations(&mut self, max_timeout_ms: u64) {
        let cap = Duration::from_millis(max_timeout_ms.max(1));
        self.timeout = self.timeout.min(cap);
        self.wait = self.wait.min(cap);
    }

    pub fn validate_policy(&self, mode: SecurityMode) -> Result<(), String> {
        if mode == SecurityMode::Compat {
            return Ok(());
        }
        // Keep in sync with `sessions::uses_compat_only_options`, or an
        // option refused here slips through session-open and only errors at
        // first fetch (and vice versa).
        let mut refused = Vec::new();
        if self.proxy.is_some() {
            refused.push("proxy");
        }
        if self.cdp_url.is_some() {
            refused.push("cdp_url");
        }
        if self.dns_over_https {
            refused.push("dns_over_https");
        }
        if !self.extra_flags.is_empty() {
            refused.push("extra_flags");
        }
        // Refused rather than silently discarded: safe mode never honors it,
        // and an ignored option is a lie to the caller.
        if self.real_chrome {
            refused.push("real_chrome");
        }
        if !refused.is_empty() {
            return Err(format!(
                "{} require browser.scrapling.security_mode=compat",
                refused.join(", ")
            ));
        }
        Ok(())
    }
}

fn present<'a>(payload: &'a Value, key: &str) -> Option<&'a Value> {
    payload.get(key).filter(|value| !value.is_null())
}

fn python_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(number) if number.is_i64() || number.is_u64() => "int",
        Value::Number(_) => "float",
        Value::String(_) => "str",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn type_error(expected: &str, value: &Value, path: &str) -> String {
    format!(
        "Invalid argument type: Expected `{expected}`, got `{}` - at `${path}`",
        python_type(value)
    )
}

fn bool_field(payload: &Value, key: &str, default: bool) -> Result<bool, String> {
    let Some(value) = present(payload, key) else {
        return Ok(default);
    };
    if let Some(value) = value.as_bool() {
        return Ok(value);
    }
    if value
        .as_f64()
        .is_some_and(|value| value == if default { 1.0 } else { 0.0 })
    {
        return Ok(default);
    }
    Err(type_error("bool", value, &format!(".{key}")))
}

fn float_field(payload: &Value, key: &str, default: f64) -> Result<f64, String> {
    let Some(value) = present(payload, key) else {
        return Ok(default);
    };
    if let Some(number) = value.as_f64() {
        if number < 0.0 {
            return Err(format!(
                "Invalid argument type: Expected `float` >= 0.0 - at `$.{key}`"
            ));
        }
        return Ok(number);
    }
    if value
        .as_bool()
        .is_some_and(|value| (if value { 1.0 } else { 0.0 }) == default)
    {
        return Ok(default);
    }
    Err(type_error("float", value, &format!(".{key}")))
}

fn bounded_int_field(
    payload: &Value,
    key: &str,
    default: i64,
    minimum: i64,
    maximum: i64,
) -> Result<i64, String> {
    let Some(value) = present(payload, key) else {
        return Ok(default);
    };
    let number = if let Some(number) = value.as_i64() {
        number
    } else if let Some(number) = value.as_u64() {
        if number > maximum as u64 {
            return Err(format!(
                "Invalid argument type: Expected `int` <= {maximum} - at `$.{key}`"
            ));
        }
        number as i64
    } else if value
        .as_f64()
        .is_some_and(|number| number == default as f64)
        || value
            .as_bool()
            .is_some_and(|number| i64::from(number) == default)
    {
        default
    } else {
        return Err(type_error("int", value, &format!(".{key}")));
    };
    if number < minimum {
        return Err(format!(
            "Invalid argument type: Expected `int` >= {minimum} - at `$.{key}`"
        ));
    }
    if number > maximum {
        return Err(format!(
            "Invalid argument type: Expected `int` <= {maximum} - at `$.{key}`"
        ));
    }
    Ok(number)
}

fn string_field(payload: &Value, key: &str, default: &str) -> Result<String, String> {
    let Some(value) = present(payload, key) else {
        return Ok(default.to_string());
    };
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| type_error("str", value, &format!(".{key}")))
}

fn optional_string_field(payload: &Value, key: &str) -> Result<Option<String>, String> {
    let Some(value) = present(payload, key) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(|value| Some(value.to_string()))
        .ok_or_else(|| type_error("str | null", value, &format!(".{key}")))
}

fn string_list_field(payload: &Value, key: &str) -> Result<Vec<String>, String> {
    let Some(value) = present(payload, key) else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| type_error("array | null", value, &format!(".{key}")))?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| type_error("str", value, &format!(".{key}[{index}]")))
        })
        .collect()
}

fn string_map_field(payload: &Value, key: &str) -> Result<Map<String, Value>, String> {
    let Some(value) = present(payload, key) else {
        return Ok(Map::new());
    };
    let values = value
        .as_object()
        .ok_or_else(|| type_error("object | null", value, &format!(".{key}")))?;
    for value in values.values() {
        if !value.is_string() {
            return Err(type_error("str", value, &format!(".{key}[...]")));
        }
    }
    Ok(values.clone())
}

fn cdp_url(payload: &Value) -> Result<Option<String>, String> {
    let Some(url) = optional_string_field(payload, "cdp_url")? else {
        return Ok(None);
    };
    if url.is_empty() {
        return Ok(None);
    }
    let Some(authority) = url
        .strip_prefix("ws://")
        .or_else(|| url.strip_prefix("wss://"))
    else {
        return Err(
            "Invalid argument type: CDP URL must use 'ws://' or 'wss://' scheme".to_string(),
        );
    };
    if authority
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .is_empty()
    {
        return Err("Invalid argument type: Invalid hostname for the CDP URL".to_string());
    }
    Ok(Some(url))
}

fn proxy_field(payload: &Value) -> Result<ProxyConfig, String> {
    let Some(value) = present(payload, "proxy") else {
        return Ok((None, None));
    };
    if let Some(proxy) = value.as_str() {
        if proxy.is_empty() {
            return Ok((None, None));
        }
        let parsed = url::Url::parse(proxy)
            .map_err(|_| "Invalid argument type: Invalid proxy string!".to_string())?;
        if !matches!(parsed.scheme(), "http" | "https" | "socks4" | "socks5")
            || parsed.host_str().is_none()
        {
            return Err("Invalid argument type: Invalid proxy string!".to_string());
        }
        let mut server = format!(
            "{}://{}",
            parsed.scheme(),
            parsed.host_str().unwrap_or_default()
        );
        if let Some(port) = parsed.port() {
            server.push_str(&format!(":{port}"));
        }
        let username = parsed.username().to_string();
        let password = parsed.password().unwrap_or_default().to_string();
        let auth = (!username.is_empty() || !password.is_empty()).then_some((username, password));
        return Ok((Some(server), auth));
    }
    if let Some(values) = value.as_object() {
        for value in values.values() {
            if !value.is_string() {
                return Err(type_error("str", value, ".proxy[...]"));
            }
        }
        let server = values.get("server").and_then(Value::as_str).ok_or_else(|| {
            "Invalid argument type: Invalid proxy dictionary: Object missing required field `server`"
                .to_string()
        })?;
        let username = values
            .get("username")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let password = values
            .get("password")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let auth = (!username.is_empty() || !password.is_empty()).then_some((username, password));
        return Ok((Some(server.to_string()), auth));
    }
    if value.as_array().is_some_and(Vec::is_empty) {
        return Ok((None, None));
    }
    Err(type_error("str | object | array | null", value, ".proxy"))
}

pub struct RawBrowser {
    client: CdpClient,
    context_id: Option<String>,
    persistent: bool,
    remote: bool,
    closed: AtomicBool,
    child_router: tokio::task::JoinHandle<()>,
    _profile: Option<TempProfile>,
    _gate: Option<EgressGate>,
}

impl RawBrowser {
    pub async fn start(
        config: &WorkerConfig,
        options: &RawBrowserOptions,
        stealth: bool,
        persistent: bool,
    ) -> Result<Self, String> {
        crate::scrapling::browserforge::initialize_browser_defaults()?;
        options.validate_policy(config.scrapling.security_mode)?;
        let gate = if config.scrapling.security_mode == SecurityMode::Safe {
            Some(
                EgressGate::start(SsrfPolicy {
                    allow_loopback: config.scrapling.allow_loopback,
                })
                .await?,
            )
        } else {
            None
        };

        let (client, profile, remote) = if let Some(url) = &options.cdp_url {
            (
                CdpClient::connect_websocket_url(url)
                    .await
                    .map_err(|e| e.to_string())?,
                None,
                true,
            )
        } else {
            let executable = chromium_executable(config, options.real_chrome)?;
            // The exact-version pin exists for the compat fingerprint story
            // (its error message says so); safe mode must accept whatever
            // Chromium the system has or the default config is dead on every
            // stock Linux install.
            if config.scrapling.security_mode == SecurityMode::Compat
                && cfg!(all(
                    target_os = "linux",
                    any(target_arch = "x86_64", target_arch = "aarch64")
                ))
            {
                certify_chromium(&executable)?;
            }
            let profile = TempProfile::new()?;
            let mut command =
                launch_command(&executable, &profile.path, options, stealth, gate.as_ref());
            let client = CdpClient::launch_pipe(&mut command).map_err(|e| e.to_string())?;
            (client, Some(profile), false)
        };

        initialize_root(&client, persistent && !remote).await?;
        let child_router = tokio::spawn(route_child_targets(client.clone(), stealth));
        let context_id = if remote {
            Some(create_browser_context(&client, options.proxy.as_deref()).await?)
        } else {
            None
        };
        Ok(Self {
            client,
            context_id,
            persistent,
            remote,
            closed: AtomicBool::new(false),
            child_router,
            _profile: profile,
            _gate: gate,
        })
    }

    pub async fn fetch(
        &self,
        url: &str,
        options: &RawBrowserOptions,
        stealth: bool,
    ) -> Result<PageData, String> {
        self.fetch_inner(url, options, stealth, false).await
    }

    pub async fn fetch_session(
        &self,
        url: &str,
        options: &RawBrowserOptions,
        stealth: bool,
        proxy_override: bool,
    ) -> Result<PageData, String> {
        self.fetch_inner(url, options, stealth, proxy_override)
            .await
    }

    async fn fetch_inner(
        &self,
        url: &str,
        options: &RawBrowserOptions,
        stealth: bool,
        proxy_override: bool,
    ) -> Result<PageData, String> {
        let mut last = None;
        for attempt in 0..options.retries {
            let temporary_context = if proxy_override && options.proxy.is_some() {
                if !self.remote {
                    return Err("Browser not initialized for proxy rotation mode".to_string());
                }
                Some(create_browser_context(&self.client, options.proxy.as_deref()).await?)
            } else {
                None
            };
            let context = temporary_context.as_deref().or(self.context_id.as_deref());
            let result = self.fetch_once(url, options, stealth, context).await;
            if let Some(context) = temporary_context {
                dispose_browser_context(&self.client, &context).await;
            }
            match result {
                Ok(page) => return Ok(page),
                Err(error) => last = Some(error),
            }
            if attempt + 1 < options.retries {
                tokio::time::sleep(options.retry_delay).await;
            }
        }
        Err(last.unwrap_or_else(|| "Request failed".to_string()))
    }

    async fn fetch_once(
        &self,
        url: &str,
        options: &RawBrowserOptions,
        stealth: bool,
        context_id: Option<&str>,
    ) -> Result<PageData, String> {
        let mut page = RawPage::open(&self.client, context_id, options, stealth).await?;
        let result = page.navigate(url, options).await;
        page.close().await;
        result
    }

    pub async fn screenshot(
        &self,
        url: &str,
        options: &RawBrowserOptions,
        stealth: bool,
        full_page: bool,
        format: &str,
    ) -> Result<(Vec<Value>, String, String), String> {
        let mut page =
            RawPage::open(&self.client, self.context_id.as_deref(), options, stealth).await?;
        // Close the page on every path — an early `?` here would leak the tab.
        let shot = async {
            let final_url = page.navigate(url, options).await?.url;
            let (image, mime) = page.screenshot(full_page, format).await?;
            Ok::<_, String>((final_url, image, mime))
        }
        .await;
        page.close().await;
        let (final_url, image, mime) = shot?;
        let (tiles, width, height, truncated) = tile_screenshot(&image, format)?;
        let kb =
            python_round(tiles.iter().map(Vec::len).sum::<usize>() as f64 / 1024.0).max(1) as usize;
        let mut content = tiles
            .into_iter()
            .map(|tile| json!({"type": "image", "mime": mime, "data": STANDARD.encode(tile)}))
            .collect::<Vec<_>>();
        let mut caption = format!(
            "screenshot of {final_url} — {width}x{height}px, {} tile(s), {kb} KB",
            content.len()
        );
        if truncated {
            caption.push_str(" (truncated: page taller than 6 tiles)");
        }
        content.push(json!({"type": "text", "text": caption}));
        Ok((content, mime, final_url))
    }

    pub async fn shutdown(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.child_router.abort();
        if let Some(context) = &self.context_id {
            dispose_browser_context(&self.client, context).await;
        }
        if self.persistent && !self.remote {
            if let Ok(command) = self.client.send("Browser.close", json!({})) {
                let _ = command.await;
            }
        }
        let _ = self.client.close();
    }
}

impl Drop for RawBrowser {
    fn drop(&mut self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.child_router.abort();
        if let Some(context_id) = self.context_id.take() {
            if let Ok(command) = self.client.send(
                "Target.disposeBrowserContext",
                json!({"browserContextId": context_id}),
            ) {
                drop(command);
            }
        }
        if self.persistent && !self.remote {
            if let Ok(command) = self.client.send("Browser.close", json!({})) {
                drop(command);
            }
        }
        let _ = self.client.close();
    }
}

async fn create_browser_context(client: &CdpClient, proxy: Option<&str>) -> Result<String, String> {
    let mut params = Map::new();
    params.insert("disposeOnDetach".to_string(), json!(true));
    if let Some(proxy) = proxy {
        params.insert("proxyServer".to_string(), json!(proxy));
        params.insert("proxyBypassList".to_string(), json!("<-loopback>"));
    }
    client
        .send("Target.createBrowserContext", Value::Object(params))
        .map_err(|error| error.to_string())?
        .await
        .map_err(|error| error.to_string())?
        .get("browserContextId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "Target.createBrowserContext returned no browserContextId".to_string())
}

async fn dispose_browser_context(client: &CdpClient, context: &str) {
    if let Ok(command) = client.send(
        "Target.disposeBrowserContext",
        json!({"browserContextId": context}),
    ) {
        let _ = command.await;
    }
}

async fn route_child_targets(client: CdpClient, stealth: bool) {
    // Auto-attach happens on the PAGE session, so child targets announce
    // themselves tagged with the page's sessionId — a root-scoped receiver
    // never sees them and the child stays frozen on waitForDebuggerOnStart.
    let mut events = client.subscribe_any();
    while let Ok(event) = events.recv().await {
        if event.method != "Target.attachedToTarget" {
            continue;
        }
        let Some(session_id) = event.params.get("sessionId").and_then(Value::as_str) else {
            continue;
        };
        let target_type = event
            .params
            .pointer("/targetInfo/type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if target_type == "page" {
            continue;
        }
        let session = client.session(session_id);
        let runtime = if stealth {
            session.send(
                "Runtime.evaluate",
                json!({
                    "expression": "globalThis",
                    "serializationOptions": {"serialization": "idOnly"},
                    "returnByValue": false
                }),
            )
        } else {
            session.send("Runtime.enable", json!({}))
        };
        if let Ok(runtime) = runtime {
            let _ = runtime.await;
        }
        if let Ok(resume) = session.send("Runtime.runIfWaitingForDebugger", json!({})) {
            let _ = resume.await;
        }
    }
}

struct RawPage {
    client: CdpClient,
    session: CdpSession,
    target_id: String,
    events: EventReceiver,
}

impl RawPage {
    async fn open(
        client: &CdpClient,
        context_id: Option<&str>,
        options: &RawBrowserOptions,
        stealth: bool,
    ) -> Result<Self, String> {
        let mut root_events = client.subscribe();
        let mut params = json!({"url": "about:blank", "newWindow": false});
        if let Some(context) = context_id {
            params["browserContextId"] = json!(context);
        }
        let target_id = client
            .send("Target.createTarget", params)
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or("Target.createTarget returned no targetId")?
            .to_string();
        // From here on the created target must not leak: every error path
        // closes it, or failed opens pile up tabs in the pooled browser.
        let attach = async {
            let deadline = Instant::now() + options.timeout;
            let session_id = loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(format!("timed out attaching to browser target {target_id}"));
                }
                let event = tokio::time::timeout(remaining, root_events.recv())
                    .await
                    .map_err(|_| format!("timed out attaching to browser target {target_id}"))?
                    .map_err(|e| e.to_string())?;
                if event.method == "Target.attachedToTarget"
                    && event
                        .params
                        .pointer("/targetInfo/targetId")
                        .and_then(Value::as_str)
                        == Some(target_id.as_str())
                {
                    break event
                        .params
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .ok_or("Target.attachedToTarget returned no sessionId")?
                        .to_string();
                }
            };
            let session = client.session(session_id);
            let events = session.subscribe();
            initialize_page(&session, options, stealth).await?;
            Ok((session, events))
        }
        .await;
        match attach {
            Ok((session, events)) => Ok(Self {
                client: client.clone(),
                session,
                target_id,
                events,
            }),
            Err(error) => {
                if let Ok(close) = client.send("Target.closeTarget", json!({"targetId": target_id}))
                {
                    let _ = close.await;
                }
                Err(error)
            }
        }
    }

    async fn navigate(
        &mut self,
        url: &str,
        options: &RawBrowserOptions,
    ) -> Result<PageData, String> {
        let navigate = self
            .session
            .send(
                "Page.navigate",
                json!({
                    "url": url,
                    "referrer": referer(options),
                    "transitionType": "typed"
                }),
            )
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
        if let Some(error) = navigate.get("errorText").and_then(Value::as_str) {
            return Err(error.to_string());
        }
        let main_frame = navigate
            .get("frameId")
            .and_then(Value::as_str)
            .map(str::to_string);

        let deadline = Instant::now() + options.timeout;
        let mut loaded = false;
        let mut dom_loaded = !options.load_dom;
        let mut inflight = HashSet::new();
        let mut quiet_since = Instant::now();
        let mut responses = HashMap::<String, ResponseInfo>::new();
        let mut document_request = None;
        let mut xhr_ids = Vec::new();
        loop {
            if loaded
                && dom_loaded
                && (!options.network_idle
                    || (inflight.is_empty() && quiet_since.elapsed() >= Duration::from_millis(500)))
            {
                break;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let event = match tokio::time::timeout(
                remaining.min(Duration::from_millis(100)),
                self.events.recv(),
            )
            .await
            {
                Ok(Ok(event)) => event,
                Ok(Err(error)) => return Err(error.to_string()),
                Err(_) => continue,
            };
            self.track_event(
                event,
                options,
                main_frame.as_deref(),
                &mut loaded,
                &mut dom_loaded,
                &mut inflight,
                &mut quiet_since,
                &mut responses,
                &mut document_request,
                &mut xhr_ids,
            )
            .await?;
        }

        if options.solve_cloudflare {
            self.solve_cloudflare(
                options,
                main_frame.as_deref(),
                &mut loaded,
                &mut dom_loaded,
                &mut inflight,
                &mut quiet_since,
                &mut responses,
                &mut document_request,
                &mut xhr_ids,
            )
            .await?;
        }
        if let Some(selector) = &options.wait_selector {
            let found = self
                .wait_selector(selector, &options.wait_selector_state, options.timeout)
                .await;
            if found && options.network_idle {
                self.wait_for_network_idle(
                    options.timeout,
                    options,
                    main_frame.as_deref(),
                    &mut loaded,
                    &mut dom_loaded,
                    &mut inflight,
                    &mut quiet_since,
                    &mut responses,
                    &mut document_request,
                    &mut xhr_ids,
                )
                .await?;
            }
        }
        if !options.wait.is_zero() {
            tokio::time::sleep(options.wait).await;
        }
        let final_url = self
            .evaluate("location.href", true)
            .await?
            .as_str()
            .unwrap_or(url)
            .to_string();
        let response = document_request
            .as_ref()
            .and_then(|request| responses.get(request))
            .cloned();
        let content_type = response
            .as_ref()
            .and_then(|value| value.headers.get("content-type"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let html = if content_type.contains("html") || document_request.is_none() {
            self.page_html().await?
        } else {
            let body = self
                .session
                .send(
                    "Network.getResponseBody",
                    json!({"requestId": document_request.as_deref().unwrap_or_default()}),
                )
                .map_err(|e| e.to_string())?
                .await
                .map_err(|e| e.to_string())?;
            let body = decode_network_body(&body, charset(content_type).as_deref())?;
            let document = crate::scrapling::dom::parse(&body);
            crate::scrapling::dom::outer_html(document.root())
        };
        let mut captured_xhr = Vec::new();
        for id in xhr_ids {
            if let Some(info) = responses.get(&id) {
                captured_xhr.push(info.as_value());
            }
        }
        Ok(PageData {
            status: response.as_ref().and_then(|value| value.status),
            url: final_url,
            headers: response
                .as_ref()
                .map(|value| value.headers.clone())
                .unwrap_or_default(),
            // The frozen wrapper's tuple-of-cookie-dicts serialization quirk is `{}`.
            cookies: Map::new(),
            encoding: response
                .as_ref()
                .and_then(|value| value.headers.get("content-type"))
                .and_then(Value::as_str)
                .and_then(charset)
                .or_else(|| Some("utf-8".to_string())),
            html,
            captured_xhr,
        })
    }

    async fn evaluate(&self, expression: &str, return_by_value: bool) -> Result<Value, String> {
        let result = self
            .session
            .send(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": return_by_value,
                    "awaitPromise": true,
                    "userGesture": true
                }),
            )
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
        if let Some(exception) = result.get("exceptionDetails") {
            return Err(format!("JavaScript evaluation failed: {exception}"));
        }
        Ok(result
            .get("result")
            .and_then(|value| value.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    async fn intercept(&self, event: &CdpEvent, options: &RawBrowserOptions) -> Result<(), String> {
        let request_id = event
            .params
            .get("requestId")
            .and_then(Value::as_str)
            .ok_or("Fetch.requestPaused returned no requestId")?;
        let resource = event
            .params
            .get("resourceType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let host = event
            .params
            .pointer("/request/url")
            .and_then(Value::as_str)
            .and_then(|url| url::Url::parse(url).ok())
            .and_then(|url| url.host_str().map(str::to_string));
        let blocked = (options.disable_resources && BLOCKED_RESOURCE_TYPES.contains(&resource))
            || host.as_deref().is_some_and(|host| {
                options
                    .blocked_domains
                    .iter()
                    .any(|domain| domain_matches(host, domain))
                    || (options.block_ads
                        && ad_domains()
                            .iter()
                            .any(|domain| domain_matches(host, domain)))
            });
        let (method, params) = if blocked {
            (
                "Fetch.failRequest",
                json!({"requestId": request_id, "errorReason": "BlockedByClient"}),
            )
        } else {
            ("Fetch.continueRequest", json!({"requestId": request_id}))
        };
        self.session
            .send(method, params)
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn authenticate(
        &self,
        event: &CdpEvent,
        options: &RawBrowserOptions,
    ) -> Result<(), String> {
        let request_id = event
            .params
            .get("requestId")
            .and_then(Value::as_str)
            .ok_or("Fetch.authRequired returned no requestId")?;
        let is_proxy = event
            .params
            .pointer("/authChallenge/source")
            .and_then(Value::as_str)
            == Some("Proxy");
        let response = if let (true, Some((username, password))) = (is_proxy, &options.proxy_auth) {
            json!({
                "response": "ProvideCredentials",
                "username": username,
                "password": password
            })
        } else {
            json!({"response": "Default"})
        };
        self.session
            .send(
                "Fetch.continueWithAuth",
                json!({"requestId": request_id, "authChallengeResponse": response}),
            )
            .map_err(|error| error.to_string())?
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    async fn page_html(&self) -> Result<String, String> {
        Ok(self
            .evaluate("document.documentElement.outerHTML", true)
            .await?
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    async fn wait_selector(&self, selector: &str, state: &str, timeout: Duration) -> bool {
        let selector = serde_json::to_string(selector).unwrap_or_else(|_| "null".to_string());
        let predicate = match state {
            "detached" => format!("!document.querySelector({selector})"),
            "visible" => format!("(()=>{{const e=document.querySelector({selector});return !!e&&(e.offsetParent!==null||e.getClientRects().length>0)}})()"),
            "hidden" => format!("(()=>{{const e=document.querySelector({selector});return !e||(e.offsetParent===null&&e.getClientRects().length===0)}})()"),
            _ => format!("!!document.querySelector({selector})"),
        };
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.evaluate(&predicate, true).await == Ok(Value::Bool(true)) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    async fn track_event(
        &self,
        event: CdpEvent,
        options: &RawBrowserOptions,
        main_frame: Option<&str>,
        loaded: &mut bool,
        dom_loaded: &mut bool,
        inflight: &mut HashSet<String>,
        quiet_since: &mut Instant,
        responses: &mut HashMap<String, ResponseInfo>,
        document_request: &mut Option<String>,
        xhr_ids: &mut Vec<String>,
    ) -> Result<(), String> {
        match event.method.as_str() {
            "Fetch.requestPaused" => {
                self.intercept(&event, options).await?;
                return Ok(());
            }
            "Fetch.authRequired" => {
                self.authenticate(&event, options).await?;
                return Ok(());
            }
            _ => {}
        }
        handle_event(
            event,
            main_frame,
            options.capture_xhr.as_deref(),
            loaded,
            dom_loaded,
            inflight,
            quiet_since,
            responses,
            document_request,
            xhr_ids,
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn wait_for_network_idle(
        &mut self,
        timeout: Duration,
        options: &RawBrowserOptions,
        main_frame: Option<&str>,
        loaded: &mut bool,
        dom_loaded: &mut bool,
        inflight: &mut HashSet<String>,
        quiet_since: &mut Instant,
        responses: &mut HashMap<String, ResponseInfo>,
        document_request: &mut Option<String>,
        xhr_ids: &mut Vec<String>,
    ) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            let idle = inflight.is_empty() && quiet_since.elapsed() >= Duration::from_millis(500);
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(());
            }
            let wait = if idle {
                Duration::from_millis(1)
            } else {
                remaining.min(Duration::from_millis(100))
            };
            match tokio::time::timeout(wait, self.events.recv()).await {
                Ok(Ok(event)) => {
                    self.track_event(
                        event,
                        options,
                        main_frame,
                        loaded,
                        dom_loaded,
                        inflight,
                        quiet_since,
                        responses,
                        document_request,
                        xhr_ids,
                    )
                    .await?;
                }
                Ok(Err(error)) => return Err(error.to_string()),
                Err(_) if idle => return Ok(()),
                Err(_) => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn wait_for_load_state(
        &mut self,
        timeout: Duration,
        options: &RawBrowserOptions,
        main_frame: Option<&str>,
        loaded: &mut bool,
        dom_loaded: &mut bool,
        inflight: &mut HashSet<String>,
        quiet_since: &mut Instant,
        responses: &mut HashMap<String, ResponseInfo>,
        document_request: &mut Option<String>,
        xhr_ids: &mut Vec<String>,
    ) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            match tokio::time::timeout(Duration::from_millis(1), self.events.recv()).await {
                Ok(Ok(event)) => {
                    self.track_event(
                        event,
                        options,
                        main_frame,
                        loaded,
                        dom_loaded,
                        inflight,
                        quiet_since,
                        responses,
                        document_request,
                        xhr_ids,
                    )
                    .await?;
                    continue;
                }
                Ok(Err(error)) => return Err(error.to_string()),
                Err(_) => {}
            }
            if self
                .evaluate("document.readyState === 'complete'", true)
                .await?
                == Value::Bool(true)
            {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("Timeout while waiting for page load state".to_string());
            }
            match tokio::time::timeout(
                remaining.min(Duration::from_millis(100)),
                self.events.recv(),
            )
            .await
            {
                Ok(Ok(event)) => {
                    self.track_event(
                        event,
                        options,
                        main_frame,
                        loaded,
                        dom_loaded,
                        inflight,
                        quiet_since,
                        responses,
                        document_request,
                        xhr_ids,
                    )
                    .await?;
                }
                Ok(Err(error)) => return Err(error.to_string()),
                Err(_) => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn solve_cloudflare(
        &mut self,
        options: &RawBrowserOptions,
        main_frame: Option<&str>,
        loaded: &mut bool,
        dom_loaded: &mut bool,
        inflight: &mut HashSet<String>,
        quiet_since: &mut Instant,
        responses: &mut HashMap<String, ResponseInfo>,
        document_request: &mut Option<String>,
        xhr_ids: &mut Vec<String>,
    ) -> Result<(), String> {
        // Every poll loop below waits on Cloudflare resolving; a challenge
        // that never clears (a hard block, a broken Turnstile) would otherwise
        // spin forever and brick the session slot. One deadline bounds the
        // whole solve.
        let solve_deadline = Instant::now() + options.timeout;
        let expired = || Instant::now() >= solve_deadline;
        let mut html;
        loop {
            if expired() {
                return Err("timed out solving the Cloudflare challenge".to_string());
            }
            self.wait_for_network_idle(
                Duration::from_secs(5),
                options,
                main_frame,
                loaded,
                dom_loaded,
                inflight,
                quiet_since,
                responses,
                document_request,
                xhr_ids,
            )
            .await?;
            html = self.page_html().await?;
            let Some(challenge) = cloudflare_challenge(&html) else {
                return Ok(());
            };
            if challenge == "non-interactive" {
                while html.contains("<title>Just a moment...</title>") {
                    if expired() {
                        return Err("timed out solving the Cloudflare challenge".to_string());
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    self.wait_for_load_state(
                        options.timeout,
                        options,
                        main_frame,
                        loaded,
                        dom_loaded,
                        inflight,
                        quiet_since,
                        responses,
                        document_request,
                        xhr_ids,
                    )
                    .await?;
                    html = self.page_html().await?;
                }
                return Ok(());
            }

            let selector = if challenge == "embedded" {
                "#cf_turnstile div, #cf-turnstile div, .turnstile>div>div"
            } else {
                ".main-content p+div>div>div"
            };
            if challenge != "embedded" {
                while html.contains("Verifying you are human.") {
                    if expired() {
                        return Err("timed out solving the Cloudflare challenge".to_string());
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    html = self.page_html().await?;
                }
            }

            let (has_iframe, mut outer_box) = self.challenge_iframe_box().await?;
            if has_iframe && challenge != "embedded" {
                while outer_box.is_none() {
                    if expired() {
                        return Err("timed out solving the Cloudflare challenge".to_string());
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    outer_box = self.challenge_iframe_box().await?.1;
                }
            }
            if !has_iframe || outer_box.is_none() {
                html = self.page_html().await?;
                if !html.contains("<title>Just a moment...</title>") {
                    return Ok(());
                }
                outer_box = self.selector_box(selector).await?;
            }
            let (x, y) = outer_box
                .ok_or_else(|| "TypeError: 'NoneType' object is not subscriptable".to_string())?;
            let x = x + f64::from(crate::scrapling::browserforge::randint(26, 28));
            let y = y + f64::from(crate::scrapling::browserforge::randint(25, 27));
            let delay = crate::scrapling::browserforge::randint(100, 200);
            self.session
                .send(
                    "Input.dispatchMouseEvent",
                    json!({"type":"mouseMoved","x":x,"y":y}),
                )
                .map_err(|error| error.to_string())?
                .await
                .map_err(|error| error.to_string())?;
            self.session
                .send(
                    "Input.dispatchMouseEvent",
                    json!({"type":"mousePressed","x":x,"y":y,"button":"left","clickCount":1}),
                )
                .map_err(|error| error.to_string())?
                .await
                .map_err(|error| error.to_string())?;
            tokio::time::sleep(Duration::from_millis(u64::from(delay))).await;
            self.session
                .send(
                    "Input.dispatchMouseEvent",
                    json!({"type":"mouseReleased","x":x,"y":y,"button":"left","clickCount":1}),
                )
                .map_err(|error| error.to_string())?
                .await
                .map_err(|error| error.to_string())?;

            self.wait_for_network_idle(
                options.timeout,
                options,
                main_frame,
                loaded,
                dom_loaded,
                inflight,
                quiet_since,
                responses,
                document_request,
                xhr_ids,
            )
            .await?;
            if challenge != "embedded" {
                for _ in 0..100 {
                    html = self.page_html().await?;
                    if !html.contains("<title>Just a moment...</title>") {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
            self.wait_for_load_state(
                options.timeout,
                options,
                main_frame,
                loaded,
                dom_loaded,
                inflight,
                quiet_since,
                responses,
                document_request,
                xhr_ids,
            )
            .await?;
            html = self.page_html().await?;
            if !html.contains("<title>Just a moment...</title>") {
                return Ok(());
            }
        }
    }

    async fn challenge_iframe_box(&self) -> Result<(bool, Option<(f64, f64)>), String> {
        let tree = self
            .session
            .send("Page.getFrameTree", json!({}))
            .map_err(|error| error.to_string())?
            .await
            .map_err(|error| error.to_string())?;
        let Some(frame_id) = challenge_frame_id(tree.get("frameTree").unwrap_or(&Value::Null))
        else {
            return Ok((false, None));
        };
        let owner = self
            .session
            .send("DOM.getFrameOwner", json!({"frameId": frame_id}))
            .map_err(|error| error.to_string())?
            .await;
        let Ok(owner) = owner else {
            return Ok((true, None));
        };
        let Some(backend_node_id) = owner.get("backendNodeId").and_then(Value::as_u64) else {
            return Ok((true, None));
        };
        let model = self
            .session
            .send("DOM.getBoxModel", json!({"backendNodeId": backend_node_id}))
            .map_err(|error| error.to_string())?
            .await;
        let Ok(model) = model else {
            return Ok((true, None));
        };
        Ok((
            true,
            box_origin(model.pointer("/model/border").unwrap_or(&Value::Null)),
        ))
    }

    async fn selector_box(&self, selector: &str) -> Result<Option<(f64, f64)>, String> {
        let selector = serde_json::to_string(selector).map_err(|error| error.to_string())?;
        let point = self
            .evaluate(
                &format!("(()=>{{const e=[...document.querySelectorAll({selector})].at(-1);if(!e)return null;const r=e.getBoundingClientRect();return{{x:r.x,y:r.y,w:r.width,h:r.height}}}})()"),
                true,
            )
            .await?;
        match (
            point.get("x").and_then(Value::as_f64),
            point.get("y").and_then(Value::as_f64),
            point.get("w").and_then(Value::as_f64),
            point.get("h").and_then(Value::as_f64),
        ) {
            (Some(x), Some(y), Some(width), Some(height)) if width > 0.0 && height > 0.0 => {
                Ok(Some((x, y)))
            }
            _ => Ok(None),
        }
    }

    async fn screenshot(&self, full_page: bool, format: &str) -> Result<(Vec<u8>, String), String> {
        let (format, mime) = match format {
            "png" => ("png", "image/png"),
            "jpeg" | "jpg" => ("jpeg", "image/jpeg"),
            other => return Err(format!("unsupported format: {other} (use png or jpeg)")),
        };
        let mut params = json!({"format": format, "fromSurface": true});
        if full_page {
            let metrics = self
                .session
                .send("Page.getLayoutMetrics", json!({}))
                .map_err(|e| e.to_string())?
                .await
                .map_err(|e| e.to_string())?;
            if let Some(size) = metrics.get("cssContentSize") {
                params["captureBeyondViewport"] = json!(true);
                params["clip"] = json!({
                    "x": size.get("x").and_then(Value::as_f64).unwrap_or(0.0),
                    "y": size.get("y").and_then(Value::as_f64).unwrap_or(0.0),
                    "width": size.get("width").and_then(Value::as_f64).unwrap_or(1280.0),
                    "height": size.get("height").and_then(Value::as_f64).unwrap_or(800.0),
                    "scale": 1
                });
            }
        }
        let shot = self
            .session
            .send("Page.captureScreenshot", params)
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
        let data = shot
            .get("data")
            .and_then(Value::as_str)
            .ok_or("Page.captureScreenshot returned no data")?;
        Ok((
            STANDARD.decode(data).map_err(|e| e.to_string())?,
            mime.to_string(),
        ))
    }

    async fn close(&self) {
        if let Ok(command) = self
            .client
            .send("Target.closeTarget", json!({"targetId": self.target_id}))
        {
            let _ = command.await;
        }
    }
}

#[derive(Clone)]
struct ResponseInfo {
    url: String,
    status: Option<u16>,
    headers: Map<String, Value>,
    request_headers: Map<String, Value>,
}

impl ResponseInfo {
    fn as_value(&self) -> Value {
        json!({
            "status": self.status,
            "url": self.url,
            "headers": self.headers,
            "cookies": {},
            "encoding": self.headers.get("content-type").and_then(Value::as_str).and_then(charset).unwrap_or_else(|| "utf-8".to_string()),
            "content": "",
            "history": [],
            "request_headers": self.request_headers,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_event(
    event: CdpEvent,
    main_frame: Option<&str>,
    capture_xhr: Option<&str>,
    loaded: &mut bool,
    dom_loaded: &mut bool,
    inflight: &mut HashSet<String>,
    quiet_since: &mut Instant,
    responses: &mut HashMap<String, ResponseInfo>,
    document_request: &mut Option<String>,
    xhr_ids: &mut Vec<String>,
) {
    match event.method.as_str() {
        "Page.loadEventFired" => *loaded = true,
        "Page.domContentEventFired" => *dom_loaded = true,
        "Network.requestWillBeSent" => {
            if let Some(request_id) = event.params.get("requestId").and_then(Value::as_str) {
                inflight.insert(request_id.to_string());
            }
            *quiet_since = Instant::now();
            if event.params.get("type").and_then(Value::as_str) == Some("Document")
                && event
                    .params
                    .get("frameId")
                    .and_then(Value::as_str)
                    .is_some_and(|frame| main_frame.is_none_or(|main| main == frame))
            {
                *document_request = event
                    .params
                    .get("requestId")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
        }
        "Network.loadingFinished" | "Network.loadingFailed" => {
            if let Some(request_id) = event.params.get("requestId").and_then(Value::as_str) {
                inflight.remove(request_id);
            }
            *quiet_since = Instant::now();
        }
        "Network.responseReceived" => {
            let Some(id) = event.params.get("requestId").and_then(Value::as_str) else {
                return;
            };
            let response = event.params.get("response").cloned().unwrap_or(Value::Null);
            let url = response
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let resource_type = event.params.get("type").and_then(Value::as_str);
            if matches!(resource_type, Some("XHR" | "Fetch"))
                && capture_xhr.is_some_and(|pattern| {
                    crate::scrapling::text::compile(pattern, false)
                        .is_ok_and(|pattern| pattern.check_match(&url))
                })
            {
                xhr_ids.push(id.to_string());
            }
            responses.insert(
                id.to_string(),
                ResponseInfo {
                    url,
                    status: response
                        .get("status")
                        .and_then(Value::as_f64)
                        .map(|status| status as u16),
                    headers: response
                        .get("headers")
                        .and_then(Value::as_object)
                        .map(playwright_headers)
                        .unwrap_or_default(),
                    request_headers: event
                        .params
                        .pointer("/response/requestHeaders")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default(),
                },
            );
        }
        _ => {}
    }
}

fn playwright_headers(headers: &Map<String, Value>) -> Map<String, Value> {
    headers
        .iter()
        .map(|(name, value)| {
            let name = name.to_ascii_lowercase();
            let value = value
                .as_str()
                .map(|value| {
                    if name == "set-cookie" {
                        value.to_string()
                    } else {
                        value.replace('\n', ", ")
                    }
                })
                .map(Value::String)
                .unwrap_or_else(|| value.clone());
            (name, value)
        })
        .collect()
}

async fn initialize_root(client: &CdpClient, persistent: bool) -> Result<(), String> {
    client
        .send("Browser.getVersion", json!({}))
        .map_err(|e| e.to_string())?
        .await
        .map_err(|e| e.to_string())?;
    client
        .send(
            "Target.setAutoAttach",
            json!({
                "autoAttach": true,
                "waitForDebuggerOnStart": true,
                "flatten": true
            }),
        )
        .map_err(|e| e.to_string())?
        .await
        .map_err(|e| e.to_string())?;
    if persistent {
        client
            .send("Target.getTargetInfo", json!({}))
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn initialize_page(
    page: &CdpSession,
    options: &RawBrowserOptions,
    stealth: bool,
) -> Result<(), String> {
    for (method, params) in [
        ("Page.enable", json!({})),
        ("Page.getFrameTree", json!({})),
        ("Log.enable", json!({})),
        ("Page.setLifecycleEventsEnabled", json!({"enabled": true})),
    ] {
        page.send(method, params)
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
    }
    if !stealth {
        page.send("Runtime.enable", json!({}))
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
    }
    page.send(
        "Page.addScriptToEvaluateOnNewDocument",
        json!({"source": "", "worldName": "__playwright_utility_world__"}),
    )
    .map_err(|e| e.to_string())?
    .await
    .map_err(|e| e.to_string())?;
    let (width, height) = if stealth { (1920, 1080) } else { (1280, 720) };
    page.send(
        "Emulation.setDeviceMetricsOverride",
        json!({
            "width": width,
            "height": height,
            "deviceScaleFactor": 2,
            "mobile": false,
            "screenWidth": width,
            "screenHeight": height
        }),
    )
    .map_err(|e| e.to_string())?
    .await
    .map_err(|e| e.to_string())?;
    page.send(
        "Emulation.setEmulatedMedia",
        json!({"features": [{"name": "prefers-color-scheme", "value": "dark"}]}),
    )
    .map_err(|e| e.to_string())?
    .await
    .map_err(|e| e.to_string())?;
    page.send("Network.enable", json!({}))
        .map_err(|e| e.to_string())?
        .await
        .map_err(|e| e.to_string())?;
    if options.disable_resources
        || options.block_ads
        || !options.blocked_domains.is_empty()
        || options.proxy_auth.is_some()
    {
        page.send(
            "Fetch.enable",
            json!({
                "patterns": [{"urlPattern": "*", "requestStage": "Request"}],
                "handleAuthRequests": options.proxy_auth.is_some()
            }),
        )
        .map_err(|e| e.to_string())?
        .await
        .map_err(|e| e.to_string())?;
    }
    page.send(
        "Target.setAutoAttach",
        json!({"autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true}),
    )
    .map_err(|e| e.to_string())?
    .await
    .map_err(|e| e.to_string())?;
    let user_agent = options
        .useragent
        .as_deref()
        .or(options.headless.then_some(DEFAULT_USER_AGENT));
    if user_agent.is_some() || options.locale.is_some() {
        let mut params = Map::new();
        params.insert(
            "userAgent".to_string(),
            json!(user_agent.unwrap_or(DEFAULT_USER_AGENT)),
        );
        if let Some(locale) = &options.locale {
            params.insert("acceptLanguage".to_string(), json!(locale));
        }
        page.send("Network.setUserAgentOverride", Value::Object(params))
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
    }
    if !options.extra_headers.is_empty() {
        page.send(
            "Network.setExtraHTTPHeaders",
            json!({"headers": options.extra_headers}),
        )
        .map_err(|e| e.to_string())?
        .await
        .map_err(|e| e.to_string())?;
    }
    if let Some(timezone) = &options.timezone_id {
        page.send(
            "Emulation.setTimezoneOverride",
            json!({"timezoneId": timezone}),
        )
        .map_err(|e| e.to_string())?
        .await
        .map_err(|e| e.to_string())?;
    }
    if !options.cookies.is_empty() {
        page.send("Network.setCookies", json!({"cookies": options.cookies}))
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(locale) = &options.locale {
        page.send("Emulation.setLocaleOverride", json!({"locale": locale}))
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
    }
    page.send("Runtime.runIfWaitingForDebugger", json!({}))
        .map_err(|e| e.to_string())?
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn referer(options: &RawBrowserOptions) -> String {
    let has_referer = options
        .extra_headers
        .keys()
        .any(|header| header.eq_ignore_ascii_case("referer"));
    if options.google_search && !has_referer {
        GOOGLE_REFERER.to_string()
    } else {
        String::new()
    }
}

fn cookies(value: Option<&Value>) -> Result<Vec<Value>, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| type_error("array | null", value, ".cookies"))?;
    let mut output = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let object = value
            .as_object()
            .ok_or_else(|| type_error("object", value, &format!(".cookies[{index}]")))?;
        let mut cookie = Map::new();
        for key in ["name", "value", "url", "domain", "path", "partitionKey"] {
            let Some(value) = object.get(key) else {
                continue;
            };
            if !value.is_null() && !value.is_string() {
                return Err(type_error(
                    if matches!(key, "name" | "value") {
                        "str"
                    } else {
                        "str | null"
                    },
                    value,
                    &format!(".cookies[{index}].{key}"),
                ));
            }
            cookie.insert(key.to_string(), value.clone());
        }
        if let Some(value) = object.get("expires") {
            if !value.is_null() && value.as_f64().is_none() {
                return Err(type_error(
                    "float | null",
                    value,
                    &format!(".cookies[{index}].expires"),
                ));
            }
            cookie.insert("expires".to_string(), value.clone());
        }
        for key in ["httpOnly", "secure"] {
            let Some(value) = object.get(key) else {
                continue;
            };
            if !value.is_null() && !value.is_boolean() {
                return Err(type_error(
                    "bool | null",
                    value,
                    &format!(".cookies[{index}].{key}"),
                ));
            }
            cookie.insert(key.to_string(), value.clone());
        }
        if let Some(value) = object.get("sameSite") {
            if let Some(site) = value.as_str() {
                if !matches!(site, "Lax" | "None" | "Strict") {
                    return Err(format!(
                        "Invalid argument type: Invalid enum value '{site}' - at `$.cookies[{index}].sameSite`"
                    ));
                }
            } else if !value.is_null() {
                return Err(type_error(
                    "str | null",
                    value,
                    &format!(".cookies[{index}].sameSite"),
                ));
            }
            cookie.insert("sameSite".to_string(), value.clone());
        }
        output.push(Value::Object(cookie));
    }
    Ok(output)
}

fn charset(content_type: &str) -> Option<String> {
    content_type
        .split(';')
        .find_map(|part| part.trim().strip_prefix("charset="))
        .map(|value| value.trim_matches(['\'', '"']).to_string())
}

fn decode_network_body(body: &Value, encoding: Option<&str>) -> Result<String, String> {
    let raw = body.get("body").and_then(Value::as_str).unwrap_or_default();
    let bytes = if body
        .get("base64Encoded")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        STANDARD.decode(raw).map_err(|e| e.to_string())?
    } else {
        raw.as_bytes().to_vec()
    };
    let decoder = encoding
        .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    Ok(decoder.decode(&bytes).0.into_owned())
}

#[allow(clippy::type_complexity)]
fn tile_screenshot(bytes: &[u8], format: &str) -> Result<(Vec<Vec<u8>>, u32, u32, bool), String> {
    let mut image =
        image::load_from_memory(bytes).map_err(|e| format!("decoding screenshot: {e}"))?;
    if image.width() > MAX_TILE_WIDTH {
        let height =
            python_round(image.height() as f64 * MAX_TILE_WIDTH as f64 / image.width() as f64)
                .max(1) as u32;
        image = image.resize_exact(MAX_TILE_WIDTH, height, FilterType::CatmullRom);
    }
    let (width, height) = image.dimensions();
    let needed = height.div_ceil(MAX_TILE_HEIGHT) as usize;
    let mut tiles = Vec::with_capacity(needed.min(MAX_TILES));
    for top in (0..height)
        .step_by(MAX_TILE_HEIGHT as usize)
        .take(MAX_TILES)
    {
        let tile = image.crop_imm(0, top, width, (height - top).min(MAX_TILE_HEIGHT));
        let mut encoded = Vec::new();
        match format {
            "png" => PngEncoder::new_with_quality(
                &mut encoded,
                CompressionType::Level(6),
                PngFilterType::Adaptive,
            )
            .write_image(
                tile.as_bytes(),
                tile.width(),
                tile.height(),
                tile.color().into(),
            )
            .map_err(|e| format!("encoding PNG tile: {e}"))?,
            "jpeg" | "jpg" => {
                let tile = tile.to_rgb8();
                let mut encoder = jpeg_encoder::Encoder::new(&mut encoded, 75);
                encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_2_0);
                encoder
                    .set_chroma_subsampling_method(jpeg_encoder::ChromaSubsamplingMethod::Average);
                encoder
                    .encode(
                        tile.as_raw(),
                        tile.width() as u16,
                        tile.height() as u16,
                        jpeg_encoder::ColorType::Rgb,
                    )
                    .map_err(|e| format!("encoding JPEG tile: {e}"))?;
            }
            other => return Err(format!("unsupported format: {other} (use png or jpeg)")),
        }
        tiles.push(encoded);
    }
    Ok((tiles, width, height, needed > MAX_TILES))
}

fn python_round(value: f64) -> i64 {
    let floor = value.floor();
    let fraction = value - floor;
    if fraction < 0.5 || (fraction == 0.5 && floor as i64 % 2 == 0) {
        floor as i64
    } else {
        floor as i64 + 1
    }
}

fn ad_domains() -> &'static HashSet<&'static str> {
    AD_DOMAINS.get_or_init(|| {
        include_str!("../../vendor/scrapling-0.4.9-ad-domains.txt")
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect()
    })
}

fn domain_matches(host: &str, domain: &str) -> bool {
    host.eq_ignore_ascii_case(domain)
        || host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn cloudflare_challenge(html: &str) -> Option<&'static str> {
    for challenge in ["non-interactive", "managed", "interactive"] {
        if html.contains(&format!("cType: '{challenge}'")) {
            return Some(challenge);
        }
    }
    let document = crate::scrapling::dom::parse(html);
    crate::scrapling::query::css_query(
        &document,
        None,
        "script[src*='challenges.cloudflare.com/turnstile/v']",
    )
    .is_ok_and(|matches| !matches.is_empty())
    .then_some("embedded")
}

fn challenge_frame_id(tree: &Value) -> Option<&str> {
    let url = tree.pointer("/frame/url").and_then(Value::as_str);
    if url.is_some_and(|url| {
        ["http://", "https://"].into_iter().any(|scheme| {
            url.strip_prefix(scheme).is_some_and(|url| {
                url.starts_with("challenges.cloudflare.com/cdn-cgi/challenge-platform/")
            })
        })
    }) {
        return tree.pointer("/frame/id").and_then(Value::as_str);
    }
    tree.get("childFrames")
        .and_then(Value::as_array)
        .and_then(|children| children.iter().find_map(challenge_frame_id))
}

fn box_origin(quad: &Value) -> Option<(f64, f64)> {
    let coordinates = quad
        .as_array()?
        .iter()
        .map(Value::as_f64)
        .collect::<Option<Vec<_>>>()?;
    if coordinates.len() != 8 {
        return None;
    }
    let xs = [
        coordinates[0],
        coordinates[2],
        coordinates[4],
        coordinates[6],
    ];
    let ys = [
        coordinates[1],
        coordinates[3],
        coordinates[5],
        coordinates[7],
    ];
    let min_x = xs.into_iter().fold(f64::INFINITY, f64::min);
    let max_x = xs.into_iter().fold(f64::NEG_INFINITY, f64::max);
    let min_y = ys.into_iter().fold(f64::INFINITY, f64::min);
    let max_y = ys.into_iter().fold(f64::NEG_INFINITY, f64::max);
    (max_x > min_x && max_y > min_y).then_some((min_x, min_y))
}

fn chromium_executable(config: &WorkerConfig, _real_chrome: bool) -> Result<PathBuf, String> {
    if !config.scrapling.chromium_executable.is_empty() {
        let path = PathBuf::from(&config.scrapling.chromium_executable);
        return path.is_file().then_some(path).ok_or_else(|| {
            "configured browser.scrapling.chromium_executable is not a file".to_string()
        });
    }
    let mut interactive = config.clone();
    interactive.executable.clear();
    crate::functions::doctor::detect_chromium(&interactive).ok_or_else(|| {
        "no Chromium executable found; configure browser.scrapling.chromium_executable".to_string()
    })
}

fn certify_chromium(path: &Path) -> Result<(), String> {
    let version = crate::functions::doctor::chromium_version(path)
        .ok_or_else(|| format!("could not read Chromium version from {}", path.display()))?;
    if version
        .split_whitespace()
        .any(|value| CERTIFIED_CHROME_VERSIONS.contains(&value))
    {
        Ok(())
    } else {
        Err(format!(
            "compat mode requires Chrome {}; {} reports {version}",
            CERTIFIED_CHROME_VERSIONS.join(" or "),
            path.display()
        ))
    }
}

fn python_string_hash(value: &str) -> u64 {
    if value.is_empty() {
        return 0;
    }
    let maximum = value.chars().map(u32::from).max().unwrap_or_default();
    let mut bytes = Vec::with_capacity(value.len());
    if maximum <= u32::from(u8::MAX) {
        bytes.extend(value.chars().map(|character| character as u8));
    } else if maximum <= u32::from(u16::MAX) {
        for character in value.chars() {
            bytes.extend_from_slice(&(character as u16).to_ne_bytes());
        }
    } else {
        for character in value.chars() {
            bytes.extend_from_slice(&u32::from(character).to_ne_bytes());
        }
    }
    let mut hasher = siphasher::sip::SipHasher13::new_with_keys(0, 0);
    hasher.write(&bytes);
    match hasher.finish() {
        u64::MAX => u64::MAX - 1,
        hash => hash,
    }
}

fn python_set_insert_clean(table: &mut [Option<(String, u64)>], key: String, hash: u64) {
    let mask = table.len() - 1;
    let mut perturb = hash as usize;
    let mut index = perturb & mask;
    loop {
        if table[index].is_none() {
            table[index] = Some((key, hash));
            return;
        }
        if index + 9 <= mask {
            for offset in 1..=9 {
                if table[index + offset].is_none() {
                    table[index + offset] = Some((key, hash));
                    return;
                }
            }
        }
        perturb >>= 5;
        index = index.wrapping_mul(5).wrapping_add(1).wrapping_add(perturb) & mask;
    }
}

fn python_set_order(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut table = vec![None; 8];
    let mut used = 0usize;
    for key in values {
        let hash = python_string_hash(&key);
        let mask = table.len() - 1;
        let mut perturb = hash as usize;
        let mut index = perturb & mask;
        let slot = loop {
            let probes = if index + 9 <= mask { 9 } else { 0 };
            let mut unused = None;
            for offset in 0..=probes {
                match &table[index + offset] {
                    None => {
                        unused = Some(index + offset);
                        break;
                    }
                    Some((existing, existing_hash))
                        if *existing_hash == hash && existing == &key =>
                    {
                        unused = None;
                        break;
                    }
                    Some(_) => {}
                }
            }
            if let Some(slot) = unused {
                break Some(slot);
            }
            if (0..=probes).any(|offset| {
                table[index + offset]
                    .as_ref()
                    .is_some_and(|(existing, existing_hash)| {
                        *existing_hash == hash && existing == &key
                    })
            }) {
                break None;
            }
            perturb >>= 5;
            index = index.wrapping_mul(5).wrapping_add(1).wrapping_add(perturb) & mask;
        };
        let Some(slot) = slot else {
            continue;
        };
        table[slot] = Some((key, hash));
        used += 1;
        if used * 5 >= mask * 3 {
            let minimum = used * 4;
            let mut size = 8usize;
            while size <= minimum {
                size *= 2;
            }
            let old = std::mem::replace(&mut table, vec![None; size]);
            for (key, hash) in old.into_iter().flatten() {
                python_set_insert_clean(&mut table, key, hash);
            }
        }
    }
    table.into_iter().flatten().map(|(key, _)| key).collect()
}

fn scrapling_browser_args(options: &RawBrowserOptions, stealth: bool) -> Vec<String> {
    if !stealth && options.extra_flags.is_empty() {
        return DEFAULT_ARGS
            .iter()
            .map(|value| (*value).to_string())
            .collect();
    }
    let mut arguments = DEFAULT_ARGS
        .iter()
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();
    if !options.extra_flags.is_empty() {
        arguments.extend(options.extra_flags.iter().cloned());
    } else {
        arguments.extend(STEALTH_ARGS.iter().map(|value| (*value).to_string()));
        if options.block_webrtc {
            arguments.extend([
                "--webrtc-ip-handling-policy=disable_non_proxied_udp".to_string(),
                "--force-webrtc-ip-handling-policy".to_string(),
            ]);
        }
        if !options.allow_webgl {
            arguments.extend([
                "--disable-webgl".to_string(),
                "--disable-webgl-image-chromium".to_string(),
                "--disable-webgl2".to_string(),
            ]);
        }
        if options.hide_canvas {
            arguments.push("--fingerprinting-canvas-image-data-noise".to_string());
        }
    }
    python_set_order(arguments)
}

fn launch_command(
    executable: &Path,
    profile: &Path,
    options: &RawBrowserOptions,
    stealth: bool,
    gate: Option<&EgressGate>,
) -> Command {
    let mut command = Command::new(executable);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for argument in if stealth {
        PATCHRIGHT_ARGS
    } else {
        PLAYWRIGHT_ARGS
    } {
        command.arg(argument);
    }
    if options.headless {
        command
            .arg("--headless")
            .arg("--hide-scrollbars")
            .arg("--mute-audio")
            .arg("--blink-settings=primaryHoverType=2,availableHoverTypes=2,primaryPointerType=4,availablePointerTypes=4");
    }
    command.arg("--no-sandbox");
    command.args(scrapling_browser_args(options, stealth));
    if options.dns_over_https {
        command.arg("--dns-over-https-templates=https://cloudflare-dns.com/dns-query");
    }
    if let Some(proxy) = gate
        .map(EgressGate::proxy_url)
        .or_else(|| options.proxy.clone())
    {
        command
            .arg(format!("--proxy-server={proxy}"))
            .arg("--proxy-bypass-list=<-loopback>");
    }
    if gate.is_some() {
        command
            .arg("--disable-quic")
            .arg("--force-webrtc-ip-handling-policy")
            .arg("--webrtc-ip-handling-policy=disable_non_proxied_udp");
    }
    command
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(crate::scrapling::cdp::REMOTE_DEBUGGING_PIPE_ARG)
        .arg("about:blank");
    command
}

struct TempProfile {
    path: PathBuf,
}

impl TempProfile {
    fn new() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!(
            "browser-scrapling-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&path).map_err(|e| format!("creating browser profile: {e}"))?;
        Ok(Self { path })
    }
}

impl Drop for TempProfile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn validates_defaults_and_safe_refusals() {
        let options = RawBrowserOptions::from_payload(&json!({})).unwrap();
        assert_eq!(options.retries, 3);
        assert_eq!(options.timeout, Duration::from_secs(30));
        assert_eq!(referer(&options), GOOGLE_REFERER);
        let options = RawBrowserOptions::from_payload(&json!({
            "proxy": "http://proxy",
            "cdp_url": "ws://remote",
            "extra_flags": ["--no-proxy-server"]
        }))
        .unwrap();
        let error = options.validate_policy(SecurityMode::Safe).unwrap_err();
        assert!(error.contains("proxy, cdp_url, extra_flags"));
        assert!(options.validate_policy(SecurityMode::Compat).is_ok());
        assert_eq!(ad_domains().len(), 3526);
        assert!(ad_domains().contains("doubleclick.net"));
        assert!(domain_matches("a.b.doubleclick.net", "doubleclick.net"));
        assert!(!domain_matches("notdoubleclick.net", "doubleclick.net"));
        assert_eq!(
            BLOCKED_RESOURCE_TYPES,
            [
                "Font",
                "Image",
                "Media",
                "TextTrack",
                "WebSocket",
                "Stylesheet"
            ]
        );
        assert_eq!(
            cloudflare_challenge("<script>cType: 'managed'</script>"),
            Some("managed")
        );
        assert_eq!(
            cloudflare_challenge(
                "<script src='https://challenges.cloudflare.com/turnstile/v0/api.js'>"
            ),
            Some("embedded")
        );
        assert_eq!(
            cloudflare_challenge("<p>https://challenges.cloudflare.com/turnstile/v0/api.js</p>"),
            None
        );
        let tree = json!({
            "frame": {"id": "root", "url": "https://example.test"},
            "childFrames": [{
                "frame": {
                    "id": "challenge",
                    "url": "https://challenges.cloudflare.com/cdn-cgi/challenge-platform/h/g"
                }
            }]
        });
        assert_eq!(challenge_frame_id(&tree), Some("challenge"));
        assert_eq!(
            box_origin(&json!([10, 20, 110, 20, 110, 70, 10, 70])),
            Some((10.0, 20.0))
        );
        assert_eq!(box_origin(&json!([10, 20, 10, 20, 10, 20, 10, 20])), None);
    }

    #[test]
    fn browser_validation_matches_frozen_msgspec_errors() {
        for (payload, expected) in [
            (
                json!({"retries": 0}),
                "Invalid argument type: Expected `int` >= 1 - at `$.retries`",
            ),
            (
                json!({"retries": 11}),
                "Invalid argument type: Expected `int` <= 10 - at `$.retries`",
            ),
            (
                json!({"max_pages": 0}),
                "Invalid argument type: Expected `int` >= 1 - at `$.max_pages`",
            ),
            (
                json!({"timeout": -1}),
                "Invalid argument type: Expected `float` >= 0.0 - at `$.timeout`",
            ),
            (
                json!({"wait_selector_state": "bogus"}),
                "Invalid argument type: Invalid enum value 'bogus' - at `$.wait_selector_state`",
            ),
            (
                json!({"cookies": {}}),
                "Invalid argument type: Expected `array | null`, got `object` - at `$.cookies`",
            ),
            (
                json!({"extra_flags": [1]}),
                "Invalid argument type: Expected `str`, got `int` - at `$.extra_flags[0]`",
            ),
            (
                json!({"cookies": [{"name": 1}]}),
                "Invalid argument type: Expected `str`, got `int` - at `$.cookies[0].name`",
            ),
            (
                json!({"extra_headers": {"x-test": 1}}),
                "Invalid argument type: Expected `str`, got `int` - at `$.extra_headers[...]`",
            ),
            (
                json!({"proxy": {"username": "user"}}),
                "Invalid argument type: Invalid proxy dictionary: Object missing required field `server`",
            ),
            (
                json!({"cdp_url": "http://example.test"}),
                "Invalid argument type: CDP URL must use 'ws://' or 'wss://' scheme",
            ),
        ] {
            assert_eq!(RawBrowserOptions::from_payload(&payload).unwrap_err(), expected);
        }
        let options = RawBrowserOptions::from_payload(&json!({
            "proxy": "http://user:pass@proxy.example:8080/path",
            "cookies": [{"name": "a", "value": "b", "unknown": true}]
        }))
        .unwrap();
        assert_eq!(options.proxy.as_deref(), Some("http://proxy.example:8080"));
        assert_eq!(
            options.proxy_auth,
            Some(("user".to_string(), "pass".to_string()))
        );
        assert_eq!(options.cookies, vec![json!({"name": "a", "value": "b"})]);
    }

    #[test]
    fn screenshot_tiles_match_pillow_dimensions_pixels_and_cap() {
        let source = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            800,
            1600,
            image::Rgba([7, 11, 13, 255]),
        ));
        let mut png = Vec::new();
        source
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let (tiles, width, height, truncated) = tile_screenshot(&png, "png").unwrap();
        assert_eq!(
            (width, height, tiles.len(), truncated),
            (800, 1600, 2, false)
        );
        assert_eq!(
            image::load_from_memory(&tiles[0])
                .unwrap()
                .get_pixel(0, 0)
                .0,
            [7, 11, 13, 255]
        );

        let tall = image::DynamicImage::new_rgb8(100, MAX_TILE_HEIGHT * 7);
        let mut png = Vec::new();
        tall.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let (tiles, _, _, truncated) = tile_screenshot(&png, "png").unwrap();
        assert_eq!(tiles.len(), 6);
        assert!(truncated);
        assert_eq!(python_round(2.5), 2);
        assert_eq!(python_round(3.5), 4);
    }

    #[test]
    fn launch_args_force_safe_proxy_and_webrtc() {
        let options = RawBrowserOptions::from_payload(&json!({})).unwrap();
        let profile = Path::new("/tmp/profile");
        let gate = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(EgressGate::start(SsrfPolicy {
                allow_loopback: true,
            }))
            .unwrap();
        let command = launch_command(
            Path::new("/bin/true"),
            profile,
            &options,
            false,
            Some(&gate),
        );
        let args = command
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert!(args
            .iter()
            .any(|value| value.starts_with("--proxy-server=http://127.0.0.1:")));
        assert!(args.contains(&"--disable-quic".to_string()));
        assert!(args.contains(&"--webrtc-ip-handling-policy=disable_non_proxied_udp".to_string()));
    }

    #[test]
    fn dynamic_launch_order_matches_the_frozen_worker() {
        let options = RawBrowserOptions::from_payload(&json!({})).unwrap();
        let command = launch_command(
            Path::new("/bin/true"),
            Path::new("/tmp/profile"),
            &options,
            false,
            None,
        );
        let actual = command
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        let expected = PLAYWRIGHT_ARGS
            .iter()
            .copied()
            .chain([
                "--headless",
                "--hide-scrollbars",
                "--mute-audio",
                "--blink-settings=primaryHoverType=2,availableHoverTypes=2,primaryPointerType=4,availablePointerTypes=4",
                "--no-sandbox",
            ])
            .chain(DEFAULT_ARGS.iter().copied())
            .chain([
                "--user-data-dir=/tmp/profile",
                "--remote-debugging-pipe",
                "about:blank",
            ])
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn cpython_set_order_matches_frozen_hash_seed_zero() {
        assert_eq!(python_string_hash("a") as i64, 4_644_417_185_603_328_019);
        assert_eq!(python_string_hash("abc") as i64, -4_594_863_902_769_663_758);
        assert_eq!(
            python_set_order(
                DEFAULT_ARGS
                    .iter()
                    .map(|value| (*value).to_string())
                    .chain(["--z-last", "--a-first", "--no-pings"].map(str::to_string))
            ),
            [
                "--homepage=about:blank",
                "--z-last",
                "--a-first",
                "--disable-breakpad",
                "--disable-infobars",
                "--no-default-browser-check",
                "--disable-hang-monitor",
                "--no-service-autorun",
                "--password-store=basic",
                "--no-pings",
                "--disable-session-crashed-bubble",
                "--disable-search-engine-choice-screen",
                "--no-first-run",
            ]
            .map(str::to_string)
        );
    }

    #[test]
    fn stealth_launch_order_matches_the_frozen_worker() {
        let options = RawBrowserOptions::from_payload(&json!({})).unwrap();
        let command = launch_command(
            Path::new("/bin/true"),
            Path::new("/tmp/profile"),
            &options,
            true,
            None,
        );
        let actual = command
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        let expected = PATCHRIGHT_ARGS
            .iter()
            .copied()
            .chain([
                "--headless",
                "--hide-scrollbars",
                "--mute-audio",
                "--blink-settings=primaryHoverType=2,availableHoverTypes=2,primaryPointerType=4,availablePointerTypes=4",
                "--no-sandbox",
            ])
            .chain(STEALTH_DEFAULT_ARGS.iter().copied())
            .chain([
                "--user-data-dir=/tmp/profile",
                "--remote-debugging-pipe",
                "about:blank",
            ])
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        assert_eq!(
            scrapling_browser_args(&options, true),
            STEALTH_DEFAULT_ARGS
                .iter()
                .map(|value| (*value).to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn explicit_stealth_flags_preserve_the_frozen_override_quirk() {
        let options = RawBrowserOptions::from_payload(&json!({
            "extra_flags": ["--z-last", "--a-first", "--no-pings"],
            "block_webrtc": true,
            "allow_webgl": false,
            "hide_canvas": true
        }))
        .unwrap();
        assert_eq!(
            scrapling_browser_args(&options, true),
            [
                "--homepage=about:blank",
                "--z-last",
                "--a-first",
                "--disable-breakpad",
                "--disable-infobars",
                "--no-default-browser-check",
                "--disable-hang-monitor",
                "--no-service-autorun",
                "--password-store=basic",
                "--no-pings",
                "--disable-session-crashed-bubble",
                "--disable-search-engine-choice-screen",
                "--no-first-run",
            ]
            .map(str::to_string)
        );
    }

    #[tokio::test]
    async fn safe_browser_fetches_loopback_through_the_gate() {
        let executable = std::env::var_os("SCRAPLING_CHROMIUM_EXECUTABLE")
            .map(PathBuf::from)
            .or_else(|| crate::functions::doctor::detect_chromium(&WorkerConfig::default()));
        let Some(executable) = executable.filter(|path| certify_chromium(path).is_ok()) else {
            return;
        };
        let origin = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = origin.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = origin.accept().await.unwrap();
                let mut request = [0; 4096];
                let _ = socket.read(&mut request).await.unwrap();
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: 48\r\nConnection: close\r\n\r\n<html><body><h1>raw cdp works</h1></body></html>",
                    )
                    .await
                    .unwrap();
            }
        });
        let mut config = WorkerConfig::default();
        config.scrapling.chromium_executable = executable.display().to_string();
        config.scrapling.allow_loopback = true;
        let options =
            RawBrowserOptions::from_payload(&json!({"retries": 1, "timeout": 5000})).unwrap();
        let page = tokio::time::timeout(Duration::from_secs(15), async {
            let browser = RawBrowser::start(&config, &options, false, false).await?;
            browser
                .fetch(&format!("http://{address}/"), &options, false)
                .await
        })
        .await
        .expect("raw browser fetch timed out")
        .unwrap();
        assert_eq!(page.status, Some(200));
        assert!(page.html.contains("raw cdp works"), "{}", page.html);
        server.abort();
    }
}
