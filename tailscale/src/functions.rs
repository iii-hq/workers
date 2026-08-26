use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use qrcode::render::svg;
use qrcode::QrCode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::config::{SharedConfig, WorkerConfig};

pub const STATUS_ID: &str = "tailscale::status";
pub const CONFIGURATION_ID: &str = "tailscale::configuration";
pub const SHARE_ID: &str = "tailscale::share";
pub const STOP_ID: &str = "tailscale::share::stop";

const STATUS_DESC: &str = "Inspect local Tailscale connectivity, node identity, health notices, and the active Serve and Funnel routes. Keys, users, and capability maps are omitted.";
const CONFIGURATION_DESC: &str = "Return the non-secret worker settings and the current Serve configuration. The CLI path is omitted.";
const SHARE_DESC: &str = "Share the local iii Console. Serve is tailnet-only and the default; Funnel is public and requires allow_funnel in the configuration plus confirm_public in the request.";
const STOP_DESC: &str = "Stop one exact route by mode, HTTPS port, and path. mode=funnel removes public access and keeps the tailnet-only route; mode=serve removes the route entirely. Other routes are never reset.";
pub const CONNECT_ID: &str = "tailscale::connect";
pub const DISCONNECT_ID: &str = "tailscale::disconnect";
const CONNECT_DESC: &str = "Connect this node to the tailnet (`tailscale up`). When the node still needs a sign-in, returns the Tailscale login URL instead of connecting.";
const DISCONNECT_DESC: &str = "Disconnect this node from the tailnet (`tailscale down`). Shared routes stop answering until the node connects again.";

const FUNNEL_PORTS: [u16; 3] = [443, 8443, 10000];
const CONNECT_TIMEOUT_SECS: u64 = 15;

pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

fn schema_of<T: JsonSchema>() -> schemars::schema::RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req: JsonSchema, Resp: JsonSchema>(
    function_id: &'static str,
    description: &'static str,
) -> FunctionSpec {
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<EmptyInput, StatusOutput>(STATUS_ID, STATUS_DESC),
        spec::<EmptyInput, ConfigurationOutput>(CONFIGURATION_ID, CONFIGURATION_DESC),
        spec::<EmptyInput, ConnectOutput>(CONNECT_ID, CONNECT_DESC),
        spec::<EmptyInput, ConnectOutput>(DISCONNECT_ID, DISCONNECT_DESC),
        spec::<ShareInput, ShareOutput>(SHARE_ID, SHARE_DESC),
        spec::<StopInput, StopOutput>(STOP_ID, STOP_DESC),
    ]
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ConnectOutput {
    /// True when the node is connected to the tailnet after the call.
    pub connected: bool,
    /// Backend state reported by the client after the call.
    pub backend_state: Option<String>,
    /// Tailscale sign-in page when the node still needs a login; open it, then call connect again.
    pub authorization_url: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct EmptyInput {}

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
    /// Local target the route proxies to.
    pub target: String,
    /// Full HTTPS URL a device opens to reach the route.
    pub url: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StatusOutput {
    /// Whether the Tailscale CLI could be started.
    pub installed: bool,
    /// Tailscale client version.
    pub version: Option<String>,
    /// Backend state reported by the client, `Running` when connected.
    pub backend_state: Option<String>,
    /// True when the client is running and this node is online.
    pub online: bool,
    /// Machine name of this node.
    pub hostname: Option<String>,
    /// MagicDNS name of this node without the trailing dot.
    pub dns_name: Option<String>,
    /// Tailscale IPv4 and IPv6 addresses of this node.
    pub tailscale_ips: Vec<String>,
    /// Health notices reported by the client.
    pub health: Vec<String>,
    /// Whether the tailnet policy allows this node to use Funnel.
    pub funnel_allowed: bool,
    /// Active Serve and Funnel routes on this node.
    pub routes: Vec<Route>,
    /// Error text when the client could not be queried.
    pub error: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ConfigurationOutput {
    /// Local Console URL the routes proxy to.
    pub console_url: String,
    /// HTTPS port used when a share request omits one.
    pub default_https_port: u16,
    /// Whether public Funnel shares are permitted by the operator.
    pub allow_funnel: bool,
    /// Per-command timeout for the Tailscale CLI.
    pub command_timeout_ms: u64,
    /// Active Serve and Funnel routes on this node.
    pub routes: Vec<Route>,
    /// Raw `tailscale serve status --json` output.
    pub serve_config: Value,
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
    /// URL to open: the shared Console, or the Tailscale authorization page when `stage` is `authorization_required`.
    pub url: String,
    /// QR code for `url` as inline SVG markup.
    pub qr_svg: String,
    /// Tailscale page that enables Funnel for this node, present only when authorization is required.
    pub authorization_url: Option<String>,
    /// Local Console URL the route proxies to.
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
    /// True when the route was removed.
    pub stopped: bool,
    /// Mode of the stopped route.
    pub mode: ShareMode,
    /// HTTPS port of the stopped route.
    pub https_port: u16,
    /// URL path prefix of the stopped route.
    pub path: String,
}

pub fn register_all(iii: &IIIClient, config: SharedConfig) {
    register_status(iii, config.clone());
    register_configuration(iii, config.clone());
    register_connect(iii, config.clone());
    register_disconnect(iii, config.clone());
    register_share(iii, config.clone());
    register_stop(iii, config);
}

fn register_connect(iii: &IIIClient, config: SharedConfig) {
    iii.register_function(
        CONNECT_ID,
        RegisterFunction::new_async(move |_: EmptyInput| {
            let config = config.load_full();
            async move { connect(&config).await.map_err(Error::Handler) }
        })
        .description(CONNECT_DESC),
    );
}

fn register_disconnect(iii: &IIIClient, config: SharedConfig) {
    iii.register_function(
        DISCONNECT_ID,
        RegisterFunction::new_async(move |_: EmptyInput| {
            let config = config.load_full();
            async move { disconnect(&config).await.map_err(Error::Handler) }
        })
        .description(DISCONNECT_DESC),
    );
}

async fn connect(config: &WorkerConfig) -> Result<ConnectOutput, String> {
    let mut child = Command::new(&config.tailscale_binary)
        .arg("up")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start Tailscale CLI: {error}"))?;
    let transcript = Arc::new(Mutex::new(String::new()));
    let readers = [
        child
            .stdout
            .take()
            .map(|out| spawn_line_reader(Box::pin(out), transcript.clone())),
        child
            .stderr
            .take()
            .map(|err| spawn_line_reader(Box::pin(err), transcript.clone())),
    ];
    let exit = timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS), child.wait()).await;
    if exit.is_err() {
        let _ = child.kill().await;
    }
    for reader in readers.into_iter().flatten() {
        let _ = reader.await;
    }
    let output = transcript.lock().await.clone();
    let authorization_url = login_url(&output);
    match exit {
        Ok(Ok(status)) if !status.success() && authorization_url.is_none() => {
            let text = output.trim().to_string();
            return Err(if text.is_empty() {
                format!("tailscale up exited with {status}")
            } else {
                text
            });
        }
        Ok(Err(error)) => return Err(format!("Tailscale CLI failed: {error}")),
        Err(_) if authorization_url.is_none() => {
            return Err(format!(
                "tailscale up did not finish within {CONNECT_TIMEOUT_SECS}s"
            ));
        }
        _ => {}
    }
    let state = run_json(config, &["status", "--json"]).await?;
    let backend_state = string_at(&state, "/BackendState");
    let connected = backend_state.as_deref() == Some("Running")
        && state
            .pointer("/Self/Online")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    Ok(ConnectOutput {
        connected,
        backend_state,
        authorization_url,
    })
}

async fn disconnect(config: &WorkerConfig) -> Result<ConnectOutput, String> {
    run(config, &["down"]).await?;
    let state = run_json(config, &["status", "--json"]).await?;
    Ok(ConnectOutput {
        connected: false,
        backend_state: string_at(&state, "/BackendState"),
        authorization_url: None,
    })
}

fn spawn_line_reader(
    stream: Pin<Box<dyn AsyncRead + Send>>,
    transcript: Arc<Mutex<String>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut buffer = transcript.lock().await;
            buffer.push_str(&line);
            buffer.push('\n');
        }
    })
}

pub fn login_url(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|word| word.starts_with("https://login.tailscale.com/a/"))
        .map(|word| {
            word.trim_end_matches(|c: char| !c.is_ascii_alphanumeric())
                .to_string()
        })
}

fn register_status(iii: &IIIClient, config: SharedConfig) {
    iii.register_function(
        STATUS_ID,
        RegisterFunction::new_async(move |_: EmptyInput| {
            let config = config.load_full();
            async move { Ok::<_, Error>(status(&config).await) }
        })
        .description(STATUS_DESC),
    );
}

fn register_configuration(iii: &IIIClient, config: SharedConfig) {
    iii.register_function(
        CONFIGURATION_ID,
        RegisterFunction::new_async(move |_: EmptyInput| {
            let config = config.load_full();
            async move {
                let serve_config = serve_status(&config)
                    .await
                    .unwrap_or_else(|_| Value::Object(Default::default()));
                Ok::<_, Error>(ConfigurationOutput {
                    console_url: config.console_url.clone(),
                    default_https_port: config.default_https_port,
                    allow_funnel: config.allow_funnel,
                    command_timeout_ms: config.command_timeout_ms,
                    routes: normalize_routes(&serve_config),
                    serve_config,
                })
            }
        })
        .description(CONFIGURATION_DESC),
    );
}

fn register_share(iii: &IIIClient, config: SharedConfig) {
    iii.register_function(
        SHARE_ID,
        RegisterFunction::new_async(move |input: ShareInput| {
            let config = config.load_full();
            async move { share(&config, input).await.map_err(Error::Handler) }
        })
        .description(SHARE_DESC),
    );
}

fn register_stop(iii: &IIIClient, config: SharedConfig) {
    iii.register_function(
        STOP_ID,
        RegisterFunction::new_async(move |input: StopInput| {
            let config = config.load_full();
            async move { stop(&config, input).await.map_err(Error::Handler) }
        })
        .description(STOP_DESC),
    );
}

async fn status(config: &WorkerConfig) -> StatusOutput {
    let status_json = match run_json(config, &["status", "--json"]).await {
        Ok(value) => value,
        Err(error) => {
            return StatusOutput {
                installed: !error.contains("could not start"),
                version: None,
                backend_state: None,
                online: false,
                hostname: None,
                dns_name: None,
                tailscale_ips: vec![],
                health: vec![],
                funnel_allowed: false,
                routes: vec![],
                error: Some(error),
            }
        }
    };
    let routes = serve_status(config)
        .await
        .map(|value| normalize_routes(&value))
        .unwrap_or_default();
    let backend_state = string_at(&status_json, "/BackendState");
    let online = backend_state.as_deref() == Some("Running")
        && status_json
            .pointer("/Self/Online")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    StatusOutput {
        installed: true,
        version: string_at(&status_json, "/Version"),
        backend_state,
        online,
        hostname: string_at(&status_json, "/Self/HostName"),
        dns_name: dns_name(&status_json),
        tailscale_ips: strings_at(&status_json, "/TailscaleIPs"),
        health: strings_at(&status_json, "/Health"),
        funnel_allowed: funnel_allowed(&status_json),
        routes,
        error: None,
    }
}

async fn share(config: &WorkerConfig, input: ShareInput) -> Result<ShareOutput, String> {
    let port = input.https_port.unwrap_or(config.default_https_port);
    let path = normalize_path(&input.path)?;
    validate_port(input.mode, port)?;
    if input.mode == ShareMode::Funnel && (!config.allow_funnel || !input.confirm_public) {
        return Err("public Funnel requires allow_funnel=true in the configuration and confirm_public=true in this request".to_string());
    }

    let state = run_json(config, &["status", "--json"]).await?;
    if string_at(&state, "/BackendState").as_deref() != Some("Running") {
        return Err(
            "Tailscale is not running; connect Tailscale before sharing the Console".to_string(),
        );
    }
    if input.mode == ShareMode::Funnel && !funnel_allowed(&state) {
        let node_id = string_at(&state, "/Self/ID")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                "Tailscale did not report a node ID for Funnel authorization".to_string()
            })?;
        let authorization_url = format!("https://login.tailscale.com/f/funnel?node={node_id}");
        return Ok(ShareOutput {
            stage: ShareStage::AuthorizationRequired,
            mode: input.mode,
            public: false,
            url: authorization_url.clone(),
            qr_svg: qr_svg(&authorization_url)?,
            authorization_url: Some(authorization_url),
            target: config.console_url.clone(),
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
            input.mode.command(),
            "--bg",
            "--yes",
            "--https",
            &port_arg,
            "--set-path",
            &path,
            &config.console_url,
        ],
    )
    .await?;

    let url = route_url(&dns_name, port, &path);
    Ok(ShareOutput {
        stage: ShareStage::Ready,
        mode: input.mode,
        public: input.mode == ShareMode::Funnel,
        qr_svg: qr_svg(&url)?,
        url,
        authorization_url: None,
        target: config.console_url.clone(),
        https_port: port,
        path,
    })
}

async fn stop(config: &WorkerConfig, input: StopInput) -> Result<StopOutput, String> {
    let path = normalize_path(&input.path)?;
    validate_port(input.mode, input.https_port)?;
    let port = input.https_port.to_string();
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
                &config.console_url,
            ],
        )
        .await?;
    } else {
        off("serve").await?;
    }
    Ok(StopOutput {
        stopped: true,
        mode: input.mode,
        https_port: input.https_port,
        path,
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

pub fn funnel_allowed(status: &Value) -> bool {
    let capabilities = status
        .pointer("/Self/Capabilities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    let cap_map = status
        .pointer("/Self/CapMap")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|map| map.keys().map(String::as_str));
    capabilities
        .chain(cap_map)
        .any(|capability| capability.to_ascii_lowercase().contains("funnel"))
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

pub fn route_url(host: &str, port: u16, path: &str) -> String {
    let suffix = if path == "/" { "" } else { path };
    if port == 443 {
        format!("https://{host}{suffix}/")
    } else {
        format!("https://{host}:{port}{suffix}/")
    }
}

fn dns_name(status: &Value) -> Option<String> {
    string_at(status, "/Self/DNSName")
        .map(|value| value.trim_end_matches('.').to_string())
        .filter(|value| !value.is_empty())
}

async fn serve_status(config: &WorkerConfig) -> Result<Value, String> {
    run_json(config, &["serve", "status", "--json"]).await
}

async fn run_json(config: &WorkerConfig, args: &[&str]) -> Result<Value, String> {
    let output = run_output(config, args).await?;
    if output.iter().all(u8::is_ascii_whitespace) {
        return Ok(Value::Object(Default::default()));
    }
    serde_json::from_slice(&output)
        .map_err(|error| format!("Tailscale returned invalid JSON: {error}"))
}

async fn run(config: &WorkerConfig, args: &[&str]) -> Result<(), String> {
    run_output(config, args).await.map(|_| ())
}

async fn run_output(config: &WorkerConfig, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = Command::new(&config.tailscale_binary);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command
        .spawn()
        .map_err(|error| format!("could not start Tailscale CLI: {error}"))?;
    let output = timeout(
        Duration::from_millis(config.command_timeout_ms),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| {
        format!(
            "Tailscale CLI timed out after {} ms",
            config.command_timeout_ms
        )
    })?
    .map_err(|error| format!("Tailscale CLI failed: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("Tailscale CLI exited with {}", output.status)
        } else {
            stderr
        });
    }
    Ok(output.stdout)
}

fn string_at(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn strings_at(value: &Value, pointer: &str) -> Vec<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn split_host_port(listener: &str) -> Option<(String, u16)> {
    let (host, port) = listener.rsplit_once(':')?;
    let host = host.trim_matches(|c| c == '[' || c == ']');
    Some((host.to_string(), port.parse().ok()?))
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
            let Some(target) = handler.get("Proxy").and_then(Value::as_str) else {
                continue;
            };
            let path = if path.is_empty() { "/" } else { path.as_str() };
            routes.push(Route {
                mode,
                host: host.clone(),
                port,
                path: path.to_string(),
                target: target.to_string(),
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
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].mode, ShareMode::Serve);
        assert_eq!(routes[0].port, 443);
        assert_eq!(routes[0].path, "/");
        assert_eq!(routes[0].url, "https://node.ts.net/");
        assert_eq!(routes[1].mode, ShareMode::Funnel);
        assert_eq!(routes[1].path, "/remote");
        assert_eq!(routes[1].url, "https://node.ts.net:8443/remote/");
        assert_eq!(routes[1].target, "http://127.0.0.1:3113");
    }

    #[test]
    fn login_url_is_extracted_from_cli_output() {
        let text = "To authenticate, visit:\n\n\thttps://login.tailscale.com/a/abc123def\n\n";
        assert_eq!(
            login_url(text).as_deref(),
            Some("https://login.tailscale.com/a/abc123def")
        );
        assert!(login_url("Success.").is_none());
    }

    #[test]
    fn empty_serve_config_yields_no_routes() {
        assert!(normalize_routes(&serde_json::json!({})).is_empty());
        assert!(normalize_routes(&serde_json::json!({"Web": {"bad": {}}})).is_empty());
    }

    #[test]
    fn detects_funnel_authorization_in_capabilities_and_cap_map() {
        let capability =
            serde_json::json!({"Self": {"Capabilities": ["https://tailscale.com/cap/funnel"]}});
        let cap_map = serde_json::json!({"Self": {"CapMap": {"https://tailscale.com/cap/funnel-ports": [443]}}});
        let missing = serde_json::json!({"Self": {"Capabilities": ["https"]}});
        assert!(funnel_allowed(&capability));
        assert!(funnel_allowed(&cap_map));
        assert!(!funnel_allowed(&missing));
    }
}
