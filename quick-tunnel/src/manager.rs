use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use chrono::Utc;
use fs2::FileExt;
use futures_util::StreamExt;
use tokio::{
    io::AsyncRead,
    process::{Child, Command},
    sync::{broadcast, mpsc, oneshot},
    task::JoinHandle,
    time::Instant,
};
use tokio_util::codec::{FramedRead, LinesCodec};
use uuid::Uuid;

use crate::{
    api::*,
    config::{valid_id, Config},
    parser::{self, LogEvent},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("lease persistence: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid persisted leases: {0}")]
    Json(#[from] serde_json::Error),
    #[error("manager stopped")]
    Stopped,
}
type Result<T> = std::result::Result<T, Error>;
type Reply<T> = oneshot::Sender<Result<T>>;

enum Request {
    Acquire(AcquireRequest, Reply<AcquireResponse>),
    Release(ReleaseRequest, Reply<ReleaseResponse>),
    Status(StatusRequest, Reply<StatusResponse>),
    Shutdown(oneshot::Sender<()>),
}

/// All mutations and process transitions are serialized by one owning task.
#[derive(Clone)]
pub struct Manager {
    tx: mpsc::Sender<Request>,
    events: broadcast::Sender<Snapshot>,
}

impl Manager {
    pub fn open(config: Config) -> Result<Self> {
        config.validate().map_err(Error::Invalid)?;
        let parent = state_parent(&config.state_path);
        std::fs::create_dir_all(parent)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(config.state_path.with_extension("lock"))?;
        lock.try_lock_exclusive()?;
        let leases: Vec<Lease> = match std::fs::read(&config.state_path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        let mut unique = std::collections::BTreeSet::new();
        let mut ids = std::collections::BTreeSet::new();
        if leases.len() > config.max_leases
            || leases.iter().any(|l| {
                !valid_id(&l.consumer_id)
                    || !config.targets.contains_key(&l.tunnel_id)
                    || Uuid::parse_str(&l.lease_id).is_err()
                    || !unique.insert((l.consumer_id.clone(), l.tunnel_id.clone()))
                    || !ids.insert(l.lease_id.clone())
            })
        {
            return Err(Error::Invalid(
                "persisted lease identities or targets are invalid".into(),
            ));
        }
        let leases: Vec<_> = leases
            .into_iter()
            .filter(|l| l.expires_at > Utc::now())
            .collect();
        persist(&config.state_path, &leases)?;
        let (tx, rx) = mpsc::channel(128);
        let (log_tx, log_rx) = mpsc::channel(128);
        let (events, _) = broadcast::channel(256);
        let tunnels = config
            .targets
            .keys()
            .map(|id| (id.clone(), Tunnel::new(id)))
            .collect();
        let actor = Actor {
            config,
            leases,
            tunnels,
            events: events.clone(),
            log_tx,
            log_rx,
            _lock: lock,
            persistence_failed: false,
        };
        tokio::spawn(actor.run(rx));
        Ok(Self { tx, events })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Snapshot> {
        self.events.subscribe()
    }

    pub async fn acquire(&self, request: AcquireRequest) -> Result<AcquireResponse> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Request::Acquire(request, tx))
            .await
            .map_err(|_| Error::Stopped)?;
        rx.await.map_err(|_| Error::Stopped)?
    }
    pub async fn release(&self, request: ReleaseRequest) -> Result<ReleaseResponse> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Request::Release(request, tx))
            .await
            .map_err(|_| Error::Stopped)?;
        rx.await.map_err(|_| Error::Stopped)?
    }
    pub async fn status(&self, request: StatusRequest) -> Result<StatusResponse> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Request::Status(request, tx))
            .await
            .map_err(|_| Error::Stopped)?;
        rx.await.map_err(|_| Error::Stopped)?
    }
    /// Stop and reap children; keep unexpired leases for recovery on the next boot.
    pub async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Request::Shutdown(tx)).await.is_ok() {
            let _ = rx.await;
        }
    }
}

fn state_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn persist(path: &Path, leases: &[Lease]) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(state_parent(path))?;
    serde_json::to_writer(&mut file, leases)?;
    file.flush()?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    File::open(state_parent(path))?.sync_all()?;
    Ok(())
}

struct Process {
    child: Child,
    readers: Vec<JoinHandle<()>>,
    _home: tempfile::TempDir,
}
impl Drop for Process {
    fn drop(&mut self) {
        // Child::kill_on_drop is the final safety net if the owning task is cancelled.
        for reader in &self.readers {
            reader.abort();
        }
    }
}
impl Process {
    async fn stop(mut self) {
        if let Err(e) = self.child.kill().await {
            tracing::warn!(error = %e, "child kill failed; waiting for reap");
        }
        if let Err(e) = self.child.wait().await {
            tracing::warn!(error = %e, "child reap failed");
        }
        for reader in &self.readers {
            reader.abort();
        }
        for reader in self.readers.drain(..) {
            let _ = reader.await;
        }
    }
}

struct Tunnel {
    snapshot: Snapshot,
    process: Option<Process>,
    candidate: Option<String>,
    connected: bool,
    deadline: Option<Instant>,
    retry_at: Option<Instant>,
    attempts: u32,
}
impl Tunnel {
    fn new(id: &str) -> Self {
        Self {
            snapshot: Snapshot {
                tunnel_id: id.into(),
                status: Status::Stopped,
                public_url: None,
                generation: Uuid::new_v4().to_string(),
                error: None,
            },
            process: None,
            candidate: None,
            connected: false,
            deadline: None,
            retry_at: None,
            attempts: 0,
        }
    }
    fn emit(
        &mut self,
        events: &broadcast::Sender<Snapshot>,
        status: Status,
        error: Option<String>,
    ) {
        self.snapshot.status = status;
        self.snapshot.error = error;
        self.snapshot.public_url = if status == Status::Ready {
            self.candidate.clone()
        } else {
            None
        };
        let _ = events.send(self.snapshot.clone());
    }
}

struct Output {
    id: String,
    generation: String,
    event: Option<LogEvent>,
    broken: bool,
}
struct Actor {
    config: Config,
    leases: Vec<Lease>,
    tunnels: BTreeMap<String, Tunnel>,
    events: broadcast::Sender<Snapshot>,
    log_tx: mpsc::Sender<Output>,
    log_rx: mpsc::Receiver<Output>,
    _lock: File,
    persistence_failed: bool,
}

impl Actor {
    async fn run(mut self, mut requests: mpsc::Receiver<Request>) {
        self.reconcile().await;
        let mut tick = tokio::time::interval(Duration::from_millis(50));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let done = loop {
            tokio::select! {
                request = requests.recv() => {
                    let Some(request) = request else { break None; };
                    self.expire().await;
                    match request {
                        Request::Acquire(req, reply) => {
                            let result = self.acquire(req);
                            self.reconcile().await;
                            let result = result.map(|(id, lease_id)| AcquireResponse { lease_id, snapshot: self.tunnels[&id].snapshot.clone() });
                            let _ = reply.send(result);
                        }
                        Request::Release(req, reply) => {
                            let result = self.release(&req.lease_id);
                            self.reconcile().await;
                            let _ = reply.send(result);
                        }
                        Request::Status(req, reply) => {
                            self.reconcile().await;
                            let result = self.tunnels.get(&req.tunnel_id).map(|t| StatusResponse {
                                snapshot: t.snapshot.clone(),
                                leases: self.leases.iter().filter(|l| l.tunnel_id == req.tunnel_id).cloned().collect(),
                            }).ok_or_else(|| Error::Invalid("unknown tunnel_id".into()));
                            let _ = reply.send(result);
                        }
                        Request::Shutdown(reply) => break Some(reply),
                    }
                }
                output = self.log_rx.recv() => if let Some(output) = output { self.output(output).await; },
                _ = tick.tick() => { self.expire().await; self.reconcile().await; }
            }
        };
        for tunnel in self.tunnels.values_mut() {
            if let Some(p) = tunnel.process.take() {
                p.stop().await;
            }
            tunnel.emit(&self.events, Status::Stopped, None);
        }
        // Release the exclusive state lock before acknowledging shutdown.
        drop(self);
        if let Some(done) = done {
            let _ = done.send(());
        }
    }

    fn acquire(&mut self, request: AcquireRequest) -> Result<(String, String)> {
        if self.persistence_failed {
            return Err(Error::Invalid(
                "state persistence failed; restart required".into(),
            ));
        }
        if !valid_id(&request.consumer_id) || !self.tunnels.contains_key(&request.tunnel_id) {
            return Err(Error::Invalid(
                "invalid consumer_id or unauthorized tunnel_id".into(),
            ));
        }
        let remaining = request.expires_at.signed_duration_since(Utc::now());
        if remaining.num_milliseconds() <= 0
            || remaining > chrono::Duration::seconds(self.config.max_lease_seconds)
        {
            return Err(Error::Invalid(
                "expires_at must be in the future within max_lease_seconds".into(),
            ));
        }
        let mut next = self.leases.clone();
        let lease_id = if let Some(lease) = next
            .iter_mut()
            .find(|l| l.consumer_id == request.consumer_id && l.tunnel_id == request.tunnel_id)
        {
            // Retries/renewals never shorten another in-flight renewal.
            lease.expires_at = lease.expires_at.max(request.expires_at);
            lease.lease_id.clone()
        } else {
            if next.len() >= self.config.max_leases {
                return Err(Error::Invalid("lease limit reached".into()));
            }
            let lease_id = Uuid::new_v4().to_string();
            next.push(Lease {
                lease_id: lease_id.clone(),
                consumer_id: request.consumer_id,
                tunnel_id: request.tunnel_id.clone(),
                expires_at: request.expires_at,
            });
            lease_id
        };
        persist(&self.config.state_path, &next)?;
        self.leases = next;
        Ok((request.tunnel_id, lease_id))
    }

    fn release(&mut self, id: &str) -> Result<ReleaseResponse> {
        let mut next = self.leases.clone();
        next.retain(|l| l.lease_id != id);
        let released = next.len() != self.leases.len();
        if released {
            persist(&self.config.state_path, &next)?;
            self.leases = next;
        }
        Ok(ReleaseResponse { released })
    }

    async fn expire(&mut self) {
        let before = self.leases.len();
        self.leases.retain(|l| l.expires_at > Utc::now());
        if before != self.leases.len() {
            if let Err(error) = persist(&self.config.state_path, &self.leases) {
                tracing::error!(%error, "expiry persistence failed; closing all tunnels");
                self.persistence_failed = true;
                for tunnel in self.tunnels.values_mut() {
                    if let Some(p) = tunnel.process.take() {
                        p.stop().await;
                    }
                    tunnel.retry_at = None;
                    tunnel.emit(
                        &self.events,
                        Status::Failed,
                        Some("lease persistence failed; restart required".into()),
                    );
                }
            }
        }
    }

    async fn reconcile(&mut self) {
        if self.persistence_failed {
            return;
        }
        for (id, tunnel) in &mut self.tunnels {
            let active = self.leases.iter().any(|l| &l.tunnel_id == id);
            if !active {
                if let Some(p) = tunnel.process.take() {
                    p.stop().await;
                }
                if tunnel.snapshot.status != Status::Stopped {
                    tunnel.emit(&self.events, Status::Stopped, None);
                }
                tunnel.attempts = 0;
                tunnel.retry_at = None;
                continue;
            }
            let failure = if let Some(p) = &mut tunnel.process {
                match p.child.try_wait() {
                    Ok(Some(status)) => Some(format!("cloudflared exited ({status})")),
                    Err(_) => Some("cloudflared wait failed".into()),
                    Ok(None) if tunnel.deadline.is_some_and(|d| Instant::now() >= d) => {
                        Some("cloudflared startup/reconnect timeout".into())
                    }
                    _ => None,
                }
            } else {
                None
            };
            if let Some(error) = failure {
                fail(tunnel, &self.config, &self.events, error).await;
            }
            if tunnel.snapshot.status == Status::Stopped
                || tunnel.retry_at.is_some_and(|d| Instant::now() >= d)
            {
                tunnel.attempts += 1;
                tunnel.snapshot.generation = Uuid::new_v4().to_string();
                tunnel.candidate = None;
                tunnel.connected = false;
                tunnel.retry_at = None;
                tunnel.deadline =
                    Some(Instant::now() + Duration::from_millis(self.config.startup_timeout_ms));
                let status = if tunnel.attempts == 1 {
                    Status::Starting
                } else {
                    Status::Reconnecting
                };
                tunnel.emit(&self.events, status, None);
                match spawn(
                    &self.config,
                    id,
                    &tunnel.snapshot.generation,
                    self.log_tx.clone(),
                ) {
                    Ok(process) => tunnel.process = Some(process),
                    Err(e) => {
                        fail(
                            tunnel,
                            &self.config,
                            &self.events,
                            format!(
                                "cloudflared spawn failed ({:?}); verify prerequisite executable",
                                e.kind()
                            ),
                        )
                        .await
                    }
                }
            }
        }
    }

    async fn output(&mut self, output: Output) {
        let Some(tunnel) = self.tunnels.get_mut(&output.id) else {
            return;
        };
        if tunnel.snapshot.generation != output.generation || tunnel.process.is_none() {
            return;
        }
        if output.broken {
            fail(
                tunnel,
                &self.config,
                &self.events,
                "cloudflared log stream invalid or oversized".into(),
            )
            .await;
            return;
        }
        match output.event {
            Some(LogEvent::Url(url)) => {
                if tunnel.candidate.as_ref().is_some_and(|old| old != &url) {
                    fail(
                        tunnel,
                        &self.config,
                        &self.events,
                        "conflicting public URLs in one generation".into(),
                    )
                    .await;
                    return;
                }
                tunnel.candidate = Some(url);
            }
            Some(LogEvent::Connected) => tunnel.connected = true,
            Some(LogEvent::Disconnected) => {
                tunnel.connected = false;
                // Do not extend this deadline on every repeated reconnect log.
                tunnel.deadline.get_or_insert(
                    Instant::now() + Duration::from_millis(self.config.startup_timeout_ms),
                );
                tunnel.emit(
                    &self.events,
                    Status::Reconnecting,
                    Some("cloudflared edge connection interrupted".into()),
                );
            }
            None => {}
        }
        if tunnel.connected && tunnel.candidate.is_some() && tunnel.snapshot.status != Status::Ready
        {
            tunnel.deadline = None;
            tunnel.emit(&self.events, Status::Ready, None);
        }
    }
}

async fn fail(
    t: &mut Tunnel,
    config: &Config,
    events: &broadcast::Sender<Snapshot>,
    error: String,
) {
    if let Some(p) = t.process.take() {
        p.stop().await;
    }
    t.connected = false;
    t.deadline = None;
    let retry = t.attempts <= config.max_retries;
    t.retry_at = retry.then(|| {
        Instant::now()
            + Duration::from_millis(
                config
                    .retry_initial_ms
                    .saturating_mul(1 << t.attempts.saturating_sub(1))
                    .min(config.retry_max_ms),
            )
    });
    t.emit(
        events,
        if retry {
            Status::Reconnecting
        } else {
            Status::Failed
        },
        Some(error),
    );
}

fn spawn(
    config: &Config,
    id: &str,
    generation: &str,
    tx: mpsc::Sender<Output>,
) -> std::io::Result<Process> {
    let home = tempfile::Builder::new()
        .prefix("cloudflared-")
        .tempdir_in(state_parent(&config.state_path))?;
    let home_path: PathBuf = home.path().canonicalize()?;
    let empty_config = home_path.join("config.yaml");
    std::fs::write(&empty_config, "{}\n")?;
    // No inherited TUNNEL_*, proxy variables, credentials, or home config.
    let mut child = Command::new(&config.cloudflared)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", &home_path)
        .current_dir(&home_path)
        .args(["tunnel", "--config"])
        .arg(empty_config)
        .args([
            "--no-autoupdate",
            "--output",
            "json",
            "--loglevel",
            "info",
            "--metrics",
            "127.0.0.1:0",
            "--retries",
            "0",
            "--grace-period",
            "0s",
            "--url",
        ])
        .arg(&config.targets[id])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        readers.push(read_output(stdout, id, generation, tx.clone()));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(read_output(stderr, id, generation, tx));
    }
    Ok(Process {
        child,
        readers,
        _home: home,
    })
}

fn read_output<R: AsyncRead + Unpin + Send + 'static>(
    reader: R,
    id: &str,
    generation: &str,
    tx: mpsc::Sender<Output>,
) -> JoinHandle<()> {
    let id = id.to_owned();
    let generation = generation.to_owned();
    tokio::spawn(async move {
        let mut lines = FramedRead::new(reader, LinesCodec::new_with_max_length(16_384));
        while let Some(line) = lines.next().await {
            let (event, broken) = match line {
                Ok(line) => (parser::parse(&line), false),
                Err(_) => (None, true),
            };
            if (event.is_some() || broken)
                && tx
                    .send(Output {
                        id: id.clone(),
                        generation: generation.clone(),
                        event,
                        broken,
                    })
                    .await
                    .is_err()
            {
                break;
            }
            if broken {
                break;
            }
        }
    })
}
