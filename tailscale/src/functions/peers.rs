use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::node::status_json;
use super::{
    bool_at, register_fn, run, run_json, run_text, spec, string_at, strings_at, u64_at, EmptyInput,
    FunctionSpec,
};
use crate::config::{SharedConfig, WorkerConfig};

pub const PEERS_LIST_ID: &str = "tailscale::peers::list";
pub const EXIT_NODE_LIST_ID: &str = "tailscale::exit-node::list";
pub const EXIT_NODE_SUGGEST_ID: &str = "tailscale::exit-node::suggest";
pub const EXIT_NODE_SET_ID: &str = "tailscale::exit-node::set";
pub const PREFS_GET_ID: &str = "tailscale::prefs::get";
pub const PREFS_SET_ID: &str = "tailscale::prefs::set";

const PEERS_LIST_DESC: &str = "List the devices on the tailnet as this node sees them: names, Tailscale IPs, OS, online state, tags, exit-node offers, and traffic counters. Keys are omitted.";
const EXIT_NODE_LIST_DESC: &str = "List the peers that offer to be an exit node for internet traffic, and which one this node uses.";
const EXIT_NODE_SUGGEST_DESC: &str =
    "Ask Tailscale for the best available exit node for this node.";
const EXIT_NODE_SET_DESC: &str = "Route this node's internet traffic through an exit node (by name or IP, or `auto:any`), or clear it with an empty value.";
const PREFS_GET_DESC: &str = "Read this node's Tailscale preferences: routes, DNS, exit node, SSH, shields-up, hostname, auto-update. Keys and login secrets are omitted.";
const PREFS_SET_DESC: &str = "Change only the given Tailscale preferences (`tailscale set`): accept routes or DNS, advertise routes or exit node, hostname, shields-up, SSH server, auto-update, LAN access with an exit node.";

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<PeersInput, PeersOutput>(PEERS_LIST_ID, PEERS_LIST_DESC),
        spec::<EmptyInput, ExitNodesOutput>(EXIT_NODE_LIST_ID, EXIT_NODE_LIST_DESC),
        spec::<EmptyInput, ExitNodeSuggestion>(EXIT_NODE_SUGGEST_ID, EXIT_NODE_SUGGEST_DESC),
        spec::<ExitNodeSetInput, Prefs>(EXIT_NODE_SET_ID, EXIT_NODE_SET_DESC),
        spec::<EmptyInput, Prefs>(PREFS_GET_ID, PREFS_GET_DESC),
        spec::<PrefsSetInput, Prefs>(PREFS_SET_ID, PREFS_SET_DESC),
    ]
}

pub fn register(iii: &IIIClient, config: &SharedConfig) {
    register_fn!(
        iii,
        config,
        PEERS_LIST_ID,
        PEERS_LIST_DESC,
        PeersInput,
        peers_list
    );
    register_fn!(
        iii,
        config,
        EXIT_NODE_LIST_ID,
        EXIT_NODE_LIST_DESC,
        EmptyInput,
        exit_node_list
    );
    register_fn!(
        iii,
        config,
        EXIT_NODE_SUGGEST_ID,
        EXIT_NODE_SUGGEST_DESC,
        EmptyInput,
        exit_node_suggest
    );
    register_fn!(
        iii,
        config,
        EXIT_NODE_SET_ID,
        EXIT_NODE_SET_DESC,
        ExitNodeSetInput,
        exit_node_set
    );
    register_fn!(
        iii,
        config,
        PREFS_GET_ID,
        PREFS_GET_DESC,
        EmptyInput,
        prefs_get
    );
    register_fn!(
        iii,
        config,
        PREFS_SET_ID,
        PREFS_SET_DESC,
        PrefsSetInput,
        prefs_set
    );
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct PeersInput {
    /// Only peers that are online right now.
    #[serde(default)]
    pub online_only: bool,
    /// Include Tailscale's Funnel ingress relay nodes (`funnel-ingress-node`, tag `tag:ingress`), which are infrastructure rather than devices.
    #[serde(default)]
    pub include_ingress: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Peer {
    /// Stable node id.
    pub id: String,
    /// Machine name.
    pub hostname: String,
    /// MagicDNS name without the trailing dot.
    pub dns_name: String,
    /// Operating system reported by the peer.
    pub os: Option<String>,
    /// Tailscale IPv4 and IPv6 addresses.
    pub tailscale_ips: Vec<String>,
    /// Whether the peer is online.
    pub online: bool,
    /// Whether this node currently exchanges traffic with the peer.
    pub active: bool,
    /// Whether this node routes internet traffic through the peer.
    pub exit_node: bool,
    /// Whether the peer offers to be an exit node.
    pub exit_node_option: bool,
    /// ACL tags on the peer.
    pub tags: Vec<String>,
    /// When the peer was last seen, RFC 3339.
    pub last_seen: Option<String>,
    /// DERP relay the connection currently uses, empty when direct.
    pub relay: Option<String>,
    /// Bytes received from the peer.
    pub rx_bytes: u64,
    /// Bytes sent to the peer.
    pub tx_bytes: u64,
    /// Whether the peer accepts Taildrop files from this node.
    pub taildrop_target: bool,
    /// True for Tailscale's Funnel ingress relay nodes, which are infrastructure rather than devices.
    pub ingress: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PeersOutput {
    /// Peers in name order.
    pub peers: Vec<Peer>,
    /// Funnel ingress relay nodes left out because `include_ingress` was false.
    pub hidden_ingress_count: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ExitNodesOutput {
    /// Peers offering to be an exit node.
    pub exit_nodes: Vec<Peer>,
    /// MagicDNS name of the exit node in use, if any.
    pub current: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ExitNodeSuggestion {
    /// Suggested exit node name, or null when Tailscale has none to offer.
    pub suggestion: Option<String>,
    /// The CLI's own wording.
    pub message: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ExitNodeSetInput {
    /// Exit node by MagicDNS name, hostname, Tailscale IP, or `auto:any`; empty or omitted clears the exit node.
    pub exit_node: Option<String>,
    /// Allow direct access to the local LAN while the exit node is in use.
    pub allow_lan_access: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Prefs {
    /// Control server URL.
    pub control_url: Option<String>,
    /// Accept subnet routes advertised by other nodes.
    pub accept_routes: bool,
    /// Accept DNS configuration from the tailnet.
    pub accept_dns: bool,
    /// Exit node in use, by id.
    pub exit_node_id: Option<String>,
    /// Exit node in use, by IP.
    pub exit_node_ip: Option<String>,
    /// LAN access allowed while using an exit node.
    pub exit_node_allow_lan_access: bool,
    /// Tailscale SSH server enabled on this node.
    pub ssh: bool,
    /// Web client exposed on port 5252.
    pub webclient: bool,
    /// Whether the node wants to be connected.
    pub want_running: bool,
    /// Whether the node is logged out.
    pub logged_out: bool,
    /// Incoming connections blocked.
    pub shields_up: bool,
    /// Hostname override, empty when the OS name is used.
    pub hostname: Option<String>,
    /// Subnet routes this node advertises.
    pub advertise_routes: Vec<String>,
    /// Whether this node advertises itself as an exit node.
    pub advertise_exit_node: bool,
    /// ACL tags requested by this node.
    pub advertise_tags: Vec<String>,
    /// Automatic update checks enabled.
    pub auto_update_check: bool,
    /// Automatic updates applied.
    pub auto_update_apply: bool,
    /// Advertised as an app connector.
    pub app_connector: bool,
    /// Device posture reporting enabled.
    pub posture_checking: bool,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct PrefsSetInput {
    /// Accept subnet routes advertised by other nodes.
    pub accept_routes: Option<bool>,
    /// Accept DNS configuration from the tailnet.
    pub accept_dns: Option<bool>,
    /// Subnet routes to advertise, CIDR notation; an empty list stops advertising.
    pub advertise_routes: Option<Vec<String>>,
    /// Offer this node as an exit node.
    pub advertise_exit_node: Option<bool>,
    /// Offer this node as an app connector.
    pub advertise_connector: Option<bool>,
    /// Hostname to use instead of the OS name; empty restores the OS name.
    pub hostname: Option<String>,
    /// Block incoming connections.
    pub shields_up: Option<bool>,
    /// Run the Tailscale SSH server.
    pub ssh: Option<bool>,
    /// Apply updates automatically.
    pub auto_update: Option<bool>,
    /// Notify about available updates.
    pub update_check: Option<bool>,
    /// Expose the web client on port 5252.
    pub webclient: Option<bool>,
    /// Allow direct LAN access while using an exit node.
    pub exit_node_allow_lan_access: Option<bool>,
    /// Report device posture to the management plane.
    pub report_posture: Option<bool>,
}

pub fn parse_peer(value: &Value) -> Option<Peer> {
    let hostname = string_at(value, "/HostName")?;
    Some(Peer {
        id: string_at(value, "/ID").unwrap_or_default(),
        hostname,
        dns_name: string_at(value, "/DNSName")
            .map(|n| n.trim_end_matches('.').to_string())
            .unwrap_or_default(),
        os: string_at(value, "/OS").filter(|s| !s.is_empty()),
        tailscale_ips: strings_at(value, "/TailscaleIPs"),
        online: bool_at(value, "/Online"),
        active: bool_at(value, "/Active"),
        exit_node: bool_at(value, "/ExitNode"),
        exit_node_option: bool_at(value, "/ExitNodeOption"),
        tags: strings_at(value, "/Tags"),
        last_seen: string_at(value, "/LastSeen").filter(|s| !s.starts_with("0001-")),
        relay: string_at(value, "/Relay").filter(|s| !s.is_empty()),
        rx_bytes: u64_at(value, "/RxBytes").unwrap_or(0),
        tx_bytes: u64_at(value, "/TxBytes").unwrap_or(0),
        taildrop_target: u64_at(value, "/TaildropTarget").unwrap_or(0) == 1,
        ingress: is_ingress(value),
    })
}

pub fn is_ingress(value: &Value) -> bool {
    strings_at(value, "/Tags")
        .iter()
        .any(|tag| tag == "tag:ingress")
        || string_at(value, "/HostName").as_deref() == Some("funnel-ingress-node")
}

pub fn parse_peers(status: &Value) -> Vec<Peer> {
    let mut peers: Vec<Peer> = status
        .get("Peer")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|map| map.values())
        .filter_map(parse_peer)
        .collect();
    peers.sort_by(|a, b| a.dns_name.cmp(&b.dns_name));
    peers
}

async fn peers_list(config: &WorkerConfig, input: PeersInput) -> Result<PeersOutput, String> {
    let status = status_json(config).await?;
    let all = parse_peers(&status);
    let hidden_ingress_count = if input.include_ingress {
        0
    } else {
        all.iter().filter(|peer| peer.ingress).count()
    };
    let peers = all
        .into_iter()
        .filter(|peer| input.include_ingress || !peer.ingress)
        .filter(|peer| !input.online_only || peer.online)
        .collect();
    Ok(PeersOutput {
        peers,
        hidden_ingress_count,
    })
}

async fn exit_node_list(config: &WorkerConfig, _: EmptyInput) -> Result<ExitNodesOutput, String> {
    let status = status_json(config).await?;
    let peers = parse_peers(&status);
    Ok(ExitNodesOutput {
        current: peers
            .iter()
            .find(|peer| peer.exit_node)
            .map(|peer| peer.dns_name.clone()),
        exit_nodes: peers
            .into_iter()
            .filter(|peer| peer.exit_node_option)
            .collect(),
    })
}

async fn exit_node_suggest(
    config: &WorkerConfig,
    _: EmptyInput,
) -> Result<ExitNodeSuggestion, String> {
    let message = match run_text(config, &["exit-node", "suggest"]).await {
        Ok(text) => text,
        Err(error) if error.to_lowercase().contains("no exit node") => error,
        Err(error) => return Err(error),
    };
    Ok(ExitNodeSuggestion {
        suggestion: parse_suggestion(&message),
        message,
    })
}

pub fn parse_suggestion(message: &str) -> Option<String> {
    let lower = message.to_ascii_lowercase();
    if lower.contains("no exit node") {
        return None;
    }
    message
        .split_once("Suggested exit node:")
        .map(|(_, rest)| rest.trim().trim_end_matches('.').to_string())
        .filter(|s| !s.is_empty())
}

async fn exit_node_set(config: &WorkerConfig, input: ExitNodeSetInput) -> Result<Prefs, String> {
    let exit_node = input.exit_node.unwrap_or_default();
    let exit_node = exit_node.trim();
    if exit_node.starts_with('-') || exit_node.chars().any(char::is_whitespace) {
        return Err("exit_node must be a node name, Tailscale IP, or auto:any".to_string());
    }
    let mut args = vec!["set".to_string(), format!("--exit-node={exit_node}")];
    if let Some(allow) = input.allow_lan_access {
        args.push(format!("--exit-node-allow-lan-access={allow}"));
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run(config, &refs).await?;
    prefs_get(config, EmptyInput::default()).await
}

async fn prefs_get(config: &WorkerConfig, _: EmptyInput) -> Result<Prefs, String> {
    let value = run_json(config, &["debug", "prefs"]).await?;
    Ok(parse_prefs(&value))
}

pub fn parse_prefs(value: &Value) -> Prefs {
    Prefs {
        control_url: string_at(value, "/ControlURL"),
        accept_routes: bool_at(value, "/RouteAll"),
        accept_dns: bool_at(value, "/CorpDNS"),
        exit_node_id: string_at(value, "/ExitNodeID").filter(|s| !s.is_empty()),
        exit_node_ip: string_at(value, "/ExitNodeIP").filter(|s| !s.is_empty()),
        exit_node_allow_lan_access: bool_at(value, "/ExitNodeAllowLANAccess"),
        ssh: bool_at(value, "/RunSSH"),
        webclient: bool_at(value, "/RunWebClient"),
        want_running: bool_at(value, "/WantRunning"),
        logged_out: bool_at(value, "/LoggedOut"),
        shields_up: bool_at(value, "/ShieldsUp"),
        hostname: string_at(value, "/Hostname").filter(|s| !s.is_empty()),
        advertise_routes: strings_at(value, "/AdvertiseRoutes")
            .into_iter()
            .filter(|r| !is_exit_route(r))
            .collect(),
        advertise_exit_node: strings_at(value, "/AdvertiseRoutes")
            .iter()
            .any(|r| is_exit_route(r)),
        advertise_tags: strings_at(value, "/AdvertiseTags"),
        auto_update_check: bool_at(value, "/AutoUpdate/Check"),
        auto_update_apply: bool_at(value, "/AutoUpdate/Apply"),
        app_connector: bool_at(value, "/AppConnector/Advertise"),
        posture_checking: bool_at(value, "/PostureChecking"),
    }
}

fn is_exit_route(route: &str) -> bool {
    matches!(route, "0.0.0.0/0" | "::/0")
}

pub fn prefs_set_args(input: &PrefsSetInput) -> Result<Vec<String>, String> {
    let mut args = vec!["set".to_string()];
    let mut flag = |name: &str, value: Option<bool>| {
        if let Some(value) = value {
            args.push(format!("--{name}={value}"));
        }
    };
    flag("accept-routes", input.accept_routes);
    flag("accept-dns", input.accept_dns);
    flag("advertise-exit-node", input.advertise_exit_node);
    flag("advertise-connector", input.advertise_connector);
    flag("shields-up", input.shields_up);
    flag("ssh", input.ssh);
    flag("auto-update", input.auto_update);
    flag("update-check", input.update_check);
    flag("webclient", input.webclient);
    flag(
        "exit-node-allow-lan-access",
        input.exit_node_allow_lan_access,
    );
    flag("report-posture", input.report_posture);
    if let Some(routes) = &input.advertise_routes {
        for route in routes {
            if route.trim().is_empty()
                || route.starts_with('-')
                || !route.contains('/')
                || route.chars().any(char::is_whitespace)
            {
                return Err(format!("advertise_routes entry is not a CIDR: {route}"));
            }
        }
        args.push(format!("--advertise-routes={}", routes.join(",")));
    }
    if let Some(hostname) = &input.hostname {
        let hostname = hostname.trim();
        if hostname.starts_with('-') || hostname.chars().any(char::is_whitespace) {
            return Err("hostname must not contain whitespace".to_string());
        }
        args.push(format!("--hostname={hostname}"));
    }
    if args.len() == 1 {
        return Err("no preference given; set at least one field".to_string());
    }
    Ok(args)
}

async fn prefs_set(config: &WorkerConfig, input: PrefsSetInput) -> Result<Prefs, String> {
    let args = prefs_set_args(&input)?;
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run(config, &refs).await?;
    prefs_get(config, EmptyInput::default()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peers_are_parsed_and_sorted_without_keys() {
        let status = serde_json::json!({"Peer": {
            "nodekey:b": {"ID": "n2", "HostName": "phone", "DNSName": "phone.tail.ts.net.", "OS": "iOS", "TailscaleIPs": ["100.64.0.2"], "Online": true, "Active": false, "ExitNode": false, "ExitNodeOption": false, "Tags": null, "LastSeen": "2026-08-26T10:00:00Z", "Relay": "nyc", "RxBytes": 10, "TxBytes": 20, "TaildropTarget": 1, "PublicKey": "nodekey:secret"},
            "nodekey:a": {"ID": "n1", "HostName": "gateway", "DNSName": "gateway.tail.ts.net.", "OS": "linux", "TailscaleIPs": ["100.64.0.1"], "Online": true, "Active": true, "ExitNode": true, "ExitNodeOption": true, "Tags": ["tag:exit"], "LastSeen": "0001-01-01T00:00:00Z", "Relay": "", "RxBytes": 0, "TxBytes": 0, "TaildropTarget": 0}
        }});
        let peers = parse_peers(&status);
        assert_eq!(peers.len(), 2);
        assert_eq!(peers[0].dns_name, "gateway.tail.ts.net");
        assert!(peers[0].exit_node && peers[0].exit_node_option);
        assert_eq!(peers[0].tags, vec!["tag:exit"]);
        assert!(peers[0].last_seen.is_none());
        assert!(peers[0].relay.is_none());
        assert_eq!(peers[1].relay.as_deref(), Some("nyc"));
        assert!(peers[1].taildrop_target);
        assert!(!peers[0].ingress && !peers[1].ingress);
        assert!(!serde_json::to_string(&peers)
            .unwrap()
            .contains("nodekey:secret"));
    }

    #[test]
    fn funnel_ingress_relays_are_flagged() {
        let by_tag = serde_json::json!({"HostName": "funnel-ingress-node", "DNSName": "", "Tags": ["tag:ingress"], "TailscaleIPs": ["fd7a::1"]});
        let by_name = serde_json::json!({"HostName": "funnel-ingress-node", "DNSName": "", "TailscaleIPs": []});
        let device = serde_json::json!({"HostName": "phone", "DNSName": "phone.tail.ts.net.", "Tags": ["tag:mobile"]});
        assert!(parse_peer(&by_tag).unwrap().ingress);
        assert!(parse_peer(&by_name).unwrap().ingress);
        assert!(!parse_peer(&device).unwrap().ingress);
    }

    #[test]
    fn prefs_are_read_without_the_private_key() {
        let value = serde_json::json!({
            "ControlURL": "https://controlplane.tailscale.com", "RouteAll": true, "CorpDNS": true,
            "ExitNodeID": "", "ExitNodeIP": "", "ExitNodeAllowLANAccess": false, "RunSSH": false,
            "RunWebClient": false, "WantRunning": true, "LoggedOut": false, "ShieldsUp": false,
            "Hostname": "", "AdvertiseRoutes": ["10.0.0.0/8", "0.0.0.0/0", "::/0"], "AdvertiseTags": null,
            "AutoUpdate": {"Check": true, "Apply": true}, "AppConnector": {"Advertise": false}, "PostureChecking": false,
            "Config": {"PrivateNodeKey": "privkey:deadbeef"}
        });
        let prefs = parse_prefs(&value);
        assert!(prefs.accept_routes && prefs.accept_dns && prefs.want_running);
        assert_eq!(prefs.advertise_routes, vec!["10.0.0.0/8"]);
        assert!(prefs.advertise_exit_node);
        assert!(prefs.auto_update_apply);
        assert!(!serde_json::to_string(&prefs).unwrap().contains("privkey"));
    }

    #[test]
    fn prefs_set_builds_only_the_given_flags() {
        let args = prefs_set_args(&PrefsSetInput {
            accept_routes: Some(true),
            ssh: Some(false),
            advertise_routes: Some(vec!["10.0.0.0/8".into()]),
            hostname: Some("lab".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            args,
            vec![
                "set",
                "--accept-routes=true",
                "--ssh=false",
                "--advertise-routes=10.0.0.0/8",
                "--hostname=lab"
            ]
        );
        assert!(prefs_set_args(&PrefsSetInput::default()).is_err());
        assert!(prefs_set_args(&PrefsSetInput {
            advertise_routes: Some(vec!["--reset".into()]),
            ..Default::default()
        })
        .is_err());
    }

    #[test]
    fn exit_node_suggestion_is_parsed() {
        assert_eq!(
            parse_suggestion("Suggested exit node: gateway.tail.ts.net.").as_deref(),
            Some("gateway.tail.ts.net")
        );
        assert!(parse_suggestion("No exit node suggestion is available.").is_none());
    }
}
