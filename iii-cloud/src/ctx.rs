//! The shared context every function handler holds: configuration, the store,
//! the platform (vault and events), the HTTP client and the in-memory
//! credential.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::{CallAction, CallRequest, IIIClient};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::config::Config;
use crate::configuration::ConfigCell;
use crate::error::{CloudError, Res};
use crate::store::Store;
use crate::types::*;

pub type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The vault consumer name and the secret that holds a browser session token.
pub const SESSION_SECRET: &str = "IIICLOUD_SESSION";
pub const ON_CHANGE_FN: &str = "iii-cloud::on-change";

/// What the worker needs from the engine: the vault and the event call.
pub trait Platform: Send + Sync {
    /// `secrets::resolve`; `None` when the secret is not stored.
    fn secret_resolve<'a>(&'a self, name: &'a str) -> Fut<'a, Res<Option<String>>>;
    /// `secrets::compare-and-set` with `consumers: ["iii-cloud"]`; true when swapped.
    fn secret_cas<'a>(
        &'a self,
        name: &'a str,
        expected: Option<&'a str>,
        value: &'a str,
    ) -> Fut<'a, Res<bool>>;
    /// Fire-and-forget `iii-cloud::on-change`.
    fn emit(&self, event: ChangeEvent);
    /// Call another function on the engine (the session and harness reads of a hand-off).
    fn call<'a>(&'a self, function: &'a str, payload: Value) -> Fut<'a, Res<Value>>;
}

pub struct IiiPlatform {
    pub iii: Arc<IIIClient>,
}

fn vault_error(e: iii_sdk::errors::Error) -> CloudError {
    match e {
        iii_sdk::errors::Error::Remote { code, .. } => CloudError::new(
            "INVALID_REQUEST",
            format!("the vault refused the call ({code})"),
        ),
        _ => CloudError::new("INVALID_REQUEST", "the vault did not answer"),
    }
}

impl Platform for IiiPlatform {
    fn secret_resolve<'a>(&'a self, name: &'a str) -> Fut<'a, Res<Option<String>>> {
        Box::pin(async move {
            let request = CallRequest {
                timeout_ms: Some(10_000),
                ..CallRequest::new(
                    "secrets::resolve",
                    json!({ "ref": format!("secret://{name}") }),
                )
            };
            match self.iii.call(request).await {
                Ok(v) => Ok(v.get("value").and_then(Value::as_str).map(str::to_owned)),
                Err(iii_sdk::errors::Error::Remote { code, .. }) if code == "SECRET_NOT_FOUND" => {
                    Ok(None)
                }
                Err(e) => Err(vault_error(e)),
            }
        })
    }

    fn secret_cas<'a>(
        &'a self,
        name: &'a str,
        expected: Option<&'a str>,
        value: &'a str,
    ) -> Fut<'a, Res<bool>> {
        Box::pin(async move {
            let request = CallRequest {
                timeout_ms: Some(10_000),
                ..CallRequest::new(
                    "secrets::compare-and-set",
                    json!({
                        "name": name,
                        "expected": expected,
                        "value": value,
                        "consumers": [crate::WORKER_NAME],
                    }),
                )
            };
            let out = self.iii.call(request).await.map_err(vault_error)?;
            Ok(out.get("swapped").and_then(Value::as_bool).unwrap_or(false))
        })
    }

    fn call<'a>(&'a self, function: &'a str, payload: Value) -> Fut<'a, Res<Value>> {
        Box::pin(async move {
            let request = CallRequest {
                timeout_ms: Some(10_000),
                ..CallRequest::new(function, payload)
            };
            self.iii.call(request).await.map_err(|e| match e {
                iii_sdk::errors::Error::Remote { code, .. } => {
                    CloudError::new("ENGINE_CALL_FAILED", format!("{function} failed ({code})"))
                }
                _ => CloudError::new("ENGINE_CALL_FAILED", format!("{function} did not answer")),
            })
        })
    }

    fn emit(&self, event: ChangeEvent) {
        if !self.iii.is_observed(ON_CHANGE_FN) {
            return;
        }
        let iii = self.iii.clone();
        tokio::spawn(async move {
            let payload = serde_json::to_value(&event).unwrap_or(Value::Null);
            let request = CallRequest {
                action: Some(CallAction::Void),
                ..CallRequest::new(ON_CHANGE_FN, payload)
            };
            if let Err(e) = iii.call(request).await {
                tracing::warn!(error = %e, "iii-cloud::on-change call failed");
            }
        });
    }
}

/// A cloud credential, held in memory only. Never printed.
#[derive(Clone)]
pub struct Credential {
    pub method: Method,
    pub token: String,
}

/// The run a workspace has going: what it is of, how to stop it, how to wait for its end.
pub struct ActiveRun {
    pub id: String,
    pub env_id: String,
    /// The check's reuse key (HEAD, compose file, engine, environment, settings,
    /// values, Postgres image). A call attaches only to a run with the same
    /// environment and a key that is there and equal.
    pub key: Option<String>,
    /// The checks that attached to this run: the run publishes to them too.
    pub followers: Arc<Mutex<Vec<String>>>,
    pub cancel: watch::Sender<bool>,
    /// Turns true once the run's teardown is over.
    pub done: watch::Receiver<bool>,
}

pub struct Ctx {
    pub cfg: ConfigCell,
    pub store: Arc<Store>,
    pub plat: Arc<dyn Platform>,
    pub http: reqwest::Client,
    pub cred: Mutex<Option<Credential>>,
    /// Tests shrink the device poll's seconds to this.
    pub tick: Option<Duration>,
    /// Ids of the runs and deploys a task follows now.
    pub busy: Mutex<HashSet<String>>,
    /// The local run of each workspace (its path): one at a time per workspace.
    pub runs: Mutex<HashMap<String, ActiveRun>>,
    /// Ports the runs picked, held from the pick to the run's teardown.
    pub ports: Mutex<HashSet<u16>>,
    pub poller: Mutex<Option<JoinHandle<()>>>,
    /// The background poll of deployments made elsewhere.
    pub deploys_task: Mutex<Option<JoinHandle<()>>>,
    /// Values the person provided in this process: redacted even when no `.env` holds them.
    pub known_secrets: Mutex<Vec<String>>,
    /// Where iii cloud's functions are called: off when waits are shrunk (the tests,
    /// whose fake iii cloud is HTTP only); a test of the functions names an engine.
    pub functions_at: Mutex<crate::cloud::FunctionsAt>,
    /// The socket to those functions, for the credential in use.
    pub link: Mutex<Option<crate::cloud::Link>>,
}

impl Ctx {
    pub fn new(cfg: ConfigCell, store: Arc<Store>, plat: Arc<dyn Platform>) -> Arc<Self> {
        Self::with_tick(cfg, store, plat, None)
    }

    /// Like `new`, with every wait of whole seconds (device poll, deploy
    /// follow, push wait) shrunk to `tick`: for tests.
    pub fn with_tick(
        cfg: ConfigCell,
        store: Arc<Store>,
        plat: Arc<dyn Platform>,
        tick: Option<Duration>,
    ) -> Arc<Self> {
        Arc::new(Self {
            cfg,
            store,
            plat,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .user_agent(concat!("iii-cloud-worker/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("http client builds"),
            cred: Mutex::new(None),
            tick,
            busy: Mutex::new(HashSet::new()),
            runs: Mutex::new(HashMap::new()),
            ports: Mutex::new(HashSet::new()),
            poller: Mutex::new(None),
            deploys_task: Mutex::new(None),
            known_secrets: Mutex::new(Vec::new()),
            functions_at: Mutex::new(if tick.is_some() {
                crate::cloud::FunctionsAt::Off
            } else {
                crate::cloud::FunctionsAt::Cloud
            }),
            link: Mutex::new(None),
        })
    }

    /// The live configuration. A changed `cloud_url` signs out first: the
    /// token belongs to one cloud.
    pub async fn config(&self) -> Arc<Config> {
        let cfg = self.cfg.read().await.clone();
        let stale = self.store.read(|s| {
            s.connection
                .as_ref()
                .is_some_and(|c| c.status != ConnStatus::SignedOut && c.cloud_url != cfg.cloud_url)
        });
        if stale {
            *self.cred.lock().unwrap() = None;
            self.unlink();
            if let Some(handle) = self.poller.lock().unwrap().take() {
                handle.abort();
            }
            if let Some(handle) = self.deploys_task.lock().unwrap().take() {
                handle.abort();
                self.busy.lock().unwrap().remove("poller");
            }
            self.set_connection(Connection::signed_out(&cfg.cloud_url));
        }
        cfg
    }

    /// The current connection, after the `cloud_url` check.
    pub async fn conn(&self) -> Connection {
        let cfg = self.config().await;
        self.connection(&cfg)
    }

    pub fn connection(&self, cfg: &Config) -> Connection {
        self.store
            .read(|s| s.connection.clone())
            .unwrap_or_else(|| Connection::signed_out(&cfg.cloud_url))
    }

    /// Store the connection and emit `connection`; terminal unless waiting.
    pub fn set_connection(&self, connection: Connection) {
        let terminal = connection.status != ConnStatus::Waiting;
        let summary = match connection.status {
            ConnStatus::Connected => "signed in to iii cloud".to_string(),
            ConnStatus::Waiting => "waiting for the sign-in approval".to_string(),
            ConnStatus::Expired => "the iii cloud credential expired".to_string(),
            ConnStatus::SignedOut => "signed out of iii cloud".to_string(),
            ConnStatus::Error => "iii cloud sign-in failed".to_string(),
        };
        self.store.update(|s| {
            s.connection = Some(connection.clone());
            if terminal {
                s.pending_device = None;
            }
        });
        self.emit(
            EventKind::Connection,
            "current",
            terminal,
            summary,
            &connection,
        );
    }

    pub fn emit<T: Serialize>(
        &self,
        kind: EventKind,
        id: &str,
        terminal: bool,
        summary: String,
        record: &T,
    ) {
        self.plat.emit(ChangeEvent {
            kind,
            id: id.to_string(),
            terminal,
            summary,
            record: serde_json::to_value(record).unwrap_or(Value::Null),
        });
    }

    /// `n` seconds; a test tick stands for one second.
    pub fn secs(&self, n: u64) -> Duration {
        self.tick
            .map(|t| t * n as u32)
            .unwrap_or(Duration::from_secs(n))
    }
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}
