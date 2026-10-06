//! The `simbridge` helper (bridge/simbridge.swift): compiled once per host
//! with the host's own Xcode, then one child process per simulator being
//! watched or driven. It streams the framebuffer and injects HID events; see
//! the Swift file for the JSON-lines protocol.
//!
//! Frames reach watchers as `ios-simulator::frame-event` triggers bound to
//! the udid, so any number of console tabs share one capture. Streaming runs
//! only while someone holds a watch lease; the bridge idles (input only)
//! otherwise and exits after a minute without use.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};

use crate::config::WorkerConfig;
use crate::events::{Events, FrameEvent};
use crate::tenant::Tenant;

const SOURCE: &str = include_str!("../bridge/simbridge.swift");
/// How long one watch call keeps the stream running; watchers renew it.
pub const WATCH_LEASE_MS: i64 = 15_000;
/// An idle bridge (no watcher, no input) exits after this long.
const IDLE_EXIT_MS: i64 = 60_000;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub data: String,
    pub width: u32,
    pub height: u32,
    pub seq: u64,
    pub timestamp: i64,
}

pub struct Bridge {
    pub tenant: String,
    pub udid: String,
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    pending: StdMutex<HashMap<u64, oneshot::Sender<Result<(), String>>>>,
    next_id: AtomicU64,
    /// Device framebuffer size: the coordinate space of every touch.
    size: StdMutex<(u32, u32)>,
    latest: StdMutex<Option<Arc<Frame>>>,
    alive: AtomicBool,
    streaming: AtomicBool,
    watch_until: AtomicI64,
    last_used: AtomicI64,
    events: Arc<Events>,
}

impl Bridge {
    pub fn size(&self) -> (u32, u32) {
        *self.size.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn latest(&self) -> Option<Arc<Frame>> {
        self.latest
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    pub fn is_streaming(&self) -> bool {
        self.streaming.load(Ordering::SeqCst)
    }

    async fn write(&self, mut cmd: Value, id: Option<u64>) -> Result<(), String> {
        if let Some(id) = id {
            cmd["id"] = json!(id);
        }
        let mut line = cmd.to_string();
        line.push('\n');
        self.last_used.store(now_ms(), Ordering::Relaxed);
        self.stdin
            .lock()
            .await
            .write_all(line.as_bytes())
            .await
            .map_err(|_| format!("simulator {} is no longer reachable", self.udid))
    }

    /// Fire-and-forget: ordered with every other command, never awaited.
    pub async fn send(&self, cmd: Value) -> Result<(), String> {
        self.write(cmd, None).await
    }

    /// Send and wait until the simulator accepted the event.
    pub async fn call(&self, cmd: Value) -> Result<(), String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, tx);
        self.write(cmd, Some(id)).await?;
        match tokio::time::timeout(Duration::from_secs(10), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(format!("simulator {} bridge exited", self.udid)),
            Err(_) => Err(format!("simulator {} did not answer in 10s", self.udid)),
        }
    }

    /// Run composed steps (command, pause after) in order; waits for the last.
    pub async fn play(&self, steps: Vec<(Value, u64)>) -> Result<(), String> {
        let last = steps.len().saturating_sub(1);
        for (i, (cmd, pause_ms)) in steps.into_iter().enumerate() {
            if i == last {
                self.call(cmd).await?;
            } else {
                self.send(cmd).await?;
            }
            if pause_ms > 0 {
                tokio::time::sleep(Duration::from_millis(pause_ms)).await;
            }
        }
        Ok(())
    }

    /// Renew the watch lease; start streaming at the current config if idle.
    pub async fn watch(&self, cfg: &WorkerConfig) -> Result<(), String> {
        self.watch_until
            .store(now_ms() + WATCH_LEASE_MS, Ordering::SeqCst);
        if !self.streaming.swap(true, Ordering::SeqCst) {
            self.send(json!({
                "op": "stream",
                "fps": cfg.stream_fps,
                "max_dim": cfg.stream_max_dimension,
                "quality": cfg.stream_quality as f64 / 100.0,
            }))
            .await?;
        }
        Ok(())
    }

    async fn read_loop(self: Arc<Self>, stdout: BufReader<tokio::process::ChildStdout>) {
        let mut lines = stdout.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            match msg["type"].as_str() {
                Some("ready") => {
                    let w = msg["width"].as_u64().unwrap_or(0) as u32;
                    let h = msg["height"].as_u64().unwrap_or(0) as u32;
                    *self.size.lock().unwrap_or_else(|p| p.into_inner()) = (w, h);
                }
                Some("frame") => {
                    let frame = Frame {
                        data: msg["data"].as_str().unwrap_or_default().to_string(),
                        width: msg["width"].as_u64().unwrap_or(0) as u32,
                        height: msg["height"].as_u64().unwrap_or(0) as u32,
                        seq: msg["seq"].as_u64().unwrap_or(0),
                        timestamp: now_ms(),
                    };
                    if self.is_streaming() {
                        let (device_width, device_height) = self.size();
                        self.events.frame(FrameEvent {
                            tenant: self.tenant.clone(),
                            udid: self.udid.clone(),
                            data: frame.data.clone(),
                            width: frame.width,
                            height: frame.height,
                            device_width,
                            device_height,
                            seq: frame.seq,
                        });
                    }
                    *self.latest.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(frame));
                }
                Some("done") => {
                    let id = msg["id"].as_u64().unwrap_or(u64::MAX);
                    let result = match msg["error"].as_str() {
                        Some(e) => Err(e.to_string()),
                        None => Ok(()),
                    };
                    if let Some(tx) = self
                        .pending
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(&id)
                    {
                        let _ = tx.send(result);
                    }
                }
                Some("error") => {
                    tracing::warn!(udid = %self.udid, message = %msg["message"], "simbridge error");
                }
                _ => {}
            }
        }
        self.alive.store(false, Ordering::SeqCst);
        // Dropping the senders fails every waiter with "bridge exited".
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        self.streaming.store(false, Ordering::SeqCst);
    }

    async fn stop(&self) {
        self.alive.store(false, Ordering::SeqCst);
        let _ = self.child.lock().await.kill().await;
        self.streaming.store(false, Ordering::SeqCst);
    }
}

/// Live bridges keyed by `<tenant>/<udid>`: a bridge started for one tenant
/// is never handed to another, even for the same udid.
pub struct Bridges {
    map: StdMutex<HashMap<String, Arc<Bridge>>>,
    /// One start at a time per simulator; other simulators never wait on it.
    starting: StdMutex<HashMap<String, Arc<Mutex<()>>>>,
    binary: Mutex<Option<PathBuf>>,
    events: Arc<Events>,
}

impl Bridges {
    pub fn new(events: Arc<Events>) -> Self {
        Self {
            map: StdMutex::new(HashMap::new()),
            starting: StdMutex::new(HashMap::new()),
            binary: Mutex::new(None),
            events,
        }
    }

    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Bridge>>> {
        self.map.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn find(&self, tenant: &str, udid: &str) -> Option<Arc<Bridge>> {
        self.map()
            .get(&format!("{tenant}/{udid}"))
            .filter(|b| b.is_alive())
            .cloned()
    }

    /// The running bridge for this simulator, or a new one. The device must
    /// be booted and belong to `tenant`'s set (the helper opens it there).
    pub async fn get_or_start(
        &self,
        tenant: &Tenant,
        udid: &str,
        data_root: &Path,
    ) -> Result<Arc<Bridge>, String> {
        let key = format!("{}/{udid}", tenant.name);
        let start_lock = self
            .starting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(key.clone())
            .or_default()
            .clone();
        let _starting = start_lock.lock().await;
        if let Some(bridge) = self.find(&tenant.name, udid) {
            return Ok(bridge);
        }
        let binary = self.binary(data_root, &tenant.set.developer_dir).await?;
        let mut cmd = Command::new(&binary);
        cmd.arg("--udid")
            .arg(udid)
            .arg("--developer-dir")
            .arg(&tenant.set.developer_dir)
            .args(["--fps", "0"]);
        if let Some(set) = &tenant.set.path {
            cmd.arg("--set").arg(set);
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("start simbridge: {e}"))?;
        let stdin = child.stdin.take().ok_or("simbridge stdin")?;
        let mut stdout = BufReader::new(child.stdout.take().ok_or("simbridge stdout")?);

        // The first line is `ready` (display found) or a fatal `error`.
        let mut first = String::new();
        tokio::time::timeout(Duration::from_secs(40), stdout.read_line(&mut first))
            .await
            .map_err(|_| format!("simulator {udid} display did not come up in 40s"))?
            .map_err(|e| format!("simbridge: {e}"))?;
        let msg: Value = serde_json::from_str(&first)
            .map_err(|_| format!("simulator {udid}: the bridge exited; is it booted?"))?;
        if msg["type"] != "ready" {
            return Err(msg["message"]
                .as_str()
                .unwrap_or("simbridge failed to start")
                .to_string());
        }
        let bridge = Arc::new(Bridge {
            tenant: tenant.name.clone(),
            udid: udid.to_string(),
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            pending: StdMutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            size: StdMutex::new((
                msg["width"].as_u64().unwrap_or(0) as u32,
                msg["height"].as_u64().unwrap_or(0) as u32,
            )),
            latest: StdMutex::new(None),
            alive: AtomicBool::new(true),
            streaming: AtomicBool::new(false),
            watch_until: AtomicI64::new(0),
            last_used: AtomicI64::new(now_ms()),
            events: self.events.clone(),
        });
        tokio::spawn(bridge.clone().read_loop(stdout));
        self.map().insert(key, bridge.clone());
        tracing::info!(tenant = %tenant.name, udid, "simbridge started");
        Ok(bridge)
    }

    /// Stop one simulator's bridge (it shut down or was deleted).
    pub async fn stop(&self, tenant: &str, udid: &str) {
        let bridge = self.map().remove(&format!("{tenant}/{udid}"));
        if let Some(bridge) = bridge {
            bridge.stop().await;
        }
    }

    pub async fn stop_all(&self) {
        let all: Vec<Arc<Bridge>> = self.map().drain().map(|(_, b)| b).collect();
        for bridge in all {
            bridge.stop().await;
        }
    }

    /// Pause streams whose lease lapsed; retire idle and dead bridges.
    pub async fn sweep(&self) {
        let now = now_ms();
        let bridges: Vec<(String, Arc<Bridge>)> = self
            .map()
            .iter()
            .map(|(k, b)| (k.clone(), b.clone()))
            .collect();
        for (key, bridge) in bridges {
            if bridge.is_streaming() && bridge.watch_until.load(Ordering::SeqCst) < now {
                bridge.streaming.store(false, Ordering::SeqCst);
                let _ = bridge.send(json!({ "op": "stream", "fps": 0 })).await;
            }
            let idle = !bridge.is_streaming()
                && bridge.last_used.load(Ordering::Relaxed) < now - IDLE_EXIT_MS;
            if idle || !bridge.is_alive() {
                self.map().remove(&key);
                bridge.stop().await;
            }
        }
    }

    /// The compiled helper, built on first use from the embedded source and
    /// cached by content hash under `<data_dir>/bin`.
    async fn binary(&self, data_root: &Path, developer_dir: &Path) -> Result<PathBuf, String> {
        let mut cached = self.binary.lock().await;
        if let Some(path) = cached.as_ref().filter(|p| p.exists()) {
            return Ok(path.clone());
        }
        let mut hasher = std::hash::DefaultHasher::new();
        SOURCE.hash(&mut hasher);
        let dir = data_root.join("bin");
        let path = dir.join(format!("simbridge-{:016x}", hasher.finish()));
        if !path.exists() {
            tokio::fs::create_dir_all(&dir)
                .await
                .map_err(|e| format!("create {}: {e}", dir.display()))?;
            let source = path.with_extension("swift");
            let tmp = path.with_extension("tmp");
            tokio::fs::write(&source, SOURCE)
                .await
                .map_err(|e| format!("write {}: {e}", source.display()))?;
            tracing::info!(path = %path.display(), "compiling simbridge with xcrun swiftc");
            let out = Command::new("xcrun")
                .env("DEVELOPER_DIR", developer_dir)
                .args(["swiftc", "-O", "-swift-version", "5", "-o"])
                .arg(&tmp)
                .arg(&source)
                .output()
                .await
                .map_err(|e| format!("xcrun swiftc: {e}; install Xcode"))?;
            if !out.status.success() {
                return Err(format!(
                    "compiling simbridge failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                        .lines()
                        .find(|l| l.contains("error"))
                        .unwrap_or("swiftc failed")
                ));
            }
            tokio::fs::rename(&tmp, &path)
                .await
                .map_err(|e| format!("install simbridge: {e}"))?;
        }
        *cached = Some(path.clone());
        Ok(path)
    }
}
