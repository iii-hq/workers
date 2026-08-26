use iii_sdk::IIIClient;
use qrcode::render::svg;
use qrcode::QrCode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::node::{dns_name, funnel_allowed, status_json};
use super::{register_fn, run, run_json, spec, string_at, EmptyInput, FunctionSpec};
use crate::config::{SharedConfig, WorkerConfig};

pub const SHARE_ID: &str = "tailscale::share";
pub const STOP_ID: &str = "tailscale::share::stop";
pub const SERVE_LIST_ID: &str = "tailscale::serve::list";
pub const SERVE_ADD_ID: &str = "tailscale::serve::add";
pub const SERVE_REMOVE_ID: &str = "tailscale::serve::remove";
pub const SERVE_RESET_ID: &str = "tailscale::serve::reset";

const SHARE_DESC: &str = "Share the local iii Console. Serve is tailnet-only and the default; Funnel is public and requires allow_funnel in the configuration plus confirm_public in the request.";
const STOP_DESC: &str = "Stop one Console share by mode, HTTPS port, and path. mode=funnel removes public access and keeps the tailnet-only route; mode=serve removes the route entirely. Other routes are never reset.";
const SERVE_LIST_DESC: &str =
    "List every Serve and Funnel route on this node with its URL, target, and visibility.";
const SERVE_ADD_DESC: &str = "Publish any local service, port, or directory on this node over Tailscale Serve (tailnet only) or Funnel (public; needs allow_funnel and confirm_public).";
const SERVE_REMOVE_DESC: &str = "Remove one route by mode, HTTPS port, and path. mode=funnel removes public access and keeps the tailnet-only route with its original target; mode=serve removes the route entirely.";
const SERVE_RESET_DESC: &str = "Remove every Serve and Funnel route on this node (`serve reset` and `funnel reset`). Requires confirm=true.";

const FUNNEL_PORTS: [u16; 3] = [443, 8443, 10000];

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<ShareInput, ShareOutput>(SHARE_ID, SHARE_DESC),
        spec::<StopInput, StopOutput>(STOP_ID, STOP_DESC),
        spec::<EmptyInput, RoutesOutput>(SERVE_LIST_ID, SERVE_LIST_DESC),
        spec::<ServeAddInput, ShareOutput>(SERVE_ADD_ID, SERVE_ADD_DESC),
        spec::<StopInput, StopOutput>(SERVE_REMOVE_ID, SERVE_REMOVE_DESC),
        spec::<ResetInput, RoutesOutput>(SERVE_RESET_ID, SERVE_RESET_DESC),
    ]
}

pub fn register(iii: &IIIClient, config: &SharedConfig) {
    register_fn!(iii, config, SHARE_ID, SHARE_DESC, ShareInput, share);
    register_fn!(iii, config, STOP_ID, STOP_DESC, StopInput, stop);
    register_fn!(
        iii,
        config,
        SERVE_LIST_ID,
        SERVE_LIST_DESC,
        EmptyInput,
        serve_list
    );
    register_fn!(
        iii,
        config,
        SERVE_ADD_ID,
        SERVE_ADD_DESC,
        ServeAddInput,
        serve_add
    );
    register_fn!(
        iii,
        config,
        SERVE_REMOVE_ID,
        SERVE_REMOVE_DESC,
        StopInput,
        serve_remove
    );
    register_fn!(
        iii,
        config,
        SERVE_RESET_ID,
        SERVE_RESET_DESC,
        ResetInput,
        serve_reset
    );
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ShareMode {
    Serve,
    Funnel,
}

impl ShareMode {
    fn command(self) -> &'static str {
        match self {
            Self::Serve => "serve",
            Self::Funnel => "funnel",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Route {
    /// `serve` for a tailnet-only route, `funnel` when the same listener is also published to the internet.
    pub mode: ShareMode,
    /// MagicDNS host name the route answers on.
    pub host: String,
    /// HTTPS port of the listener.
    pub port: u16,
    /// URL path prefix the route serves.
    pub path: String,
    /// Local target the route proxies to, or the file path it serves.
    pub target: String,
    /// Full HTTPS URL a device opens to reach the route.
    pub url: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RoutesOutput {
    /// Active Serve and Funnel routes on this node.
    pub routes: Vec<Route>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ShareInput {
    /// `serve` (tailnet only, default) or `funnel` (public internet).
    #[serde(default = "default_mode")]
    pub mode: ShareMode,
    /// HTTPS port for the listener; defaults to the configured port. Funnel accepts 443, 8443, and 10000.
    pub https_port: Option<u16>,
    /// URL path prefix to serve the Console under; defaults to `/`.
    #[serde(default = "default_path")]
    pub path: String,
    /// Required `true` for Funnel: acknowledges that the Console becomes reachable by anyone with the link.
    #[serde(default)]
    pub confirm_public: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ServeAddInput {
    /// `serve` (tailnet only, default) or `funnel` (public internet).
    #[serde(default = "default_mode")]
    pub mode: ShareMode,
    /// HTTPS port for the listener; defaults to the configured port. Funnel accepts 443, 8443, and 10000.
    pub https_port: Option<u16>,
    /// URL path prefix to publish under; defaults to `/`.
    #[serde(default = "default_path")]
    pub path: String,
    /// What to publish: a local port (`3000`), a loopback URL (`http://127.0.0.1:3000`, `https+insecure://localhost:8443`), or an absolute file or directory path.
    pub target: String,
    /// Required `true` for Funnel: acknowledges that the target becomes reachable by anyone with the link.
    #[serde(default)]
    pub confirm_public: bool,
}

fn default_mode() -> ShareMode {
    ShareMode::Serve
}

fn default_path() -> String {
    "/".to_string()
}

#[derive(Debug, Clone, Copy, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShareStage {
    AuthorizationRequired,
    Ready,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ShareOutput {
    /// `ready` when the route is live; `authorization_required` when Funnel must first be enabled for this node.
    pub stage: ShareStage,
    /// Mode that was requested.
    pub mode: ShareMode,
    /// True when the route is reachable from the public internet.
    pub public: bool,
    /// URL to open: the published route, or the Tailscale authorization page when `stage` is `authorization_required`.
    pub url: String,
    /// QR code for `url` as inline SVG markup.
    pub qr_svg: String,
    /// Tailscale page that enables Funnel for this node, present only when authorization is required.
    pub authorization_url: Option<String>,
    /// Local target the route proxies to.
    pub target: String,
    /// HTTPS port of the listener.
    pub https_port: u16,
    /// URL path prefix the route serves.
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StopInput {
    /// Mode of the route to stop.
    pub mode: ShareMode,
    /// HTTPS port of the route to stop.
    pub https_port: u16,
    /// URL path prefix of the route to stop; defaults to `/`.
    #[serde(default = "default_path")]
    pub path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StopOutput {
    /// True when the route was changed.
    pub stopped: bool,
    /// Mode that was stopped.
    pub mode: ShareMode,
    /// HTTPS port of the route.
    pub https_port: u16,
    /// URL path prefix of the route.
    pub path: String,
    /// Route that remains on that listener and path after the call, if any.
    pub remaining: Option<Route>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ResetInput {
    /// Must be `true`: every Serve and Funnel route on this node is removed.
    #[serde(default)]
    pub confirm: bool,
}

async fn share(config: &WorkerConfig, input: ShareInput) -> Result<ShareOutput, String> {
    publish(
        config,
        input.mode,
        input.https_port,
        &input.path,
        &config.console_url,
        input.confirm_public,
    )
    .await
}

async fn serve_add(config: &WorkerConfig, input: ServeAddInput) -> Result<ShareOutput, String> {
    let target = validate_target(&input.target)?;
    publish(
        config,
        input.mode,
        input.https_port,
        &input.path,
        &target,
        input.confirm_public,
    )
    .await
}

async fn publish(
    config: &WorkerConfig,
    mode: ShareMode,
    https_port: Option<u16>,
    path: &str,
    target: &str,
    confirm_public: bool,
) -> Result<ShareOutput, String> {
    let port = https_port.unwrap_or(config.default_https_port);
    let path = normalize_path(path)?;
    validate_port(mode, port)?;
    if mode == ShareMode::Funnel && (!config.allow_funnel || !confirm_public) {
        return Err("public Funnel requires allow_funnel=true in the configuration and confirm_public=true in this request".to_string());
    }

    let state = status_json(config).await?;
    if string_at(&state, "/BackendState").as_deref() != Some("Running") {
        return Err("Tailscale is not running; connect Tailscale before publishing".to_string());
    }
    if mode == ShareMode::Funnel && !funnel_allowed(&state) {
        let node_id = string_at(&state, "/Self/ID")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                "Tailscale did not report a node ID for Funnel authorization".to_string()
            })?;
        let authorization_url = format!("https://login.tailscale.com/f/funnel?node={node_id}");
        return Ok(ShareOutput {
            stage: ShareStage::AuthorizationRequired,
            mode,
            public: false,
            url: authorization_url.clone(),
            qr_svg: qr_svg(&authorization_url)?,
            authorization_url: Some(authorization_url),
            target: target.to_string(),
            https_port: port,
            path,
        });
    }

    let dns_name = dns_name(&state)
        .ok_or_else(|| "Tailscale did not report a MagicDNS name for this device".to_string())?;

    let port_arg = port.to_string();
    run(
        config,
        &[
            mode.command(),
            "--bg",
            "--yes",
            "--https",
            &port_arg,
            "--set-path",
            &path,
            target,
        ],
    )
    .await?;

    let url = route_url(&dns_name, port, &path);
    Ok(ShareOutput {
        stage: ShareStage::Ready,
        mode,
        public: mode == ShareMode::Funnel,
        qr_svg: qr_svg(&url)?,
        url,
        authorization_url: None,
        target: target.to_string(),
        https_port: port,
        path,
    })
}

async fn stop(config: &WorkerConfig, input: StopInput) -> Result<StopOutput, String> {
    stop_route(config, input, Some(config.console_url.clone())).await
}

async fn serve_remove(config: &WorkerConfig, input: StopInput) -> Result<StopOutput, String> {
    stop_route(config, input, None).await
}

async fn stop_route(
    config: &WorkerConfig,
    input: StopInput,
    fallback_target: Option<String>,
) -> Result<StopOutput, String> {
    let path = normalize_path(&input.path)?;
    validate_port(input.mode, input.https_port)?;
    let port = input.https_port.to_string();
    let existing = serve_status(config)
        .await
        .map(|value| normalize_routes(&value))
        .unwrap_or_default()
        .into_iter()
        .find(|route| route.port == input.https_port && route.path == path);
    let off = |command: &'static str| {
        let port = port.clone();
        let path = path.clone();
        async move {
            run(
                config,
                &[
                    command,
                    "--yes",
                    "--https",
                    &port,
                    "--set-path",
                    &path,
                    "off",
                ],
            )
            .await
        }
    };
    if input.mode == ShareMode::Funnel {
        off("funnel").await?;
        let target = existing
            .as_ref()
            .map(|route| route.target.clone())
            .or(fallback_target);
        if let Some(target) = target {
            run(
                config,
                &[
                    "serve",
                    "--bg",
                    "--yes",
                    "--https",
                    &port,
                    "--set-path",
                    &path,
                    &target,
                ],
            )
            .await?;
        }
    } else {
        off("serve").await?;
    }
    let remaining = serve_status(config)
        .await
        .map(|value| normalize_routes(&value))
        .unwrap_or_default()
        .into_iter()
        .find(|route| route.port == input.https_port && route.path == path);
    Ok(StopOutput {
        stopped: true,
        mode: input.mode,
        https_port: input.https_port,
        path,
        remaining,
    })
}

async fn serve_list(config: &WorkerConfig, _: EmptyInput) -> Result<RoutesOutput, String> {
    let value = serve_status(config).await?;
    Ok(RoutesOutput {
        routes: normalize_routes(&value),
    })
}

async fn serve_reset(config: &WorkerConfig, input: ResetInput) -> Result<RoutesOutput, String> {
    if !input.confirm {
        return Err("serve reset removes every route on this node; pass confirm=true".to_string());
    }
    run(config, &["funnel", "reset"]).await?;
    run(config, &["serve", "reset"]).await?;
    let value = serve_status(config).await?;
    Ok(RoutesOutput {
        routes: normalize_routes(&value),
    })
}

fn qr_svg(value: &str) -> Result<String, String> {
    QrCode::new(value.as_bytes())
        .map_err(|error| format!("could not generate QR code: {error}"))
        .map(|code| {
            code.render::<svg::Color>()
                .min_dimensions(240, 240)
                .dark_color(svg::Color("#11170f"))
                .light_color(svg::Color("#ffffff"))
                .build()
        })
}

pub fn validate_port(mode: ShareMode, port: u16) -> Result<(), String> {
    if port == 0 {
        return Err("https_port must be between 1 and 65535".to_string());
    }
    if mode == ShareMode::Funnel && !FUNNEL_PORTS.contains(&port) {
        return Err("Tailscale Funnel supports HTTPS ports 443, 8443, and 10000".to_string());
    }
    Ok(())
}

pub fn normalize_path(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return Ok("/".to_string());
    }
    if !trimmed.starts_with('/') {
        return Err("path must start with /".to_string());
    }
    if trimmed.contains('?')
        || trimmed.contains('#')
        || trimmed.contains("..")
        || trimmed.chars().any(char::is_whitespace)
    {
        return Err(
            "path must not contain whitespace, query, fragment, or .. segments".to_string(),
        );
    }
    Ok(trimmed.trim_end_matches('/').to_string())
}

pub fn validate_target(raw: &str) -> Result<String, String> {
    let target = raw.trim();
    if target.is_empty() || target.starts_with('-') || target.chars().any(char::is_whitespace) {
        return Err("target must be a port, a loopback URL, or an absolute path".to_string());
    }
    if target.chars().all(|c| c.is_ascii_digit()) {
        let port: u32 = target
            .parse()
            .map_err(|_| "target port is not a number".to_string())?;
        if port == 0 || port > 65_535 {
            return Err("target port must be between 1 and 65535".to_string());
        }
        return Ok(target.to_string());
    }
    if target.starts_with('/') {
        return Ok(target.to_string());
    }
    let lower = target.to_ascii_lowercase();
    let schemes = [
        "http://",
        "https://",
        "http+insecure://",
        "https+insecure://",
    ];
    let Some(scheme) = schemes.iter().find(|scheme| lower.starts_with(*scheme)) else {
        return Err("target must be a port, a loopback URL, or an absolute path".to_string());
    };
    let rest = &lower[scheme.len()..];
    let host = match rest.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or_default(),
        None => rest.split(['/', ':']).next().unwrap_or_default(),
    };
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return Err("target URL must point at 127.0.0.1, localhost, or ::1".to_string());
    }
    Ok(target.to_string())
}

pub fn route_url(host: &str, port: u16, path: &str) -> String {
    let suffix = if path == "/" { "" } else { path };
    if port == 443 {
        format!("https://{host}{suffix}/")
    } else {
        format!("https://{host}:{port}{suffix}/")
    }
}

pub(crate) async fn serve_status(config: &WorkerConfig) -> Result<Value, String> {
    run_json(config, &["serve", "status", "--json"]).await
}

fn split_host_port(listener: &str) -> Option<(String, u16)> {
    let (host, port) = listener.rsplit_once(':')?;
    let host = host.trim_matches(|c| c == '[' || c == ']');
    Some((host.to_string(), port.parse().ok()?))
}

fn handler_target(handler: &Value) -> Option<String> {
    ["Proxy", "Path", "Text"]
        .iter()
        .find_map(|key| handler.get(*key).and_then(Value::as_str))
        .map(ToOwned::to_owned)
}

pub fn normalize_routes(config: &Value) -> Vec<Route> {
    let mut routes = Vec::new();
    let Some(web) = config.get("Web").and_then(Value::as_object) else {
        return routes;
    };
    for (listener, entry) in web {
        let Some((host, port)) = split_host_port(listener) else {
            continue;
        };
        let public = config
            .pointer(&format!(
                "/AllowFunnel/{}",
                listener.replace('~', "~0").replace('/', "~1")
            ))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mode = if public {
            ShareMode::Funnel
        } else {
            ShareMode::Serve
        };
        let Some(handlers) = entry.get("Handlers").and_then(Value::as_object) else {
            continue;
        };
        for (path, handler) in handlers {
            let Some(target) = handler_target(handler) else {
                continue;
            };
            let path = if path.is_empty() { "/" } else { path.as_str() };
            routes.push(Route {
                mode,
                host: host.clone(),
                port,
                path: path.to_string(),
                target,
                url: route_url(&host, port, path),
            });
        }
    }
    routes.sort_by(|left, right| {
        (&left.host, left.port, &left.path).cmp(&(&right.host, right.port, &right.path))
    });
    routes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn funnel_requires_supported_port() {
        assert!(validate_port(ShareMode::Funnel, 443).is_ok());
        assert!(validate_port(ShareMode::Funnel, 8443).is_ok());
        assert!(validate_port(ShareMode::Funnel, 10000).is_ok());
        assert!(validate_port(ShareMode::Funnel, 3113).is_err());
        assert!(validate_port(ShareMode::Serve, 3113).is_ok());
        assert!(validate_port(ShareMode::Serve, 0).is_err());
    }

    #[test]
    fn paths_are_strict_and_normalized() {
        assert_eq!(normalize_path("/remote/").unwrap(), "/remote");
        assert_eq!(normalize_path("").unwrap(), "/");
        assert!(normalize_path("remote").is_err());
        assert!(normalize_path("/../admin").is_err());
        assert!(normalize_path("/a b").is_err());
    }

    #[test]
    fn targets_are_ports_loopback_urls_or_paths() {
        assert_eq!(validate_target("3000").unwrap(), "3000");
        assert_eq!(
            validate_target("http://127.0.0.1:3000").unwrap(),
            "http://127.0.0.1:3000"
        );
        assert_eq!(
            validate_target("https+insecure://localhost:8443").unwrap(),
            "https+insecure://localhost:8443"
        );
        assert_eq!(validate_target("/srv/site").unwrap(), "/srv/site");
        assert_eq!(
            validate_target("http://[::1]:3000").unwrap(),
            "http://[::1]:3000"
        );
        assert!(validate_target("http://example.com").is_err());
        assert!(validate_target("http://[2001:db8::1]:3000").is_err());
        assert!(validate_target("70000").is_err());
        assert!(validate_target("--bg").is_err());
        assert!(validate_target("relative/path").is_err());
    }

    #[test]
    fn route_urls_omit_the_default_port_and_root_path() {
        assert_eq!(route_url("node.ts.net", 443, "/"), "https://node.ts.net/");
        assert_eq!(
            route_url("node.ts.net", 8443, "/remote"),
            "https://node.ts.net:8443/remote/"
        );
    }

    #[test]
    fn routes_come_from_web_handlers_and_allow_funnel() {
        let value = serde_json::json!({
            "TCP": {"443": {"HTTPS": true}, "8443": {"HTTPS": true}},
            "Web": {
                "node.ts.net:443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:3113"}}},
                "node.ts.net:8443": {"Handlers": {"/remote": {"Proxy": "http://127.0.0.1:3113"}, "/files": {"Path": "/srv"}}}
            },
            "AllowFunnel": {"node.ts.net:8443": true},
            "Secret": "nodekey:private"
        });
        let routes = normalize_routes(&value);
        assert_eq!(routes.len(), 3);
        assert_eq!(routes[0].mode, ShareMode::Serve);
        assert_eq!(routes[0].url, "https://node.ts.net/");
        assert_eq!(routes[1].mode, ShareMode::Funnel);
        assert_eq!(routes[1].path, "/files");
        assert_eq!(routes[1].target, "/srv");
        assert_eq!(routes[2].url, "https://node.ts.net:8443/remote/");
    }

    #[test]
    fn empty_serve_config_yields_no_routes() {
        assert!(normalize_routes(&serde_json::json!({})).is_empty());
        assert!(normalize_routes(&serde_json::json!({"Web": {"bad": {}}})).is_empty());
    }
}
