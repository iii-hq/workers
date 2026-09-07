use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::peers::{parse_prefs, up_args};
use super::share::{normalize_routes, serve_status, Route};
use super::{
    bool_at, register_fn, run, run_json, run_text, spec, string_at, strings_at, EmptyInput,
    FunctionSpec,
};
use crate::config::{SharedConfig, WorkerConfig};

pub const STATUS_ID: &str = "tailscale::status";
pub const CONFIGURATION_ID: &str = "tailscale::configuration";
pub const CONNECT_ID: &str = "tailscale::connect";
pub const DISCONNECT_ID: &str = "tailscale::disconnect";
pub const LOGIN_ID: &str = "tailscale::login";
pub const LOGOUT_ID: &str = "tailscale::logout";
pub const VERSION_ID: &str = "tailscale::version";
pub const IP_ID: &str = "tailscale::ip";
pub const NETCHECK_ID: &str = "tailscale::netcheck";
pub const PING_ID: &str = "tailscale::ping";
pub const WHOIS_ID: &str = "tailscale::whois";
pub const DNS_STATUS_ID: &str = "tailscale::dns::status";
pub const DNS_QUERY_ID: &str = "tailscale::dns::query";

const STATUS_DESC: &str = "Check if Tailscale is connected on this node, plus identity, health notices, and Serve and Funnel routes. Keys, users, and capability maps are omitted.";
const CONFIGURATION_DESC: &str = "Return the non-secret worker settings and the current Serve configuration. The CLI path is omitted.";
const CONNECT_DESC: &str = "Connect this node to the tailnet (`tailscale up`). When the node still needs a sign-in, returns the Tailscale login URL instead of connecting.";
const DISCONNECT_DESC: &str = "Disconnect this node from the tailnet (`tailscale down`). Shared routes stop answering until the node connects again.";
const LOGIN_DESC: &str = "Sign in to Tailscale on this node (`tailscale login`). Returns the browser URL a person completes it at; call connect afterwards.";
const LOGOUT_DESC: &str = "Log this node out (`tailscale logout`): disconnects and expires the node key, so the next connect needs a fresh sign-in.";
const VERSION_DESC: &str =
    "Report the Tailscale client version, and the latest upstream release for the track.";
const IP_DESC: &str = "Get this node's Tailscale IP addresses, or a peer's by name or IP.";
const NETCHECK_DESC: &str = "Analyse the local network for Tailscale: UDP reachability, IPv4/IPv6, NAT mapping, port-mapping protocols, the preferred DERP relay and relay latencies.";
const PING_DESC: &str = "Ping a peer at the Tailscale layer and report whether each reply came over a DERP relay or a direct path.";
const WHOIS_DESC: &str =
    "Identify the machine and user behind a Tailscale IP. Keys and endpoints are omitted.";
const DNS_STATUS_DESC: &str =
    "Report the MagicDNS and split-DNS configuration the local Tailscale DNS forwarder is using.";
const DNS_QUERY_DESC: &str =
    "Resolve a name through the Tailscale DNS forwarder (100.100.100.100).";

const CONNECT_TIMEOUT_SECS: u64 = 15;
const LOGIN_TIMEOUT_SECS: u64 = 20;

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<EmptyInput, StatusOutput>(STATUS_ID, STATUS_DESC),
        spec::<EmptyInput, ConfigurationOutput>(CONFIGURATION_ID, CONFIGURATION_DESC),
        spec::<EmptyInput, ConnectOutput>(CONNECT_ID, CONNECT_DESC),
        spec::<EmptyInput, DisconnectOutput>(DISCONNECT_ID, DISCONNECT_DESC),
        spec::<EmptyInput, ConnectOutput>(LOGIN_ID, LOGIN_DESC),
        spec::<EmptyInput, DisconnectOutput>(LOGOUT_ID, LOGOUT_DESC),
        spec::<VersionInput, VersionOutput>(VERSION_ID, VERSION_DESC),
        spec::<IpInput, IpOutput>(IP_ID, IP_DESC),
        spec::<EmptyInput, NetcheckOutput>(NETCHECK_ID, NETCHECK_DESC),
        spec::<PingInput, PingOutput>(PING_ID, PING_DESC),
        spec::<WhoisInput, WhoisOutput>(WHOIS_ID, WHOIS_DESC),
        spec::<EmptyInput, DnsStatusOutput>(DNS_STATUS_ID, DNS_STATUS_DESC),
        spec::<DnsQueryInput, DnsQueryOutput>(DNS_QUERY_ID, DNS_QUERY_DESC),
    ]
}

pub fn register(iii: &IIIClient, config: &SharedConfig) {
    register_fn!(iii, config, STATUS_ID, STATUS_DESC, EmptyInput, status);
    register_fn!(
        iii,
        config,
        CONFIGURATION_ID,
        CONFIGURATION_DESC,
        EmptyInput,
        configuration
    );
    register_fn!(iii, config, CONNECT_ID, CONNECT_DESC, EmptyInput, connect);
    register_fn!(
        iii,
        config,
        DISCONNECT_ID,
        DISCONNECT_DESC,
        EmptyInput,
        disconnect
    );
    register_fn!(iii, config, LOGIN_ID, LOGIN_DESC, EmptyInput, login);
    register_fn!(iii, config, LOGOUT_ID, LOGOUT_DESC, EmptyInput, logout);
    register_fn!(iii, config, VERSION_ID, VERSION_DESC, VersionInput, version);
    register_fn!(iii, config, IP_ID, IP_DESC, IpInput, ip);
    register_fn!(
        iii,
        config,
        NETCHECK_ID,
        NETCHECK_DESC,
        EmptyInput,
        netcheck
    );
    register_fn!(iii, config, PING_ID, PING_DESC, PingInput, ping);
    register_fn!(iii, config, WHOIS_ID, WHOIS_DESC, WhoisInput, whois);
    register_fn!(
        iii,
        config,
        DNS_STATUS_ID,
        DNS_STATUS_DESC,
        EmptyInput,
        dns_status
    );
    register_fn!(
        iii,
        config,
        DNS_QUERY_ID,
        DNS_QUERY_DESC,
        DnsQueryInput,
        dns_query
    );
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
    /// MagicDNS suffix of the tailnet.
    pub magic_dns_suffix: Option<String>,
    /// Name of the tailnet this node belongs to.
    pub tailnet: Option<String>,
    /// Tailscale IPv4 and IPv6 addresses of this node.
    pub tailscale_ips: Vec<String>,
    /// Health notices reported by the client.
    pub health: Vec<String>,
    /// Whether the tailnet policy allows this node to use Funnel.
    pub funnel_allowed: bool,
    /// Number of peers visible on the tailnet.
    pub peer_count: usize,
    /// Number of peers currently online.
    pub online_peer_count: usize,
    /// Tailscale Funnel ingress relay nodes in the peer list; infrastructure, excluded from the peer counts.
    pub ingress_node_count: usize,
    /// Exit node this node currently routes through, if any.
    pub exit_node: Option<String>,
    /// Active Serve and Funnel routes on this node.
    pub routes: Vec<Route>,
    /// Error text when the client could not be queried.
    pub error: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ConfigurationOutput {
    /// Local Console URL the Console share routes proxy to.
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

#[derive(Debug, Serialize, JsonSchema)]
pub struct ConnectOutput {
    /// True when the node is connected to the tailnet after the call.
    pub connected: bool,
    /// Backend state reported by the client after the call.
    pub backend_state: Option<String>,
    /// Tailscale sign-in page when the node still needs a login; open it, then call connect again.
    pub authorization_url: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DisconnectOutput {
    /// False once the node has left the tailnet.
    pub connected: bool,
    /// Backend state reported by the client after the call, normally `Stopped` or `NeedsLogin`.
    pub backend_state: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct VersionInput {
    /// Also fetch the latest upstream release for the current track.
    #[serde(default)]
    pub check_upstream: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct VersionOutput {
    /// Installed client version, e.g. `1.98.8`.
    pub version: String,
    /// Full version string with commit hashes.
    pub long: Option<String>,
    /// Client variant, e.g. `macsys`.
    pub os_variant: Option<String>,
    /// Latest upstream release when `check_upstream` was set.
    pub upstream: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct IpInput {
    /// Peer hostname or Tailscale IP; omitted means this node.
    pub peer: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct IpOutput {
    /// Peer the addresses belong to, or `self`.
    pub peer: String,
    /// Tailscale IPv4 and IPv6 addresses.
    pub addresses: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct NetcheckOutput {
    /// Whether UDP traffic reaches the internet.
    pub udp: bool,
    /// Whether IPv4 is usable.
    pub ipv4: bool,
    /// Whether IPv6 is usable.
    pub ipv6: bool,
    /// True when the NAT maps to different ports per destination (hard NAT).
    pub mapping_varies_by_dest_ip: Option<bool>,
    /// Port-mapping protocols the router offers.
    pub upnp: Option<bool>,
    pub pmp: Option<bool>,
    pub pcp: Option<bool>,
    /// DERP relay region the client prefers.
    pub preferred_derp: Option<u64>,
    /// Round-trip latency to each DERP region in milliseconds.
    pub region_latency_ms: Vec<RegionLatency>,
    /// Public IPv4 address seen by the relays.
    pub global_v4: Option<String>,
    /// Public IPv6 address seen by the relays.
    pub global_v6: Option<String>,
    /// Captive portal detected on the network.
    pub captive_portal: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RegionLatency {
    /// DERP region id.
    pub region: u64,
    /// Round-trip latency in milliseconds.
    pub latency_ms: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PingInput {
    /// Peer hostname or Tailscale IP.
    pub target: String,
    /// Number of pings to send; defaults to 5.
    #[serde(default = "default_ping_count")]
    pub count: u8,
    /// Per-ping timeout in milliseconds; defaults to 5000.
    #[serde(default = "default_ping_timeout")]
    pub timeout_ms: u64,
}

fn default_ping_count() -> u8 {
    5
}

fn default_ping_timeout() -> u64 {
    5_000
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PingOutput {
    /// Target as given.
    pub target: String,
    /// One entry per reply, in order.
    pub replies: Vec<PingReply>,
    /// True when at least one reply arrived over a direct path.
    pub direct: bool,
    /// Raw CLI output.
    pub raw: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PingReply {
    /// `direct` or `derp`.
    pub via: String,
    /// Round-trip time in milliseconds.
    pub latency_ms: Option<f64>,
    /// The CLI line for this reply.
    pub line: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WhoisInput {
    /// Tailscale IPv4 or IPv6 address, optionally with `:port`.
    pub ip: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WhoisOutput {
    /// MagicDNS name of the node.
    pub node_name: Option<String>,
    /// Stable node id.
    pub node_id: Option<String>,
    /// Tailscale addresses of the node.
    pub addresses: Vec<String>,
    /// Operating system reported by the node.
    pub os: Option<String>,
    /// ACL tags on the node.
    pub tags: Vec<String>,
    /// Login name of the user who owns the node.
    pub user_login: Option<String>,
    /// Display name of the user who owns the node.
    pub user_display_name: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DnsStatusOutput {
    /// Whether MagicDNS is enabled for this node.
    pub magic_dns: bool,
    /// MagicDNS suffix of the tailnet.
    pub magic_dns_suffix: Option<String>,
    /// Upstream resolvers Tailscale forwards to.
    pub resolvers: Vec<String>,
    /// Search domains pushed by the tailnet.
    pub search_domains: Vec<String>,
    /// Split-DNS routes: domain suffix to resolvers.
    pub split_dns_routes: Vec<SplitDnsRoute>,
    /// Domains this node can obtain HTTPS certificates for.
    pub cert_domains: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SplitDnsRoute {
    /// Domain suffix routed to the resolvers.
    pub domain: String,
    /// Resolvers used for that suffix.
    pub resolvers: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DnsQueryInput {
    /// Name to resolve.
    pub name: String,
    /// Record type such as A, AAAA, CNAME, TXT; defaults to A.
    #[serde(default = "default_record_type")]
    pub record_type: String,
}

fn default_record_type() -> String {
    "A".to_string()
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DnsQueryOutput {
    /// Name that was resolved.
    pub name: String,
    /// Record type that was queried.
    pub record_type: String,
    /// Answer records as the forwarder returned them.
    pub answers: Value,
}

pub(crate) fn dns_name(status: &Value) -> Option<String> {
    string_at(status, "/Self/DNSName")
        .map(|value| value.trim_end_matches('.').to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn funnel_allowed(status: &Value) -> bool {
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

fn is_running(status: &Value) -> bool {
    string_at(status, "/BackendState").as_deref() == Some("Running")
        && bool_at(status, "/Self/Online")
}

pub(crate) async fn status_json(config: &WorkerConfig) -> Result<Value, String> {
    run_json(config, &["status", "--json"]).await
}

async fn status(config: &WorkerConfig, _: EmptyInput) -> Result<StatusOutput, String> {
    let status_json = match status_json(config).await {
        Ok(value) => value,
        Err(error) => {
            return Ok(StatusOutput {
                installed: !error.contains("could not start"),
                version: None,
                backend_state: None,
                online: false,
                hostname: None,
                dns_name: None,
                magic_dns_suffix: None,
                tailnet: None,
                tailscale_ips: vec![],
                health: vec![],
                funnel_allowed: false,
                peer_count: 0,
                online_peer_count: 0,
                ingress_node_count: 0,
                exit_node: None,
                routes: vec![],
                error: Some(error),
            })
        }
    };
    let routes = serve_status(config)
        .await
        .map(|value| normalize_routes(&value))
        .unwrap_or_default();
    let peers: Vec<&Value> = status_json
        .get("Peer")
        .and_then(Value::as_object)
        .map(|map| map.values().collect())
        .unwrap_or_default();
    let exit_node = peers
        .iter()
        .find(|peer| bool_at(peer, "/ExitNode"))
        .and_then(|peer| string_at(peer, "/DNSName"))
        .map(|name| name.trim_end_matches('.').to_string());
    Ok(StatusOutput {
        installed: true,
        version: string_at(&status_json, "/Version"),
        backend_state: string_at(&status_json, "/BackendState"),
        online: is_running(&status_json),
        hostname: string_at(&status_json, "/Self/HostName"),
        dns_name: dns_name(&status_json),
        magic_dns_suffix: string_at(&status_json, "/MagicDNSSuffix"),
        tailnet: string_at(&status_json, "/CurrentTailnet/Name"),
        tailscale_ips: strings_at(&status_json, "/TailscaleIPs"),
        health: strings_at(&status_json, "/Health"),
        funnel_allowed: funnel_allowed(&status_json),
        peer_count: peers
            .iter()
            .filter(|peer| !super::peers::is_ingress(peer))
            .count(),
        online_peer_count: peers
            .iter()
            .filter(|peer| !super::peers::is_ingress(peer) && bool_at(peer, "/Online"))
            .count(),
        ingress_node_count: peers
            .iter()
            .filter(|peer| super::peers::is_ingress(peer))
            .count(),
        exit_node,
        routes,
        error: None,
    })
}

async fn configuration(
    config: &WorkerConfig,
    _: EmptyInput,
) -> Result<ConfigurationOutput, String> {
    let serve_config = serve_status(config)
        .await
        .unwrap_or_else(|_| Value::Object(Default::default()));
    Ok(ConfigurationOutput {
        console_url: config.console_url.clone(),
        default_https_port: config.default_https_port,
        allow_funnel: config.allow_funnel,
        command_timeout_ms: config.command_timeout_ms,
        routes: normalize_routes(&serve_config),
        serve_config,
    })
}

async fn connect_output(
    config: &WorkerConfig,
    authorization_url: Option<String>,
) -> Result<ConnectOutput, String> {
    let state = status_json(config).await?;
    Ok(ConnectOutput {
        connected: is_running(&state),
        backend_state: string_at(&state, "/BackendState"),
        authorization_url,
    })
}

async fn connect(config: &WorkerConfig, _: EmptyInput) -> Result<ConnectOutput, String> {
    let prefs = parse_prefs(&run_json(config, &["debug", "prefs"]).await?);
    let args = up_args(&prefs);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let authorization_url = run_with_login_watch(config, &refs, CONNECT_TIMEOUT_SECS).await?;
    connect_output(config, authorization_url).await
}

async fn disconnect(config: &WorkerConfig, _: EmptyInput) -> Result<DisconnectOutput, String> {
    run(config, &["down"]).await?;
    disconnect_output(config).await
}

async fn disconnect_output(config: &WorkerConfig) -> Result<DisconnectOutput, String> {
    let state = status_json(config).await?;
    Ok(DisconnectOutput {
        connected: is_running(&state),
        backend_state: string_at(&state, "/BackendState"),
    })
}

async fn login(config: &WorkerConfig, _: EmptyInput) -> Result<ConnectOutput, String> {
    let authorization_url = run_with_login_watch(config, &["login"], LOGIN_TIMEOUT_SECS).await?;
    connect_output(config, authorization_url).await
}

async fn logout(config: &WorkerConfig, _: EmptyInput) -> Result<DisconnectOutput, String> {
    run(config, &["logout"]).await?;
    disconnect_output(config).await
}

async fn run_with_login_watch(
    config: &WorkerConfig,
    args: &[&str],
    timeout_secs: u64,
) -> Result<Option<String>, String> {
    let mut child = Command::new(&config.tailscale_binary)
        .args(args)
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
    let exit = timeout(Duration::from_secs(timeout_secs), child.wait()).await;
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
            Err(if text.is_empty() {
                format!("tailscale {} exited with {status}", args[0])
            } else {
                text
            })
        }
        Ok(Err(error)) => Err(format!("Tailscale CLI failed: {error}")),
        Err(_) if authorization_url.is_none() => Err(format!(
            "tailscale {} did not finish within {timeout_secs}s",
            args[0]
        )),
        _ => Ok(authorization_url),
    }
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

async fn version(config: &WorkerConfig, input: VersionInput) -> Result<VersionOutput, String> {
    let local = run_json(config, &["version", "--json"]).await?;
    let upstream = if input.check_upstream {
        run_text(config, &["version", "--upstream"])
            .await
            .ok()
            .and_then(|text| parse_upstream(&text))
    } else {
        None
    };
    Ok(VersionOutput {
        version: string_at(&local, "/short")
            .or_else(|| string_at(&local, "/majorMinorPatch"))
            .unwrap_or_default(),
        long: string_at(&local, "/long"),
        os_variant: string_at(&local, "/osVariant"),
        upstream,
    })
}

pub fn parse_upstream(text: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.trim().eq_ignore_ascii_case("upstream"))
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn ip(config: &WorkerConfig, input: IpInput) -> Result<IpOutput, String> {
    let peer = input
        .peer
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());
    let text = match &peer {
        Some(target) => run_text(config, &["ip", target]).await?,
        None => run_text(config, &["ip"]).await?,
    };
    Ok(IpOutput {
        peer: peer.unwrap_or_else(|| "self".to_string()),
        addresses: text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
    })
}

async fn netcheck(config: &WorkerConfig, _: EmptyInput) -> Result<NetcheckOutput, String> {
    let report = run_json(config, &["netcheck", "--format", "json"]).await?;
    Ok(parse_netcheck(&report))
}

pub fn parse_netcheck(report: &Value) -> NetcheckOutput {
    let opt_bool = |key: &str| report.get(key).and_then(Value::as_bool);
    let mut region_latency_ms: Vec<RegionLatency> = report
        .get("RegionLatency")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|map| map.iter())
        .filter_map(|(region, nanos)| {
            Some(RegionLatency {
                region: region.parse().ok()?,
                latency_ms: nanos.as_f64()? / 1_000_000.0,
            })
        })
        .collect();
    region_latency_ms.sort_by(|a, b| a.latency_ms.total_cmp(&b.latency_ms));
    NetcheckOutput {
        udp: bool_at(report, "/UDP"),
        ipv4: bool_at(report, "/IPv4"),
        ipv6: bool_at(report, "/IPv6"),
        mapping_varies_by_dest_ip: opt_bool("MappingVariesByDestIP"),
        upnp: opt_bool("UPnP"),
        pmp: opt_bool("PMP"),
        pcp: opt_bool("PCP"),
        preferred_derp: report.get("PreferredDERP").and_then(Value::as_u64),
        region_latency_ms,
        global_v4: string_at(report, "/GlobalV4").filter(|v| !v.is_empty()),
        global_v6: string_at(report, "/GlobalV6").filter(|v| !v.is_empty()),
        captive_portal: opt_bool("CaptivePortal"),
    }
}

async fn ping(config: &WorkerConfig, input: PingInput) -> Result<PingOutput, String> {
    let target = input.target.trim().to_string();
    if target.is_empty() || target.starts_with('-') {
        return Err("target must be a peer hostname or Tailscale IP".to_string());
    }
    let count = input.count.clamp(1, 20).to_string();
    let timeout_arg = format!("{}ms", input.timeout_ms.clamp(100, 60_000));
    let raw = run_text(
        config,
        &[
            "ping",
            "--c",
            &count,
            "--timeout",
            &timeout_arg,
            "--until-direct=false",
            &target,
        ],
    )
    .await?;
    let replies = parse_ping(&raw);
    Ok(PingOutput {
        target,
        direct: replies.iter().any(|reply| reply.via == "direct"),
        replies,
        raw,
    })
}

pub fn parse_ping(raw: &str) -> Vec<PingReply> {
    raw.lines()
        .filter(|line| line.starts_with("pong from"))
        .map(|line| {
            let via = if line.contains("via DERP") {
                "derp"
            } else {
                "direct"
            };
            let latency_ms = line
                .rsplit_once(" in ")
                .and_then(|(_, tail)| tail.trim().trim_end_matches("ms").parse::<f64>().ok());
            PingReply {
                via: via.to_string(),
                latency_ms,
                line: line.to_string(),
            }
        })
        .collect()
}

async fn whois(config: &WorkerConfig, input: WhoisInput) -> Result<WhoisOutput, String> {
    let ip = input.ip.trim().to_string();
    if ip.is_empty() || ip.starts_with('-') {
        return Err("ip must be a Tailscale IPv4 or IPv6 address".to_string());
    }
    let value = run_json(config, &["whois", "--json", &ip]).await?;
    Ok(WhoisOutput {
        node_name: string_at(&value, "/Node/Name").map(|n| n.trim_end_matches('.').to_string()),
        node_id: string_at(&value, "/Node/StableID"),
        addresses: strings_at(&value, "/Node/Addresses"),
        os: string_at(&value, "/Node/Hostinfo/OS"),
        tags: strings_at(&value, "/Node/Tags"),
        user_login: string_at(&value, "/UserProfile/LoginName"),
        user_display_name: string_at(&value, "/UserProfile/DisplayName"),
    })
}

async fn dns_status(config: &WorkerConfig, _: EmptyInput) -> Result<DnsStatusOutput, String> {
    let value = run_json(config, &["dns", "status", "--json"]).await?;
    Ok(parse_dns_status(&value))
}

pub fn parse_dns_status(value: &Value) -> DnsStatusOutput {
    let resolvers = value
        .pointer("/TailscaleDNS/Resolvers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| {
            r.as_str()
                .map(ToOwned::to_owned)
                .or_else(|| string_at(r, "/Addr"))
        })
        .collect();
    let split_dns_routes = value
        .get("SplitDNSRoutes")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|map| map.iter())
        .map(|(domain, resolvers)| SplitDnsRoute {
            domain: domain.clone(),
            resolvers: resolvers
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| {
                    r.as_str()
                        .map(ToOwned::to_owned)
                        .or_else(|| string_at(r, "/Addr"))
                })
                .collect(),
        })
        .collect();
    DnsStatusOutput {
        magic_dns: bool_at(value, "/TailscaleDNS/MagicDNSEnabled")
            || bool_at(value, "/CurrentTailnet/MagicDNSEnabled"),
        magic_dns_suffix: string_at(value, "/CurrentTailnet/MagicDNSSuffix"),
        resolvers,
        search_domains: strings_at(value, "/SearchDomains"),
        split_dns_routes,
        cert_domains: strings_at(value, "/CertDomains"),
    }
}

async fn dns_query(config: &WorkerConfig, input: DnsQueryInput) -> Result<DnsQueryOutput, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() || name.starts_with('-') {
        return Err("name must be a DNS name".to_string());
    }
    let record_type = input.record_type.trim().to_ascii_uppercase();
    if record_type.is_empty() || !record_type.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("record_type must be a DNS record type such as A or AAAA".to_string());
    }
    let answers = run_json(config, &["dns", "query", "--json", &name, &record_type]).await?;
    Ok(DnsQueryOutput {
        name,
        record_type,
        answers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn detects_funnel_authorization_in_capabilities_and_cap_map() {
        let capability =
            serde_json::json!({"Self": {"Capabilities": ["https://tailscale.com/cap/funnel"]}});
        let cap_map = serde_json::json!({"Self": {"CapMap": {"https://tailscale.com/cap/funnel-ports": [443]}}});
        let missing = serde_json::json!({"Self": {"Capabilities": ["https"]}});
        assert!(funnel_allowed(&capability));
        assert!(funnel_allowed(&cap_map));
        assert!(!funnel_allowed(&missing));
    }

    #[test]
    fn ping_lines_classify_relay_and_direct() {
        let raw = "pong from node (100.64.0.2) via DERP(nyc) in 42ms\npong from node (100.64.0.2) via 192.0.2.1:41641 in 3.5ms\n";
        let replies = parse_ping(raw);
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[0].via, "derp");
        assert_eq!(replies[0].latency_ms, Some(42.0));
        assert_eq!(replies[1].via, "direct");
        assert_eq!(replies[1].latency_ms, Some(3.5));
    }

    #[test]
    fn netcheck_report_is_summarised() {
        let report = serde_json::json!({
            "UDP": true, "IPv4": true, "IPv6": false, "PreferredDERP": 10,
            "RegionLatency": {"10": 12_000_000, "2": 80_500_000}, "GlobalV4": "203.0.113.9", "GlobalV6": "",
            "MappingVariesByDestIP": false, "UPnP": true, "PMP": false, "PCP": false, "CaptivePortal": false
        });
        let out = parse_netcheck(&report);
        assert!(out.udp && out.ipv4 && !out.ipv6);
        assert_eq!(out.preferred_derp, Some(10));
        assert_eq!(out.region_latency_ms[0].region, 10);
        assert_eq!(out.region_latency_ms[0].latency_ms, 12.0);
        assert_eq!(out.global_v4.as_deref(), Some("203.0.113.9"));
        assert!(out.global_v6.is_none());
    }

    #[test]
    fn dns_status_reads_split_routes_and_suffix() {
        let value = serde_json::json!({
            "TailscaleDNS": {"MagicDNSEnabled": true, "Resolvers": [{"Addr": "1.1.1.1"}]},
            "CurrentTailnet": {"MagicDNSSuffix": "tail1.ts.net"},
            "SplitDNSRoutes": {"corp.example": [{"Addr": "10.0.0.53"}]},
            "SearchDomains": ["tail1.ts.net"],
            "CertDomains": ["node.tail1.ts.net"]
        });
        let out = parse_dns_status(&value);
        assert!(out.magic_dns);
        assert_eq!(out.magic_dns_suffix.as_deref(), Some("tail1.ts.net"));
        assert_eq!(out.resolvers, vec!["1.1.1.1"]);
        assert_eq!(out.split_dns_routes[0].domain, "corp.example");
        assert_eq!(out.split_dns_routes[0].resolvers, vec!["10.0.0.53"]);
    }

    #[test]
    fn upstream_version_is_parsed() {
        assert_eq!(
            parse_upstream("Tailscale version 1.98.8\nupstream: 1.99.0").as_deref(),
            Some("1.99.0")
        );
        assert!(parse_upstream("1.98.8").is_none());
    }
}
