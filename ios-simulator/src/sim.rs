//! The worker's state and every operation behind the `ios-simulator::*`
//! functions.
//!
//! Concurrency model:
//! - lifecycle operations on one simulator (boot, shutdown, erase, delete)
//!   serialize on a per-udid lock, so a boot never races a shutdown;
//! - boot admission (the Mac-wide and per-tenant caps) serializes on one
//!   lock held across count + boot, so two tenants cannot both take the
//!   last slot;
//! - input to one simulator is ordered through its single bridge process;
//! - tenants never share a simulator: each works in its own device set.
//!
//! State is observed, not remembered: a watcher lists every tenant's set
//! every two seconds (and right after each lifecycle call) and turns the
//! differences into `device-changed` events, so simulators booted from Xcode
//! or a terminal show up live too.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use base64::Engine;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::{Mutex, Notify};

use crate::bridge::{now_ms, Bridge, Bridges};
use crate::config::SharedConfig;
use crate::events::{ActivityEvent, DeviceChangedEvent, Events, MediaChangedEvent};
use crate::input;
use crate::simctl::{is_udid, Device, Runtime};
use crate::tenant::{self, Tenant};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Largest slice `media::read` returns: base64 stays inside one websocket frame.
pub const MAX_READ_CHUNK: u64 = 8 * 1024 * 1024;

struct Recording {
    child: Child,
    name: String,
    path: PathBuf,
    started_ms: i64,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct MediaInfo {
    /// `<udid>/<file>`: pass it to `media::read` / `media::delete`.
    pub name: String,
    pub udid: String,
    /// `screenshot` or `recording`.
    pub kind: String,
    pub mime: String,
    pub bytes: u64,
    pub created_ms: i64,
}

pub struct Sim {
    pub config: SharedConfig,
    developer_dir: PathBuf,
    pub bridges: Bridges,
    events: Arc<Events>,
    device_locks: StdMutex<HashMap<String, Arc<Mutex<()>>>>,
    admission: Mutex<()>,
    recordings: Mutex<HashMap<String, Recording>>,
    /// Last observed devices per tenant, for change detection. `None` until
    /// the first poll, which only records the baseline: a worker restart must
    /// not announce every simulator as new.
    seen: Mutex<Option<HashMap<String, HashMap<String, Device>>>>,
    /// Simulators booted from a surface that already shows them.
    quiet_boots: StdMutex<HashSet<String>>,
    poke: Notify,
}

impl Sim {
    pub fn new(
        config: SharedConfig,
        developer_dir: PathBuf,
        bridges: Bridges,
        events: Arc<Events>,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            developer_dir,
            bridges,
            events,
            device_locks: StdMutex::new(HashMap::new()),
            admission: Mutex::new(()),
            recordings: Mutex::new(HashMap::new()),
            seen: Mutex::new(None),
            quiet_boots: StdMutex::new(HashSet::new()),
            poke: Notify::new(),
        })
    }

    pub fn tenant(&self, requested: Option<&str>) -> Result<Tenant, String> {
        let cfg = self.config.load();
        let dev = if cfg.developer_dir.trim().is_empty() {
            self.developer_dir.clone()
        } else {
            iii_worker_paths::resolve_path(cfg.developer_dir.trim())
        };
        tenant::resolve(&cfg, &dev, requested)
    }

    fn data_root(&self) -> PathBuf {
        self.config.load().data_root()
    }

    fn device_lock(&self, udid: &str) -> Arc<Mutex<()>> {
        self.device_locks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(udid.to_string())
            .or_default()
            .clone()
    }

    fn check_udid(udid: &str) -> Result<(), String> {
        if is_udid(udid) {
            Ok(())
        } else {
            Err(format!("invalid udid '{udid}'"))
        }
    }

    /// The device, if it is in this tenant's set.
    pub async fn device(&self, tenant: &Tenant, udid: &str) -> Result<Device, String> {
        Self::check_udid(udid)?;
        tenant.set.device(udid).await
    }

    pub async fn devices(&self, tenant: &Tenant) -> Result<Vec<Device>, String> {
        tenant.set.devices().await
    }

    pub async fn runtimes(&self, tenant: &Tenant) -> Result<Vec<Runtime>, String> {
        tenant.set.runtimes().await
    }

    pub async fn is_recording(&self, tenant: &str, udid: &str) -> bool {
        self.recordings
            .lock()
            .await
            .contains_key(&format!("{tenant}/{udid}"))
    }

    // ── lifecycle ───────────────────────────────────────────────────────

    pub async fn create(
        &self,
        tenant: &Tenant,
        name: &str,
        device_type: &str,
        runtime: Option<&str>,
    ) -> Result<Device, String> {
        let name = name.trim();
        if name.is_empty() || name.len() > 64 || name.chars().any(char::is_control) {
            return Err("name must be 1-64 printable characters".into());
        }
        let cap = self.config.load().max_devices_per_tenant as usize;
        // Admission lock: two concurrent creates cannot both take the last slot.
        let _admit = self.admission.lock().await;
        if let Some(path) = &tenant.set.path {
            tokio::fs::create_dir_all(path)
                .await
                .map_err(|e| format!("create device set: {e}"))?;
        }
        if self.devices(tenant).await?.len() >= cap {
            return Err(format!(
                "tenant '{}' already owns {cap} simulators; delete one first",
                tenant.name
            ));
        }
        let mut args = vec!["create", name, device_type];
        if let Some(runtime) = runtime.filter(|r| !r.is_empty()) {
            args.push(runtime);
        }
        let udid = tenant.set.run(&args).await?.trim().to_string();
        self.poke.notify_one();
        self.device(tenant, &udid).await
    }

    pub async fn delete(&self, tenant: &Tenant, udid: &str) -> Result<(), String> {
        let device = self.device(tenant, udid).await?;
        let lock = self.device_lock(udid);
        let _guard = lock.lock().await;
        self.release(&tenant.name, udid).await;
        if device.state != "Shutdown" {
            let _ = tenant.set.run(&["shutdown", udid]).await;
        }
        tenant.set.run(&["delete", udid]).await?;
        let media = tenant.media_dir(udid);
        if media.is_dir() {
            let _ = tokio::fs::remove_dir_all(media).await;
        }
        self.poke.notify_one();
        Ok(())
    }

    /// `preview: false` marks the boot's `device-changed` event so live
    /// previews skip it (the caller is already showing the simulator).
    pub async fn boot(&self, tenant: &Tenant, udid: &str, preview: bool) -> Result<Device, String> {
        let device = self.device(tenant, udid).await?;
        if device.state == "Booted" {
            return Ok(device);
        }
        if !preview {
            self.quiet_boots
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(udid.to_string());
        }
        let lock = self.device_lock(udid);
        let _guard = lock.lock().await;
        {
            let _admit = self.admission.lock().await;
            let cfg = self.config.load_full();
            let (total, mine) = self.booted_counts(&tenant.name).await;
            if total >= cfg.max_booted as usize {
                return Err(format!(
                    "this Mac already runs {total} simulators (max_booted {}); shut one down first",
                    cfg.max_booted
                ));
            }
            if mine >= cfg.max_booted_per_tenant as usize {
                return Err(format!(
                    "tenant '{}' already runs {mine} simulators (max_booted_per_tenant {})",
                    tenant.name, cfg.max_booted_per_tenant
                ));
            }
            if let Err(e) = tenant.set.run(&["boot", udid]).await {
                // Booted meanwhile by someone else (Xcode): that is success.
                if !e.contains("current state: Booted") {
                    return Err(e);
                }
            }
        }
        self.poke.notify_one();
        self.device(tenant, udid).await
    }

    pub async fn shutdown(&self, tenant: &Tenant, udid: &str) -> Result<Device, String> {
        let device = self.device(tenant, udid).await?;
        let lock = self.device_lock(udid);
        let _guard = lock.lock().await;
        self.release(&tenant.name, udid).await;
        if device.state != "Shutdown" {
            if let Err(e) = tenant.set.run(&["shutdown", udid]).await {
                if !e.contains("current state: Shutdown") {
                    return Err(e);
                }
            }
        }
        self.poke.notify_one();
        self.device(tenant, udid).await
    }

    pub async fn erase(&self, tenant: &Tenant, udid: &str) -> Result<Device, String> {
        let device = self.device(tenant, udid).await?;
        if device.state != "Shutdown" {
            return Err("shut the simulator down before erasing it".into());
        }
        let lock = self.device_lock(udid);
        let _guard = lock.lock().await;
        tenant.set.run(&["erase", udid]).await?;
        self.poke.notify_one();
        self.device(tenant, udid).await
    }

    /// (booted on this Mac across every visible set, booted by `tenant`).
    async fn booted_counts(&self, tenant: &str) -> (usize, usize) {
        let mut total = 0;
        let mut mine = 0;
        for name in tenant::known(&self.config.load()) {
            let Ok(t) = self.tenant(Some(&name)) else {
                continue;
            };
            let booted = t
                .set
                .devices()
                .await
                .unwrap_or_default()
                .iter()
                .filter(|d| d.state == "Booted" || d.state == "Booting")
                .count();
            total += booted;
            if name == tenant {
                mine = booted;
            }
        }
        (total, mine)
    }

    /// Stop the bridge and any recording of a simulator going away.
    async fn release(&self, tenant: &str, udid: &str) {
        self.bridges.stop(tenant, udid).await;
        let recording = self
            .recordings
            .lock()
            .await
            .remove(&format!("{tenant}/{udid}"));
        if let Some(recording) = recording {
            let _ = self.finish_recording(tenant, udid, recording).await;
        }
    }

    // ── input ───────────────────────────────────────────────────────────

    /// The simulator's bridge, started on demand. Needs a booted device.
    pub async fn bridge(&self, tenant: &Tenant, udid: &str) -> Result<Arc<Bridge>, String> {
        Self::check_udid(udid)?;
        if let Some(bridge) = self.bridges.find(&tenant.name, udid) {
            return Ok(bridge);
        }
        let device = self.device(tenant, udid).await?;
        if device.state != "Booted" {
            return Err(format!(
                "simulator '{}' is {}; boot it first",
                device.name, device.state
            ));
        }
        self.bridges
            .get_or_start(tenant, udid, &self.data_root())
            .await
    }

    /// Type text: key events for US-keyboard characters, paste otherwise.
    pub async fn type_text(&self, tenant: &Tenant, udid: &str, text: &str) -> Result<(), String> {
        let bridge = self.bridge(tenant, udid).await?;
        match input::type_commands(text) {
            Some(cmds) => {
                bridge
                    .play(cmds.into_iter().map(|c| (c, 0)).collect())
                    .await
            }
            None => {
                self.pbcopy(tenant, udid, text).await?;
                bridge
                    .play(
                        input::chord_commands(&["cmd".into(), "v".into()])?
                            .into_iter()
                            .map(|c| (c, 0))
                            .collect(),
                    )
                    .await
            }
        }
    }

    async fn pbcopy(&self, tenant: &Tenant, udid: &str, text: &str) -> Result<(), String> {
        use tokio::io::AsyncWriteExt;
        let mut args = vec!["simctl".to_string()];
        if let Some(path) = &tenant.set.path {
            args.push("--set".into());
            args.push(path.to_string_lossy().into_owned());
        }
        args.extend(["pbcopy".into(), udid.to_string()]);
        let mut child = tokio::process::Command::new("xcrun")
            .env("DEVELOPER_DIR", &tenant.set.developer_dir)
            .args(&args)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("simctl pbcopy: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .await
                .map_err(|e| format!("simctl pbcopy: {e}"))?;
        }
        let status = child.wait().await.map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err("simctl pbcopy failed".into())
        }
    }

    // ── media ───────────────────────────────────────────────────────────

    fn media_name(udid: &str, kind: &str, ext: &str) -> String {
        format!("{udid}/{kind}-{}.{ext}", now_ms())
    }

    /// Save a PNG screenshot; returns its info and a small JPEG preview.
    pub async fn screenshot(
        &self,
        tenant: &Tenant,
        udid: &str,
        preview: bool,
    ) -> Result<Shot, String> {
        let device = self.device(tenant, udid).await?;
        if device.state != "Booted" {
            return Err(format!(
                "simulator '{}' is {}; boot it first",
                device.name, device.state
            ));
        }
        let name = Self::media_name(udid, "screenshot", "png");
        let path = tenant.media_path(&name)?;
        tokio::fs::create_dir_all(tenant.media_dir(udid))
            .await
            .map_err(|e| format!("create media folder: {e}"))?;
        let path_str = path.to_string_lossy().into_owned();
        tenant
            .set
            .run(&["io", udid, "screenshot", "--type=png", &path_str])
            .await?;
        let png = tokio::fs::read(&path)
            .await
            .map_err(|e| format!("read screenshot: {e}"))?;
        let (width, height) = png_size(&png).unwrap_or((0, 0));
        let preview = if preview {
            Some(jpeg_preview(&path, width.max(height)).await?)
        } else {
            None
        };
        let info = MediaInfo {
            name: name.clone(),
            udid: udid.to_string(),
            kind: "screenshot".into(),
            mime: "image/png".into(),
            bytes: png.len() as u64,
            created_ms: now_ms(),
        };
        self.media_event(&tenant.name, udid, &name, "screenshot", "added");
        Ok(Shot {
            media: info,
            device: device.name,
            width,
            height,
            preview,
        })
    }

    pub async fn recording_start(&self, tenant: &Tenant, udid: &str) -> Result<String, String> {
        let device = self.device(tenant, udid).await?;
        if device.state != "Booted" {
            return Err(format!(
                "simulator '{}' is {}; boot it first",
                device.name, device.state
            ));
        }
        let key = format!("{}/{udid}", tenant.name);
        if self.recordings.lock().await.contains_key(&key) {
            return Err("this simulator is already recording".into());
        }
        let name = Self::media_name(udid, "recording", "mov");
        let path = tenant.media_path(&name)?;
        tokio::fs::create_dir_all(tenant.media_dir(udid))
            .await
            .map_err(|e| format!("create media folder: {e}"))?;
        let path_str = path.to_string_lossy().into_owned();
        let mut child = tenant.set.spawn(&[
            "io",
            udid,
            "recordVideo",
            "--codec=h264",
            "--mask=black",
            "--force",
            &path_str,
        ])?;
        // simctl prints "Recording started" once the first frame is in.
        let stderr = child.stderr.take().ok_or("recordVideo stderr")?;
        let mut lines = BufReader::new(stderr).lines();
        let started = tokio::time::timeout(Duration::from_secs(15), async {
            let mut last = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.contains("Recording started") {
                    return Ok(());
                }
                last = line;
            }
            Err(if last.is_empty() {
                "recordVideo exited".to_string()
            } else {
                last
            })
        })
        .await;
        match started {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(format!("recording failed: {e}")),
            Err(_) => {
                let _ = child.kill().await;
                return Err("recording did not start in 15s".into());
            }
        }
        // Keep draining stderr so simctl never blocks on a full pipe.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        let mut recordings = self.recordings.lock().await;
        if recordings.contains_key(&key) {
            // A concurrent start won; drop ours.
            let _ = child.kill().await;
            let _ = tokio::fs::remove_file(&path).await;
            return Err("this simulator is already recording".into());
        }
        recordings.insert(
            key,
            Recording {
                child,
                name: name.clone(),
                path,
                started_ms: now_ms(),
            },
        );
        Ok(name)
    }

    pub async fn recording_stop(
        &self,
        tenant: &Tenant,
        udid: &str,
    ) -> Result<Option<(MediaInfo, i64)>, String> {
        Self::check_udid(udid)?;
        let recording = self
            .recordings
            .lock()
            .await
            .remove(&format!("{}/{udid}", tenant.name));
        match recording {
            Some(recording) => self
                .finish_recording(&tenant.name, udid, recording)
                .await
                .map(Some),
            None => Ok(None),
        }
    }

    /// SIGINT is simctl's "stop and finalize the movie".
    async fn finish_recording(
        &self,
        tenant: &str,
        udid: &str,
        mut recording: Recording,
    ) -> Result<(MediaInfo, i64), String> {
        if let Some(pid) = recording.child.id() {
            // SAFETY: plain kill(2) on our own child's pid.
            unsafe { libc::kill(pid as i32, libc::SIGINT) };
        }
        if tokio::time::timeout(Duration::from_secs(30), recording.child.wait())
            .await
            .is_err()
        {
            let _ = recording.child.kill().await;
        }
        let duration_ms = now_ms() - recording.started_ms;
        let bytes = tokio::fs::metadata(&recording.path)
            .await
            .map(|m| m.len())
            .map_err(|_| "the recording produced no file".to_string())?;
        self.media_event(tenant, udid, &recording.name, "recording", "added");
        Ok((
            MediaInfo {
                name: recording.name,
                udid: udid.to_string(),
                kind: "recording".into(),
                mime: "video/quicktime".into(),
                bytes,
                created_ms: recording.started_ms,
            },
            duration_ms,
        ))
    }

    pub async fn media_list(
        &self,
        tenant: &Tenant,
        udid: Option<&str>,
    ) -> Result<Vec<MediaInfo>, String> {
        if let Some(udid) = udid {
            Self::check_udid(udid)?;
        }
        let recording: Vec<String> = self
            .recordings
            .lock()
            .await
            .values()
            .map(|r| r.name.clone())
            .collect();
        let mut out = Vec::new();
        let Ok(mut dirs) = tokio::fs::read_dir(tenant.root.join("media")).await else {
            return Ok(out);
        };
        while let Ok(Some(dir)) = dirs.next_entry().await {
            let dev = dir.file_name().to_string_lossy().into_owned();
            if !is_udid(&dev) || udid.is_some_and(|u| u != dev) {
                continue;
            }
            let Ok(mut files) = tokio::fs::read_dir(dir.path()).await else {
                continue;
            };
            while let Ok(Some(file)) = files.next_entry().await {
                let name = format!("{dev}/{}", file.file_name().to_string_lossy());
                if tenant.media_path(&name).is_err() || recording.contains(&name) {
                    continue;
                }
                let Ok(meta) = file.metadata().await else {
                    continue;
                };
                let (kind, mime) = if name.ends_with(".png") {
                    ("screenshot", "image/png")
                } else {
                    ("recording", "video/quicktime")
                };
                out.push(MediaInfo {
                    udid: dev.clone(),
                    kind: kind.into(),
                    mime: mime.into(),
                    bytes: meta.len(),
                    created_ms: created_ms(&name).unwrap_or(0),
                    name,
                });
            }
        }
        out.sort_by_key(|m| std::cmp::Reverse(m.created_ms));
        Ok(out)
    }

    /// One base64 slice of a media file: (data, bytes read, total bytes).
    pub async fn media_read(
        &self,
        tenant: &Tenant,
        name: &str,
        offset: u64,
        length: u64,
    ) -> Result<(String, u64, u64), String> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        let path = tenant.media_path(name)?;
        let mut file = tokio::fs::File::open(&path)
            .await
            .map_err(|_| format!("no media named '{name}'"))?;
        let total = file.metadata().await.map_err(|e| e.to_string())?.len();
        let length = length
            .clamp(1, MAX_READ_CHUNK)
            .min(total.saturating_sub(offset));
        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(|e| e.to_string())?;
        let mut buf = vec![0; length as usize];
        file.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
        Ok((
            base64::engine::general_purpose::STANDARD.encode(buf),
            length,
            total,
        ))
    }

    pub async fn media_delete(&self, tenant: &Tenant, name: &str) -> Result<bool, String> {
        let path = tenant.media_path(name)?;
        let removed = tokio::fs::remove_file(&path).await.is_ok();
        if removed {
            let (udid, _) = name.split_once('/').unwrap_or_default();
            let kind = if name.ends_with(".png") {
                "screenshot"
            } else {
                "recording"
            };
            self.media_event(&tenant.name, udid, name, kind, "removed");
        }
        Ok(removed)
    }

    /// Tell live previews that `function` is driving this simulator.
    pub fn used(&self, tenant: &str, udid: &str, function: &str) {
        self.events.activity(ActivityEvent {
            tenant: tenant.into(),
            udid: udid.into(),
            function: function.into(),
            timestamp: now_ms(),
        });
    }

    fn media_event(&self, tenant: &str, udid: &str, name: &str, kind: &str, change: &str) {
        self.events.media(MediaChangedEvent {
            tenant: tenant.into(),
            udid: udid.into(),
            name: name.into(),
            kind: kind.into(),
            change: change.into(),
            timestamp: now_ms(),
        });
    }

    // ── background work ─────────────────────────────────────────────────

    /// Watch every tenant's set; emit changes; release what went away.
    pub async fn watch_loop(self: Arc<Self>) {
        loop {
            self.poll().await;
            tokio::select! {
                _ = tokio::time::sleep(POLL_INTERVAL) => {}
                _ = self.poke.notified() => {}
            }
        }
    }

    // ponytail: lists every tenant's set each tick (one simctl per tenant);
    // switch to CoreSimulator device notifications if tenants reach dozens.
    async fn poll(&self) {
        let cfg = self.config.load_full();
        let mut seen = self.seen.lock().await;
        let baseline = seen.is_none();
        let seen = seen.get_or_insert_with(HashMap::new);
        for name in tenant::known(&cfg) {
            let Ok(t) = self.tenant(Some(&name)) else {
                continue;
            };
            let Ok(devices) = t.set.devices().await else {
                continue;
            };
            let now: HashMap<String, Device> =
                devices.into_iter().map(|d| (d.udid.clone(), d)).collect();
            let before = seen.insert(name.clone(), now.clone()).unwrap_or_default();
            if baseline {
                continue;
            }
            for (udid, device) in &now {
                let (change, previous_state) = match before.get(udid) {
                    None => ("added", String::new()),
                    Some(old) if old != device => ("updated", old.state.clone()),
                    _ => continue,
                };
                if device.state != "Booted" {
                    self.release(&name, udid).await;
                }
                let quiet = device.state == "Booted"
                    && self
                        .quiet_boots
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(udid);
                self.events.device(DeviceChangedEvent {
                    tenant: name.clone(),
                    udid: udid.clone(),
                    name: device.name.clone(),
                    state: device.state.clone(),
                    previous_state,
                    change: change.into(),
                    preview: !quiet,
                    timestamp: now_ms(),
                });
            }
            for (udid, device) in before.iter().filter(|(u, _)| !now.contains_key(*u)) {
                self.release(&name, udid).await;
                self.events.device(DeviceChangedEvent {
                    tenant: name.clone(),
                    udid: udid.clone(),
                    name: device.name.clone(),
                    state: "Deleted".into(),
                    previous_state: device.state.clone(),
                    change: "removed".into(),
                    preview: true,
                    timestamp: now_ms(),
                });
            }
        }
    }

    /// Stop over-long recordings; retire idle bridges.
    pub async fn sweep_loop(self: Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            self.bridges.sweep().await;
            let max_ms = self.config.load().max_recording_seconds as i64 * 1000;
            let expired: Vec<(String, Recording)> = {
                let mut recordings = self.recordings.lock().await;
                let keys: Vec<String> = recordings
                    .iter()
                    .filter(|(_, r)| now_ms() - r.started_ms > max_ms)
                    .map(|(k, _)| k.clone())
                    .collect();
                keys.into_iter()
                    .filter_map(|k| recordings.remove(&k).map(|r| (k, r)))
                    .collect()
            };
            for (key, recording) in expired {
                let (tenant, udid) = key.split_once('/').unwrap_or_default();
                tracing::info!(tenant, udid, "recording reached max_recording_seconds");
                let _ = self.finish_recording(tenant, udid, recording).await;
            }
        }
    }

    pub async fn shutdown_all(&self) {
        let recordings: Vec<(String, Recording)> = self.recordings.lock().await.drain().collect();
        for (key, recording) in recordings {
            let (tenant, udid) = key.split_once('/').unwrap_or_default();
            let _ = self.finish_recording(tenant, udid, recording).await;
        }
        self.bridges.stop_all().await;
    }
}

/// Width/height from a PNG's IHDR.
fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    let w = png.get(16..20)?;
    let h = png.get(20..24)?;
    Some((
        u32::from_be_bytes(w.try_into().ok()?),
        u32::from_be_bytes(h.try_into().ok()?),
    ))
}

/// `<kind>-<ms>.<ext>` → ms.
fn created_ms(name: &str) -> Option<i64> {
    let file = name.rsplit('/').next()?;
    let stem = file.split('.').next()?;
    stem.rsplit('-').next()?.parse().ok()
}

/// Base64 bytes a screenshot preview may take. The harness caps a whole
/// result at 256 KB and counts the image twice (content and details).
const PREVIEW_BUDGET: usize = 96 * 1024;

pub struct Shot {
    pub media: MediaInfo,
    /// The simulator's name.
    pub device: String,
    /// Screenshot pixels: the coordinate space of touch.
    pub width: u32,
    pub height: u32,
    pub preview: Option<Preview>,
}

pub struct Preview {
    /// Base64 JPEG.
    pub data: String,
    /// Longest edge, in pixels.
    pub edge: u32,
}

/// A JPEG of the screenshot for the model, via macOS `sips`: the largest
/// (longest edge, quality) step whose base64 fits [`PREVIEW_BUDGET`].
async fn jpeg_preview(png: &Path, longest: u32) -> Result<Preview, String> {
    const STEPS: [(u32, u32); 4] = [(1024, 60), (900, 55), (800, 50), (640, 45)];
    let out = png.with_extension("preview.jpg");
    let mut result = Err("could not encode the screenshot preview".to_string());
    for (edge, quality) in STEPS {
        let edge = edge.min(longest.max(1));
        let status = tokio::process::Command::new("sips")
            .args(["-s", "format", "jpeg", "-s", "formatOptions"])
            .arg(quality.to_string())
            .arg("-Z")
            .arg(edge.to_string())
            .arg(png)
            .arg("--out")
            .arg(&out)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map_err(|e| format!("sips: {e}"))?;
        if !status.success() {
            break;
        }
        let Ok(bytes) = tokio::fs::read(&out).await else {
            break;
        };
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        let fits = data.len() <= PREVIEW_BUDGET;
        result = Ok(Preview { data, edge });
        if fits {
            break;
        }
    }
    let _ = tokio::fs::remove_file(&out).await;
    result
}

pub fn content_blocks(image_b64: Option<String>, text: String) -> Value {
    let mut blocks = Vec::new();
    if let Some(data) = image_b64 {
        blocks.push(json!({ "type": "image", "mime": "image/jpeg", "data": data }));
    }
    blocks.push(json!({ "type": "text", "text": text }));
    Value::Array(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_size_reads_ihdr() {
        let mut png = vec![0u8; 24];
        png[16..20].copy_from_slice(&1206u32.to_be_bytes());
        png[20..24].copy_from_slice(&2622u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((1206, 2622)));
        assert_eq!(png_size(&[0; 8]), None);
    }

    #[test]
    fn media_names_carry_their_timestamp() {
        assert_eq!(
            created_ms("5FEFD8EB-AB2D-4B6A-A5AB-6604312D1993/recording-1790532937493.mov"),
            Some(1790532937493)
        );
    }
}
