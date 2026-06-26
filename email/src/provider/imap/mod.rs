pub mod connection;
pub mod fetch;
pub mod idle;
pub mod reconnect;

use dashmap::DashMap;
use iii_sdk::errors::Error;
use serde_json::json;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio_rustls::client::TlsStream;

use crate::config::WorkerConfig;

/// Pool of ad-hoc IMAP sessions, one per (account, folder), used by
/// list/get/search/flag/move/attachment::get. The IDLE listener runs in a
/// separate task per (account, folder) and holds its own connection — these
/// pool entries are for synchronous request/response work only.
pub struct ImapPool {
    sessions: DashMap<(String, String), Arc<Mutex<Option<Session>>>>,
    cfg: Arc<WorkerConfig>,
}

pub type Session = async_imap::Session<TlsStream<TcpStream>>;

impl ImapPool {
    pub fn new(cfg: Arc<WorkerConfig>) -> Self {
        Self {
            sessions: DashMap::new(),
            cfg,
        }
    }

    /// Acquire (or open) a connection for the given (account, folder). Returns
    /// an owned guard that holds the lock for the duration of the IMAP
    /// exchange. Reconnects transparently on first acquire after a drop.
    pub async fn acquire(&self, account: &str, folder: &str) -> Result<SessionGuard, Error> {
        // Validate FIRST — invalid (account, folder) tuples must never reach
        // the DashMap, otherwise a malformed-payload flood from a hostile
        // caller can balloon `self.sessions` without bound.
        let acct = self.cfg.accounts.get(account).ok_or_else(|| {
            Error::Handler(
                json!({"code":"E600","message":format!("unknown account `{account}`")}).to_string(),
            )
        })?;
        let imap_cfg = acct.imap.as_ref().ok_or_else(|| {
            Error::Handler(
                json!({"code":"E603","message":format!("account `{account}` has no imap config")})
                    .to_string(),
            )
        })?;
        if !imap_cfg.folders.iter().any(|f| f == folder) {
            return Err(Error::Handler(
                json!({
                    "code":"E613",
                    "message":format!("folder `{folder}` not in account `{account}`.imap.folders")
                })
                .to_string(),
            ));
        }

        let key = (account.to_string(), folder.to_string());
        let slot = self
            .sessions
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .clone();

        let mut guard = slot.lock_owned().await;
        if guard.is_none() {
            *guard = Some(
                connection::open_and_select(
                    account,
                    folder,
                    imap_cfg,
                    self.cfg.limits.imap_connect_timeout_ms,
                )
                .await?,
            );
        }
        Ok(SessionGuard { guard })
    }
}

pub struct SessionGuard {
    guard: OwnedMutexGuard<Option<Session>>,
}

impl SessionGuard {
    pub fn session(&mut self) -> &mut Session {
        self.guard.as_mut().expect("session populated on acquire")
    }

    /// Mark the underlying session as dead so the next `acquire` opens a fresh
    /// one. Call after any error that suggests the socket is no longer healthy.
    pub fn poison(&mut self) {
        *self.guard = None;
    }
}
