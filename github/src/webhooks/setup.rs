//! Webhook prerequisites as one checklist shared by the GitHub console page
//! and agents: the quick-tunnel worker, the cloudflared binary it runs, and the
//! http worker's restricted webhook listener. Nothing here installs software;
//! the page installs the quick-tunnel worker only after the operator confirms.

use super::*;

pub const QUICK_TUNNEL_WORKER: &str = "quick-tunnel";
/// quick-tunnel's default `webhooks` target is http://127.0.0.1:3112.
pub const DEFAULT_LISTENER_PORT: u16 = 3112;
/// Shown when quick-tunnel cannot report its own install pointer.
pub const CLOUDFLARED_INSTALL_URL: &str =
    "https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    /// Satisfied.
    Ok,
    /// Definitely missing; `fix` says how to satisfy it.
    Missing,
    /// Waits for an earlier check.
    Blocked,
    /// Could not be verified (older worker version, transient error).
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SetupFix {
    /// Install a worker with compose::add; the GitHub page asks first.
    InstallWorker { worker: String, command: String },
    /// Update a worker to a version that supports this check.
    UpdateWorker { worker: String, command: String },
    /// Install software outside iii; nothing is downloaded for you.
    OpenUrl { url: String },
    /// github::setup::enable-http-listener turns the restricted listener on.
    EnableHttpListener { port: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SetupCheck {
    /// quick_tunnel | cloudflared | http_listener
    pub id: String,
    pub title: String,
    pub state: CheckState,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<SetupFix>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SetupStatus {
    /// `webhooks.enabled` in the live configuration.
    pub enabled: bool,
    /// Webhook storage is open in this process (it opens at startup only).
    pub active: bool,
    /// Every prerequisite is satisfied.
    pub ready: bool,
    /// `enabled` differs from what this process started with: restart the
    /// github worker to apply it.
    pub restart_required: bool,
    pub checks: Vec<SetupCheck>,
    /// Why webhook storage failed to open at startup while enabled; restarting
    /// will not help until this is fixed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_error: Option<String>,
}

/// How `quick-tunnel::status` failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelProbe {
    /// The function does not exist and compose does not declare the worker.
    NotInstalled,
    /// Declared but not answering (restarting, stopped) or a transient error.
    NotResponding(String),
    /// Not registered, and compose could not say whether it is declared.
    Unconfirmed,
}

/// `compose::status` routed the way iii-directory and the harness route it:
/// the supervisor's daemon namespace and compose file when set, otherwise this
/// worker's own namespace with an empty payload. Returns (payload, namespace).
pub fn compose_status_route(
    namespace: Option<&str>,
    file: Option<&str>,
) -> (Value, Option<String>) {
    let mut payload = serde_json::Map::new();
    if let Some(file) = file {
        payload.insert("file".into(), json!(file));
    }
    if let Some(namespace) = namespace {
        payload.insert("namespace".into(), json!(namespace));
    }
    (Value::Object(payload), namespace.map(str::to_owned))
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetupStatusRequest {}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnableListenerRequest {
    /// Restricted listener port; defaults to quick-tunnel's target, 3112.
    #[serde(default)]
    pub port: Option<u16>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnableWebhooksRequest {
    /// Enabling requires every prerequisite; disabling never does.
    pub enabled: bool,
}

fn check(
    id: &str,
    title: &str,
    state: CheckState,
    detail: String,
    fix: Option<SetupFix>,
) -> SetupCheck {
    SetupCheck {
        id: id.into(),
        title: title.into(),
        state,
        detail,
        fix,
    }
}

/// The engine's structured "no such function" answer (never a text match).
pub fn function_missing(e: &Failure) -> bool {
    matches!(e, Failure::Engine(iii_sdk::Error::Remote { code, .. }) if code == "function_not_found")
}

/// Build the three checks from what the workers answered. Pure for tests.
pub fn evaluate(
    tunnel: &std::result::Result<Value, TunnelProbe>,
    listener: &std::result::Result<Option<Value>, String>,
) -> Vec<SetupCheck> {
    let quick = match tunnel {
        Ok(_) => check(
            "quick_tunnel",
            "quick-tunnel worker",
            CheckState::Ok,
            "Installed.".into(),
            None,
        ),
        Err(TunnelProbe::NotInstalled) => check(
            "quick_tunnel",
            "quick-tunnel worker",
            CheckState::Missing,
            "PR webhooks need the quick-tunnel worker to publish a temporary public URL.".into(),
            Some(SetupFix::InstallWorker {
                worker: QUICK_TUNNEL_WORKER.into(),
                command: "compose::add { worker: \"quick-tunnel\" }".into(),
            }),
        ),
        Err(TunnelProbe::NotResponding(e)) => check(
            "quick_tunnel",
            "quick-tunnel worker",
            CheckState::Unknown,
            format!("quick-tunnel did not answer: {e}"),
            None,
        ),
        Err(TunnelProbe::Unconfirmed) => check(
            "quick_tunnel",
            "quick-tunnel worker",
            CheckState::Unknown,
            "quick-tunnel is not registered, and compose could not confirm whether it is installed.".into(),
            Some(SetupFix::InstallWorker {
                worker: QUICK_TUNNEL_WORKER.into(),
                command: "compose::add { worker: \"quick-tunnel\" }".into(),
            }),
        ),
    };
    let cloudflared = match tunnel {
        Ok(status) => match status.pointer("/prerequisites/cloudflared") {
            Some(c) if c["found"] == true => check(
                "cloudflared",
                "cloudflared binary",
                CheckState::Ok,
                match (c["version"].as_str(), c["path"].as_str()) {
                    (Some(v), Some(p)) => format!("{v} ({p})"),
                    (_, Some(p)) => format!("Found at {p}."),
                    _ => "Found.".into(),
                },
                None,
            ),
            Some(c) => check(
                "cloudflared",
                "cloudflared binary",
                CheckState::Missing,
                c["error"]
                    .as_str()
                    .unwrap_or("cloudflared was not found.")
                    .to_owned(),
                Some(SetupFix::OpenUrl {
                    url: c["install_url"]
                        .as_str()
                        .unwrap_or(CLOUDFLARED_INSTALL_URL)
                        .to_owned(),
                }),
            ),
            None => check(
                "cloudflared",
                "cloudflared binary",
                CheckState::Unknown,
                "This quick-tunnel version does not report cloudflared; update it.".into(),
                Some(SetupFix::UpdateWorker {
                    worker: QUICK_TUNNEL_WORKER.into(),
                    command: "compose::update { worker: \"quick-tunnel\" }".into(),
                }),
            ),
        },
        Err(TunnelProbe::NotInstalled) => check(
            "cloudflared",
            "cloudflared binary",
            CheckState::Blocked,
            "Install quick-tunnel first: it is the worker that runs cloudflared.".into(),
            None,
        ),
        // quick-tunnel exists but did not answer: unverifiable, never a blocker.
        Err(TunnelProbe::NotResponding(_) | TunnelProbe::Unconfirmed) => check(
            "cloudflared",
            "cloudflared binary",
            CheckState::Unknown,
            "quick-tunnel did not answer, so cloudflared could not be checked.".into(),
            None,
        ),
    };
    let http = match listener {
        Ok(Some(value)) => match value.get("webhook_listener").filter(|l| l.is_object()) {
            Some(l) => check(
                "http_listener",
                "http webhook listener",
                CheckState::Ok,
                format!(
                    "Listening on {}:{}.",
                    l["host"].as_str().unwrap_or("127.0.0.1"),
                    l["port"].as_u64().unwrap_or(u64::from(DEFAULT_LISTENER_PORT))
                ),
                None,
            ),
            None => check(
                "http_listener",
                "http webhook listener",
                CheckState::Missing,
                "The http worker's restricted webhook listener is off; quick-tunnel forwards GitHub deliveries to it.".into(),
                Some(SetupFix::EnableHttpListener {
                    port: DEFAULT_LISTENER_PORT,
                }),
            ),
        },
        Ok(None) => check(
            "http_listener",
            "http webhook listener",
            CheckState::Unknown,
            "This http version has no webhook listener; update it to 0.22 or later.".into(),
            Some(SetupFix::UpdateWorker {
                worker: "http".into(),
                command: "compose::update { worker: \"http\" }".into(),
            }),
        ),
        Err(e) => check(
            "http_listener",
            "http webhook listener",
            CheckState::Unknown,
            format!("http configuration unavailable: {e}"),
            None,
        ),
    };
    vec![quick, cloudflared, http]
}

/// Human-readable list of definite blockers, or None when nothing is missing.
pub fn blockers(checks: &[SetupCheck]) -> Option<String> {
    let missing: Vec<String> = checks
        .iter()
        .filter(|c| matches!(c.state, CheckState::Missing | CheckState::Blocked))
        .map(|c| format!("{}: {}", c.title, c.detail))
        .collect();
    (!missing.is_empty()).then(|| {
        format!(
            "PR webhooks are not set up — {}. Open the GitHub page's webhook setup or call github::setup::webhooks-status.",
            missing.join("; ")
        )
    })
}

impl Service {
    /// Whether compose declares quick-tunnel: Some(declared) when compose
    /// answered, None when it could not be asked (never proof of absence).
    async fn quick_tunnel_declared(&self) -> Option<bool> {
        let env = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let (payload, namespace) = compose_status_route(
            env("III_COMPOSE_NAMESPACE").as_deref(),
            env("III_COMPOSE_FILE").as_deref(),
        );
        let status = self
            .invoke_target("compose::status", payload, namespace.as_deref(), None)
            .await
            .ok()?;
        let containers = status["containers"].as_array()?;
        Some(
            containers
                .iter()
                .any(|c| c["container"] == QUICK_TUNNEL_WORKER),
        )
    }

    /// The http worker's configuration id and raw value; None when http
    /// predates the webhook listener (no http::configuration-id).
    async fn http_config(&self) -> Result<Option<(String, Value)>> {
        let id = match self.invoke("http::configuration-id", json!({})).await {
            Ok(v) => v["id"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| Failure::Invalid("http returned no configuration id".into()))?,
            Err(e) if function_missing(&e) => return Ok(None),
            Err(e) => return Err(e),
        };
        let got = self
            .invoke("configuration::get", json!({"id": id, "raw": true}))
            .await?;
        Ok(Some((id, got["value"].clone())))
    }

    pub(super) async fn setup_status(&self, _req: SetupStatusRequest) -> Result<SetupStatus> {
        let live = self.cell.read().await.webhooks.clone();
        let tunnel = match self
            .invoke("quick-tunnel::status", json!({"tunnel_id": live.tunnel_id}))
            .await
        {
            Ok(status) => Ok(status),
            Err(e) if function_missing(&e) => Err(match self.quick_tunnel_declared().await {
                Some(true) => TunnelProbe::NotResponding(
                    "declared in compose but not answering (restarting or stopped)".into(),
                ),
                Some(false) => TunnelProbe::NotInstalled,
                None => TunnelProbe::Unconfirmed,
            }),
            Err(e) => Err(TunnelProbe::NotResponding(e.to_string())),
        };
        let listener = self
            .http_config()
            .await
            .map(|c| c.map(|(_, value)| value))
            .map_err(|e| e.to_string());
        let checks = evaluate(&tunnel, &listener);
        let active = self.store.is_some();
        Ok(SetupStatus {
            enabled: live.enabled,
            active,
            ready: checks.iter().all(|c| c.state == CheckState::Ok),
            restart_required: live.enabled != active,
            checks,
            storage_error: (live.enabled && !active)
                .then(|| self.storage_error.clone())
                .flatten(),
        })
    }

    /// Refuse a watch only on definite blockers; unverifiable checks pass.
    pub(super) async fn require_setup(&self) -> Result<()> {
        let status = self.setup_status(SetupStatusRequest {}).await?;
        match blockers(&status.checks) {
            Some(message) => Err(Failure::Invalid(message)),
            None => Ok(()),
        }
    }

    pub(super) async fn enable_http_listener(
        &self,
        req: EnableListenerRequest,
    ) -> Result<SetupStatus> {
        let Some((id, mut value)) = self.http_config().await? else {
            return Err(Failure::Invalid(
                "this http version has no webhook listener; update http to 0.22 or later".into(),
            ));
        };
        if !value.is_object() {
            value = json!({});
        }
        let existing = value
            .get("webhook_listener")
            .filter(|l| l.is_object())
            .cloned();
        // An operator-configured listener is kept unless a port is requested.
        if existing.is_some() && req.port.is_none() {
            return self.setup_status(SetupStatusRequest {}).await;
        }
        let host = existing
            .as_ref()
            .and_then(|l| l["host"].as_str())
            .unwrap_or("127.0.0.1")
            .to_owned();
        value["webhook_listener"] = json!({
            "host": host,
            "port": req.port.unwrap_or(DEFAULT_LISTENER_PORT),
        });
        self.invoke("configuration::set", json!({"id": id, "value": value}))
            .await?;
        self.setup_status(SetupStatusRequest {}).await
    }

    pub(super) async fn enable_webhooks(&self, req: EnableWebhooksRequest) -> Result<SetupStatus> {
        if req.enabled {
            let status = self.setup_status(SetupStatusRequest {}).await?;
            if !status.ready {
                return Err(Failure::Invalid(blockers(&status.checks).unwrap_or_else(
                    || "PR webhooks cannot be enabled until every prerequisite is verified".into(),
                )));
            }
        }
        let id = crate::configuration::config_id();
        let got = self
            .invoke("configuration::get", json!({"id": id, "raw": true}))
            .await?;
        let mut value = got["value"].clone();
        if !value.is_object() {
            value = json!({});
        }
        if !value["webhooks"].is_object() {
            value["webhooks"] = json!({});
        }
        value["webhooks"]["enabled"] = json!(req.enabled);
        self.invoke("configuration::set", json!({"id": id, "value": value}))
            .await?;
        // The live cell refreshes from the configuration trigger; report the
        // value just written so the caller knows a restart is due.
        let mut status = self.setup_status(SetupStatusRequest {}).await?;
        status.enabled = req.enabled;
        status.restart_required = req.enabled != status.active;
        Ok(status)
    }
}
