//! Spawn / shutdown / discovery of the rustfs sidecar.
//!
//! Spawned only when `provider: local` is configured. One sidecar per
//! worker process; multiple `local` buckets share it.

use crate::error::StorageError;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::{Child, Command};

pub struct RustfsSpawnOpts {
    pub data_dir: String,
    pub access_key: String,
    pub secret_key: String,
    /// Pre-allocated port (TOCTOU window accepted on loopback).
    pub port: u16,
    /// If set, configure rustfs's notify-webhook target to POST events here.
    /// Format follows MinIO conventions (rustfs's notify subsystem mirrors them):
    /// `RUSTFS_NOTIFY_WEBHOOK_ENABLE_<id>=on`, `..._ENDPOINT_<id>=URL`.
    pub notify_webhook_url: Option<String>,
}

pub struct RustfsHandle {
    pub child: Child,
    pub port: u16,
    pub access_key: String,
    pub secret_key: String,
}

/// Find a rustfs binary in this order:
/// 1. `$RUSTFS_BIN`
/// 2. `./rustfs` next to the storage binary
/// 3. `rustfs` on `$PATH`
pub fn discover_binary() -> Result<PathBuf, StorageError> {
    if let Ok(p) = std::env::var("RUSTFS_BIN") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Ok(pb);
        }
    }
    if let Ok(self_exe) = std::env::current_exe() {
        if let Some(dir) = self_exe.parent() {
            let candidate = dir.join("rustfs");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    if let Ok(p) = which::which("rustfs") {
        return Ok(p);
    }
    Err(StorageError::LocalBackendBinNotFound {
        reason: "set $RUSTFS_BIN, place a rustfs binary next to storage, or install rustfs on PATH"
            .into(),
    })
}

/// Allocate a kernel-assigned port via a temporary `TcpListener` bind.
/// There's a small TOCTOU window between drop here and rustfs bind; on
/// loopback in a single-tenant worker container that's acceptable.
pub fn allocate_port() -> Result<u16, StorageError> {
    use std::net::TcpListener;
    let l = TcpListener::bind("127.0.0.1:0").map_err(|e| StorageError::LocalBackendBootFailed {
        reason: format!("port allocation: {e}"),
    })?;
    let port = l
        .local_addr()
        .map_err(|e| StorageError::LocalBackendBootFailed {
            reason: format!("port read: {e}"),
        })?
        .port();
    drop(l);
    Ok(port)
}

/// Random ephemeral credentials. Used as RUSTFS_ACCESS_KEY / RUSTFS_SECRET_KEY
/// — only the worker talks to rustfs over loopback so these never leave the
/// worker process.
pub fn ephemeral_credentials() -> (String, String) {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let user: [u8; 12] = rng.gen();
    let pass: [u8; 32] = rng.gen();
    (hex::encode(user), hex::encode(pass))
}

pub async fn spawn(opts: RustfsSpawnOpts) -> Result<RustfsHandle, StorageError> {
    let bin = discover_binary()?;
    let addr = format!("127.0.0.1:{}", opts.port);
    // rustfs CLI: `rustfs server <volumes>` with --address and creds via env.
    // Console server is disabled to avoid an extra bound port.
    let mut cmd = Command::new(&bin);
    cmd.arg("server")
        .arg(&opts.data_dir)
        .arg("--address")
        .arg(&addr)
        .env("RUSTFS_ACCESS_KEY", &opts.access_key)
        .env("RUSTFS_SECRET_KEY", &opts.secret_key)
        .env("RUSTFS_CONSOLE_ENABLE", "false")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if opts.notify_webhook_url.is_some() {
        // rustfs has a global notification module switch that defaults to OFF
        // (see rustfs::server::event::NOTIFY_MODULE_ENABLED + DEFAULT_NOTIFY_ENABLE).
        // Without this the system never `init()`s and put_bucket_notification_configuration
        // returns `InternalError: System has not been initialized`.
        cmd.env("RUSTFS_NOTIFY_ENABLE", "true");

        // We deliberately DO NOT pass `RUSTFS_NOTIFY_WEBHOOK_ENDPOINT_1` etc.
        // here. rustfs runs target init() (which probes the URL with HEAD)
        // during server startup; if our worker's `/notify` receiver isn't
        // bound yet — and it isn't, since we bind it after rustfs becomes
        // healthy — the probe times out, init returns NotConnected, and the
        // target is silently dropped from the live registry. Instead, we
        // register the webhook target via rustfs's admin API AFTER the
        // receiver is up (see `local::register_webhook_target`).
    }
    let child = cmd
        .spawn()
        .map_err(|e| StorageError::LocalBackendBootFailed {
            reason: format!("spawn rustfs: {e}"),
        })?;
    Ok(RustfsHandle {
        child,
        port: opts.port,
        access_key: opts.access_key,
        secret_key: opts.secret_key,
    })
}

pub async fn shutdown(mut handle: RustfsHandle) {
    use tokio::time::{timeout, Duration};
    let pid = handle.child.id();
    #[cfg(unix)]
    if let Some(id) = pid {
        // SAFETY: id is the rustfs sidecar's PID we just looked up; libc::kill
        // with SIGTERM is async-signal-safe and cannot trigger UB even if the
        // PID has been recycled (worst case: ESRCH or EPERM, both surfaced
        // below).
        let rc = unsafe { libc::kill(id as i32, libc::SIGTERM) };
        if rc != 0 {
            let errno = std::io::Error::last_os_error();
            tracing::debug!(
                pid = id,
                error = %errno,
                "rustfs SIGTERM returned non-zero (already exited?)"
            );
        }
    }
    match timeout(Duration::from_secs(10), handle.child.wait()).await {
        Ok(Ok(status)) => {
            tracing::info!(?pid, ?status, "rustfs sidecar exited cleanly");
        }
        Ok(Err(e)) => {
            tracing::warn!(?pid, error = %e, "rustfs wait() failed");
        }
        Err(_) => {
            tracing::warn!(
                ?pid,
                "rustfs sidecar did not exit within 10s; sending SIGKILL"
            );
            if let Err(e) = handle.child.kill().await {
                tracing::warn!(?pid, error = %e, "rustfs SIGKILL failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_binary_reads_env_var() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("RUSTFS_BIN", temp.path());
        let p = discover_binary().unwrap();
        assert_eq!(p, temp.path());
        std::env::remove_var("RUSTFS_BIN");
    }

    #[test]
    fn ephemeral_credentials_are_unique() {
        let (a, _) = ephemeral_credentials();
        let (b, _) = ephemeral_credentials();
        assert_ne!(a, b);
    }

    #[test]
    fn allocate_port_returns_nonzero() {
        let p = allocate_port().unwrap();
        assert!(p > 0);
    }
}
