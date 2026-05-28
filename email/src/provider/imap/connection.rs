//! Open a single TLS+IMAP session, login, select a folder, verify IDLE
//! capability. Used by both the IDLE listener (`idle.rs`) and the on-demand
//! pool (`mod.rs::ImapPool`). The two paths must NEVER share a session — IMAP
//! is half-duplex once a command is in flight.

use iii_sdk::{protocol::TriggerRequest, IIIError, III};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

use crate::config::{ImapConfig, WorkerConfig};
use crate::triggers::dispatcher::EventDispatcher;

use super::Session;

pub async fn open_and_select(
    account: &str,
    folder: &str,
    cfg: &ImapConfig,
    connect_timeout_ms: u64,
) -> Result<Session, IIIError> {
    let cred = fetch_credential(account).await?;
    open_and_select_with_cred(account, folder, cfg, connect_timeout_ms, &cred).await
}

pub async fn open_and_select_with_cred(
    account: &str,
    folder: &str,
    cfg: &ImapConfig,
    connect_timeout_ms: u64,
    cred: &Value,
) -> Result<Session, IIIError> {
    let user = cred
        .get("username")
        .and_then(|v| v.as_str())
        .ok_or_else(|| handler_err("E608", "credential missing `username`"))?;
    let pass = cred
        .get("password")
        .and_then(|v| v.as_str())
        .ok_or_else(|| handler_err("E608", "credential missing `password`"))?;

    let tcp = tokio::time::timeout(
        Duration::from_millis(connect_timeout_ms),
        TcpStream::connect((cfg.host.as_str(), cfg.port)),
    )
    .await
    .map_err(|_| {
        handler_err(
            "E614",
            &format!("imap connect timeout to {}:{}", cfg.host, cfg.port),
        )
    })?
    .map_err(|e| handler_err("E614", &format!("imap connect failed: {e}")))?;

    if !cfg.tls {
        return Err(handler_err(
            "E615",
            "plain (non-TLS) imap is refused; set imap.tls: true",
        ));
    }

    let tls = build_tls_connector()?;
    let server_name = rustls::pki_types::ServerName::try_from(cfg.host.clone())
        .map_err(|e| handler_err("E614", &format!("invalid imap host `{}`: {e}", cfg.host)))?;
    let stream: TlsStream<TcpStream> = tls
        .connect(server_name, tcp)
        .await
        .map_err(|e| handler_err("E614", &format!("imap TLS handshake failed: {e}")))?;

    let client = async_imap::Client::new(stream);
    let mut session = client
        .login(user, pass)
        .await
        .map_err(|(e, _)| handler_err("E616", &format!("imap login failed for {account}: {e}")))?;

    let caps = session
        .capabilities()
        .await
        .map_err(|e| handler_err("E614", &format!("imap CAPABILITY failed: {e}")))?;
    let has_idle = caps.iter().any(|c| {
        matches!(
            c,
            async_imap::types::Capability::Atom(s) if s.eq_ignore_ascii_case("IDLE")
        )
    });
    drop(caps);
    if !has_idle {
        return Err(handler_err(
            "E610",
            &format!(
                "imap server {} does not advertise IDLE; refusing to fall back to polling",
                cfg.host
            ),
        ));
    }

    session
        .select(folder)
        .await
        .map_err(|e| handler_err("E617", &format!("imap SELECT `{folder}` failed: {e}")))?;

    tracing::info!(account = %account, folder = %folder, host = %cfg.host, "imap session ready (IDLE supported)");
    Ok(session)
}

pub async fn run_until_shutdown(
    account: String,
    folder: String,
    cfg: Arc<WorkerConfig>,
    dispatcher: Arc<dyn EventDispatcher>,
) {
    let acct_cfg = match cfg.accounts.get(&account) {
        Some(a) => a.clone(),
        None => {
            tracing::error!(account = %account, "account vanished from config; idle supervisor exiting");
            return;
        }
    };
    let imap_cfg = match acct_cfg.imap.clone() {
        Some(i) => i,
        None => {
            tracing::error!(account = %account, "account has no imap config; idle supervisor exiting");
            return;
        }
    };

    let attempts = Arc::new(Mutex::new(0u32));

    loop {
        let connect_attempt = {
            let g = attempts.lock().await;
            *g
        };
        if connect_attempt > 0 {
            super::reconnect::backoff(connect_attempt).await;
        }

        let session = match open_and_select(
            &account,
            &folder,
            &imap_cfg,
            cfg.limits.imap_connect_timeout_ms,
        )
        .await
        {
            Ok(s) => {
                let mut g = attempts.lock().await;
                *g = 0;
                s
            }
            Err(e) => {
                tracing::warn!(
                    account = %account,
                    folder = %folder,
                    attempt = connect_attempt,
                    error = %e,
                    "imap connect failed; will retry on reconnect event"
                );
                let mut g = attempts.lock().await;
                *g = (*g).saturating_add(1);
                continue;
            }
        };

        // Hand the session to the IDLE loop; returns only when the connection
        // dies or shutdown fires. No timer-based polling inside.
        if let Err(e) =
            super::idle::run(session, dispatcher.clone(), account.clone(), folder.clone()).await
        {
            tracing::warn!(account = %account, folder = %folder, error = %e, "imap IDLE loop exited");
        }

        let mut g = attempts.lock().await;
        *g = (*g).saturating_add(1);
    }
}

async fn fetch_credential(account: &str) -> Result<Value, IIIError> {
    // The connection module is invoked from contexts that hold an `Arc<III>`
    // upstream. For the supervisor path we re-resolve via the global SDK
    // handle exposed at `connection::iii_handle`. For the pool path the
    // caller already has the engine handle. Both forward to auth-credentials.
    let iii = iii_handle().ok_or_else(|| {
        handler_err(
            "E618",
            "internal: iii handle not initialized for credential fetch",
        )
    })?;
    let cred = iii
        .trigger(TriggerRequest {
            function_id: "auth::get_token".to_string(),
            payload: json!({ "provider": format!("email::{}", account) }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .map_err(|e| {
            handler_err(
                "E606",
                &format!("auth::get_token failed for `email::{account}`: {e}"),
            )
        })?;
    if cred.is_null() {
        return Err(handler_err(
            "E607",
            &format!("no credential stored for `email::{account}` — call auth::set_token first"),
        ));
    }
    Ok(cred)
}

fn build_tls_connector() -> Result<TlsConnector, IIIError> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let cfg = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(cfg)))
}

fn handler_err(code: &str, msg: &str) -> IIIError {
    IIIError::Handler(json!({ "code": code, "message": msg }).to_string())
}

// ---- shared III handle for credential lookup from non-handler tasks ----
//
// The IMAP supervisor runs in tokio::spawn from main; it doesn't carry an
// Arc<III> through every layer. We stash a OnceCell-backed handle that
// main.rs initializes once after register_worker.

use std::sync::OnceLock;
static III_HANDLE: OnceLock<Arc<III>> = OnceLock::new();

pub fn install_iii_handle(iii: Arc<III>) {
    let _ = III_HANDLE.set(iii);
}

fn iii_handle() -> Option<Arc<III>> {
    III_HANDLE.get().cloned()
}

#[allow(dead_code)]
fn _suppress_unused(_: anyhow::Error) {}
