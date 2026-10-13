//! `browser::chromium::install`: download Chrome for Testing into the
//! install dir in the background, reporting progress as
//! `browser::chromium-install-progress` events.
//!
//! Steps: read the Chrome for Testing "last known good" manifest, take the
//! Stable channel's `chrome` build for this platform, stream the zip to a
//! `.partial` file, unpack it into a staging folder, run the binary with
//! `--version` (missing shared libraries show up here), then rename the
//! staging folder to `<install dir>/<version>/` and point `current.json` at
//! it. Nothing outside the install dir is touched, and a Chrome the person
//! installs later still wins (the resolver puts system installs first).
//!
//! One job per worker process (a second call while one runs returns that
//! job), plus a lock file in the install dir so two workers sharing a cache
//! never unpack into each other.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{cft_executable, is_executable, read_current, write_current, CurrentInstall};
use crate::events::{Emitter, EventKind};
use crate::session::now_ms;

/// Chrome for Testing's "last known good" versions, with download URLs.
pub const MANIFEST_URL: &str = "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";
/// Points the installer at a mirror of [`MANIFEST_URL`].
pub const MANIFEST_URL_ENV: &str = "III_BROWSER_CFT_MANIFEST_URL";
const LOCK_FILE: &str = ".install.lock";
/// A lock older than this is a leftover from a crashed install.
const STALE_LOCK: Duration = Duration::from_secs(2 * 60 * 60);
/// No bytes for this long and the download is declared stalled.
const CHUNK_TIMEOUT: Duration = Duration::from_secs(60);
const MANIFEST_TIMEOUT: Duration = Duration::from_secs(30);
/// First start of a fresh binary can be slow (disk cache, Gatekeeper).
const VERIFY_TIMEOUT: Duration = Duration::from_secs(60);
/// Progress events are spaced at least this far apart...
const EMIT_INTERVAL: Duration = Duration::from_millis(250);
/// ...and, without a known total, at least this many bytes apart.
const EMIT_BYTES_UNKNOWN_TOTAL: u64 = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// Reading the Chrome for Testing manifest.
    Resolving,
    /// Streaming the zip; `bytes_done`/`bytes_total` are download bytes.
    Downloading,
    /// Unpacking; `bytes_done`/`bytes_total` are unpacked bytes.
    Extracting,
    /// Running the new binary with `--version`.
    Verifying,
    /// Installed; `path` and `version` are set.
    Done,
    /// Stopped; `error` (and maybe `hint`) say why.
    Failed,
}

impl Phase {
    pub fn is_terminal(self) -> bool {
        matches!(self, Phase::Done | Phase::Failed)
    }
}

/// `browser::chromium-install-progress` — one install job moved: a new
/// phase, or about 1 % more bytes (at most every 250 ms). Also the `job`
/// field of `browser::chromium::status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InstallProgress {
    pub job_id: String,
    pub phase: Phase,
    /// Chrome for Testing version being installed, once known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Bytes downloaded (downloading) or unpacked (extracting) so far.
    pub bytes_done: u64,
    /// Total for `bytes_done`, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_total: Option<u64>,
    /// The installed binary (done).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// What went wrong (failed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// How to fix it, when known (failed) — e.g. the system packages a
    /// Linux machine is missing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    pub timestamp: i64,
}

impl InstallProgress {
    fn new(job_id: String) -> Self {
        Self {
            job_id,
            phase: Phase::Resolving,
            version: None,
            bytes_done: 0,
            bytes_total: None,
            path: None,
            error: None,
            hint: None,
            timestamp: now_ms(),
        }
    }
}

/// The latest state of this process's install job (running or finished).
static JOB: Mutex<Option<InstallProgress>> = Mutex::new(None);

fn job_slot() -> std::sync::MutexGuard<'static, Option<InstallProgress>> {
    JOB.lock().unwrap_or_else(|p| p.into_inner())
}

/// This process's most recent install job, if any.
pub fn latest_job() -> Option<InstallProgress> {
    job_slot().clone()
}

/// What [`start`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    /// A new job is running in the background.
    Started(String),
    /// A job was already running; this is its id.
    Running(String),
}

/// Start an install job for `platform` (a Chrome for Testing platform), or
/// return the one already running. Returns at once; progress arrives as
/// events and in [`latest_job`].
pub fn start(emitter: Arc<Emitter>, platform: &'static str, force: bool) -> Start {
    let mut slot = job_slot();
    if let Some(job) = slot.as_ref().filter(|j| !j.phase.is_terminal()) {
        return Start::Running(job.job_id.clone());
    }
    let simple = uuid::Uuid::new_v4().simple().to_string();
    let job_id = format!("chromium-{}", &simple[..12]);
    let first = InstallProgress::new(job_id.clone());
    *slot = Some(first.clone());
    drop(slot);
    let reporter = Reporter {
        emitter,
        state: first,
        last_emit: None,
        last_emit_bytes: 0,
    };
    tokio::spawn(run(reporter, super::install_dir(), platform, force));
    Start::Started(job_id)
}

/// Whether a byte-progress update is worth an event: at least
/// [`EMIT_INTERVAL`] since the last one, and about 1 % of the total (1 MiB
/// when the total is unknown) more bytes.
pub fn should_emit(since_last: Duration, last_bytes: u64, bytes: u64, total: Option<u64>) -> bool {
    if since_last < EMIT_INTERVAL {
        return false;
    }
    let step = match total {
        Some(total) if total > 0 => (total / 100).max(1),
        _ => EMIT_BYTES_UNKNOWN_TOTAL,
    };
    bytes.saturating_sub(last_bytes) >= step
}

/// Keeps the job slot current and paces the events.
struct Reporter {
    emitter: Arc<Emitter>,
    state: InstallProgress,
    last_emit: Option<Instant>,
    last_emit_bytes: u64,
}

impl Reporter {
    fn store(&mut self) {
        self.state.timestamp = now_ms();
        *job_slot() = Some(self.state.clone());
    }

    async fn publish(&mut self) {
        self.store();
        self.last_emit = Some(Instant::now());
        self.last_emit_bytes = self.state.bytes_done;
        // Not a session event: empty session id, so only unfiltered
        // bindings receive it.
        self.emitter
            .emit(EventKind::ChromiumInstallProgress, "", &self.state)
            .await;
    }

    /// Enter `phase`; always an event.
    async fn phase(&mut self, phase: Phase) {
        self.state.phase = phase;
        self.state.bytes_done = 0;
        self.state.bytes_total = None;
        self.publish().await;
    }

    /// Byte progress within the current phase; an event when it moved
    /// enough (see [`should_emit`]).
    async fn bytes(&mut self, done: u64, total: Option<u64>) {
        self.state.bytes_done = done;
        self.state.bytes_total = total;
        let since = self.last_emit.map(|t| t.elapsed()).unwrap_or(Duration::MAX);
        if should_emit(since, self.last_emit_bytes, done, total) {
            self.publish().await;
        } else {
            self.store();
        }
    }
}

/// Why a job failed, plus how to fix it when we know.
#[derive(Debug)]
struct Failure {
    error: String,
    hint: Option<String>,
}

impl From<String> for Failure {
    fn from(error: String) -> Self {
        Self { error, hint: None }
    }
}

async fn run(mut rep: Reporter, dir: PathBuf, platform: &'static str, force: bool) {
    rep.publish().await;
    match install(&mut rep, &dir, platform, force).await {
        Ok(path) => {
            rep.state.path = Some(path.display().to_string());
            rep.phase(Phase::Done).await;
            tracing::info!(
                version = rep.state.version.as_deref().unwrap_or(""),
                path = %path.display(),
                "chromium installed"
            );
        }
        Err(failure) => {
            tracing::warn!(error = %failure.error, "chromium install failed");
            rep.state.error = Some(failure.error);
            rep.state.hint = failure.hint;
            rep.phase(Phase::Failed).await;
        }
    }
}

/// Removes what a failed job leaves behind (the partial download, the
/// staging folder) unless disarmed.
struct Cleanup(Vec<PathBuf>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        for path in &self.0 {
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(path);
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

async fn install(
    rep: &mut Reporter,
    dir: &Path,
    platform: &'static str,
    force: bool,
) -> Result<PathBuf, Failure> {
    let rel = cft_executable(platform)
        .ok_or_else(|| format!("no Chrome for Testing build for platform {platform}"))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let _lock = InstallLock::acquire(dir)?;
    let client = http_client()?;

    let manifest_url = std::env::var(MANIFEST_URL_ENV).unwrap_or_else(|_| MANIFEST_URL.to_string());
    let manifest = fetch_manifest(&client, &manifest_url).await?;
    let pick = pick_download(&manifest, platform)?;
    rep.state.version = Some(pick.version.clone());
    if !force {
        let installed = read_current(dir)
            .filter(|c| c.version == pick.version)
            .and_then(|c| c.executable_path(dir))
            .filter(|p| is_executable(p));
        if let Some(path) = installed {
            return Ok(path);
        }
    }

    rep.phase(Phase::Downloading).await;
    let partial = dir.join(format!(".chrome-{}-{platform}.zip.partial", pick.version));
    let zip_path = dir.join(format!(".chrome-{}-{platform}.zip", pick.version));
    let staging = dir.join(format!(".staging-{}", rep.state.job_id));
    let mut cleanup = Cleanup(vec![partial.clone(), zip_path.clone(), staging.clone()]);
    download(&client, &pick.url, &partial, rep).await?;
    tokio::fs::rename(&partial, &zip_path)
        .await
        .map_err(|e| format!("cannot finish the download: {e}"))?;

    rep.phase(Phase::Extracting).await;
    let _ = std::fs::remove_dir_all(&staging);
    extract(&zip_path, &staging, rep).await?;

    rep.phase(Phase::Verifying).await;
    let staged_exe = staging.join(rel);
    verify(&staged_exe, &staging).await?;

    // Commit: the staging folder becomes `<dir>/<version>/` in one rename.
    let dest = dir.join(&pick.version);
    if dest.exists() {
        let old = dir.join(format!(".old-{}", rep.state.job_id));
        std::fs::rename(&dest, &old)
            .map_err(|e| format!("cannot replace {}: {e}", dest.display()))?;
        cleanup.0.push(old);
    }
    std::fs::rename(&staging, &dest)
        .map_err(|e| format!("cannot move Chromium into {}: {e}", dest.display()))?;
    write_current(
        dir,
        &CurrentInstall {
            version: pick.version.clone(),
            platform: platform.to_string(),
            executable: rel.to_string(),
            installed_at: now_ms(),
        },
    )
    .map_err(|e| {
        format!(
            "cannot write {}: {e}",
            dir.join(super::CURRENT_FILE).display()
        )
    })?;
    // The zip, the staging folder (now renamed away) and any replaced
    // version go; dropping `cleanup` removes whatever is still there.
    drop(cleanup);
    Ok(dest.join(rel))
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Manifest {
    channels: std::collections::HashMap<String, Channel>,
}

#[derive(Deserialize)]
struct Channel {
    version: String,
    #[serde(default)]
    downloads: Downloads,
}

#[derive(Deserialize, Default)]
struct Downloads {
    #[serde(default)]
    chrome: Vec<Download>,
}

#[derive(Deserialize)]
struct Download {
    platform: String,
    url: String,
}

/// The build to install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    pub version: String,
    pub url: String,
}

/// The Stable channel's full `chrome` build (not the headless shell) for
/// `platform`, from the "last known good versions with downloads" JSON.
pub fn pick_download(manifest: &str, platform: &str) -> Result<Pick, String> {
    let manifest: Manifest = serde_json::from_str(manifest)
        .map_err(|e| format!("the Chrome for Testing manifest is not what we expect: {e}"))?;
    let stable = manifest
        .channels
        .get("Stable")
        .ok_or("the Chrome for Testing manifest has no Stable channel")?;
    let valid_version = !stable.version.is_empty()
        && stable
            .version
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.')
        && !stable.version.starts_with('.');
    if !valid_version {
        return Err(format!(
            "the Chrome for Testing manifest names an odd version '{}'",
            stable.version
        ));
    }
    let download = stable
        .downloads
        .chrome
        .iter()
        .find(|d| d.platform == platform)
        .ok_or_else(|| format!("Chrome for Testing has no {platform} build of Stable"))?;
    if !download.url.starts_with("https://") {
        return Err(format!(
            "refusing a non-https Chrome for Testing download: {}",
            download.url
        ));
    }
    Ok(Pick {
        version: stable.version.clone(),
        url: download.url.clone(),
    })
}

// ---------------------------------------------------------------------------
// Network — a plain client: this is the worker fetching its own dependency
// from a fixed public host, not a page fetch, so the SSRF-guarded fetch
// client does not apply.
// ---------------------------------------------------------------------------

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("iii-browser-worker/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("cannot build an HTTP client: {e}"))
}

async fn fetch_manifest(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let response = client
        .get(url)
        .timeout(MANIFEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("cannot reach Chrome for Testing ({url}): {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Chrome for Testing answered HTTP {} for {url}",
            response.status()
        ));
    }
    response
        .text()
        .await
        .map_err(|e| format!("cannot read the Chrome for Testing manifest: {e}"))
}

async fn download(
    client: &reqwest::Client,
    url: &str,
    partial: &Path,
    rep: &mut Reporter,
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;

    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("download failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "download failed: HTTP {} from {url}",
            response.status()
        ));
    }
    let total = response.content_length();
    let mut file = tokio::fs::File::create(partial)
        .await
        .map_err(|e| format!("cannot write {}: {e}", partial.display()))?;
    let mut done = 0u64;
    rep.bytes(0, total).await;
    loop {
        let chunk = match tokio::time::timeout(CHUNK_TIMEOUT, response.chunk()).await {
            Err(_) => {
                return Err(format!(
                    "download stalled: no data for {} s",
                    CHUNK_TIMEOUT.as_secs()
                ))
            }
            Ok(Err(e)) => return Err(format!("download interrupted: {e}")),
            Ok(Ok(None)) => break,
            Ok(Ok(Some(chunk))) => chunk,
        };
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("cannot write {}: {e}", partial.display()))?;
        done += chunk.len() as u64;
        rep.bytes(done, total).await;
    }
    file.flush()
        .await
        .map_err(|e| format!("cannot write {}: {e}", partial.display()))?;
    if let Some(total) = total {
        if done != total {
            return Err(format!("download incomplete: got {done} of {total} bytes"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Unpacking
// ---------------------------------------------------------------------------

/// macOS: `ditto` keeps the `.app` bundle's symlinks and signature intact.
#[cfg(target_os = "macos")]
async fn extract(zip: &Path, dest: &Path, _rep: &mut Reporter) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    let output = tokio::process::Command::new("/usr/bin/ditto")
        .arg("-x")
        .arg("-k")
        .arg(zip)
        .arg(dest)
        .output()
        .await
        .map_err(|e| format!("cannot run ditto: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "unpacking failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
async fn extract(zip: &Path, dest: &Path, rep: &mut Reporter) -> Result<(), String> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(u64, u64)>();
    let (zip, dest) = (zip.to_path_buf(), dest.to_path_buf());
    let task = tokio::task::spawn_blocking(move || {
        extract_zip(&zip, &dest, |done, total| {
            let _ = tx.send((done, total));
        })
    });
    // Ends when the blocking task drops its sender.
    while let Some((done, total)) = rx.recv().await {
        rep.bytes(done, Some(total)).await;
    }
    task.await.map_err(|e| format!("unpacking stopped: {e}"))?
}

/// Unpack `zip` into `dest`, keeping Unix modes (and relative symlinks that
/// stay inside `dest`). `progress(done, total)` gets unpacked bytes after
/// each file. Blocking.
pub fn extract_zip(
    zip: &Path,
    dest: &Path,
    mut progress: impl FnMut(u64, u64),
) -> Result<(), String> {
    let file =
        std::fs::File::open(zip).map_err(|e| format!("cannot open {}: {e}", zip.display()))?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|e| format!("the download is not a valid zip: {e}"))?;
    let total: u64 = (0..archive.len())
        .filter_map(|i| archive.by_index_raw(i).ok().map(|f| f.size()))
        .sum();
    std::fs::create_dir_all(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    let mut done = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("cannot read the zip: {e}"))?;
        let rel = entry
            .enclosed_name()
            .ok_or_else(|| format!("refusing an unsafe path in the zip: {}", entry.name()))?;
        let out = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)
                .map_err(|e| format!("cannot create {}: {e}", out.display()))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        if entry.is_symlink() {
            let mut target = String::new();
            std::io::Read::read_to_string(&mut entry, &mut target)
                .map_err(|e| format!("cannot read a link in the zip: {e}"))?;
            write_symlink(&target, &out, &rel)?;
            continue;
        }
        let mut writer = std::fs::File::create(&out)
            .map_err(|e| format!("cannot write {}: {e}", out.display()))?;
        std::io::copy(&mut entry, &mut writer)
            .map_err(|e| format!("cannot unpack {}: {e}", rel.display()))?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode & 0o7777))
                .map_err(|e| format!("cannot set the mode of {}: {e}", out.display()))?;
        }
        done += entry.size();
        progress(done, total);
    }
    Ok(())
}

/// A symlink from the zip, only when it is relative and stays inside the
/// archive's root (`rel` is the link's own path inside it).
fn write_symlink(target: &str, out: &Path, rel: &Path) -> Result<(), String> {
    let target_path = Path::new(target);
    let depth = rel.components().count().saturating_sub(1) as isize;
    let mut level = depth;
    let mut escapes = target_path.is_absolute();
    for component in target_path.components() {
        match component {
            std::path::Component::ParentDir => level -= 1,
            std::path::Component::Normal(_) => level += 1,
            std::path::Component::CurDir => {}
            _ => escapes = true,
        }
        if level < 0 {
            escapes = true;
        }
    }
    if escapes {
        return Err(format!(
            "refusing a link that leaves the archive: {} -> {target}",
            rel.display()
        ));
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target_path, out)
            .map_err(|e| format!("cannot create link {}: {e}", out.display()))
    }
    #[cfg(not(unix))]
    {
        let _ = out;
        Err(format!(
            "the archive contains a symlink this platform cannot unpack: {}",
            rel.display()
        ))
    }
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

async fn verify(exe: &Path, root: &Path) -> Result<String, Failure> {
    if !is_executable(exe) {
        return Err(format!(
            "the download did not contain a runnable Chromium at {}",
            exe.display()
        )
        .into());
    }
    let run = tokio::process::Command::new(exe)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = match tokio::time::timeout(VERIFY_TIMEOUT, run).await {
        Err(_) => {
            return Err(format!(
                "the new Chromium did not answer `--version` within {} s",
                VERIFY_TIMEOUT.as_secs()
            )
            .into())
        }
        Ok(Err(e)) => return Err(format!("the new Chromium does not start: {e}").into()),
        Ok(Ok(output)) => output,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next().unwrap_or("").trim().to_string();
    if output.status.success() && !line.is_empty() {
        return Ok(line);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let libraries = missing_libraries(&stderr);
    let deb_deps = ["chrome-linux64", "chrome-linux-arm64"]
        .iter()
        .find_map(|dir| std::fs::read_to_string(root.join(dir).join("deb.deps")).ok());
    let detail = stderr.lines().next().unwrap_or("").trim();
    Err(Failure {
        error: if libraries.is_empty() {
            format!(
                "the new Chromium does not start ({}){}",
                output.status,
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            )
        } else {
            format!(
                "the new Chromium does not start: this machine is missing {}",
                libraries.join(", ")
            )
        },
        hint: dependency_hint(&libraries, deb_deps.as_deref()),
    })
}

/// Shared libraries the dynamic loader could not find, from a failed run's
/// stderr (`error while loading shared libraries: libnss3.so: ...`).
pub fn missing_libraries(stderr: &str) -> Vec<String> {
    const NEEDLE: &str = "error while loading shared libraries: ";
    let mut out: Vec<String> = Vec::new();
    for line in stderr.lines() {
        if let Some(rest) = line.split(NEEDLE).nth(1) {
            let lib = rest.split(':').next().unwrap_or("").trim();
            if !lib.is_empty() && !out.iter().any(|l| l == lib) {
                out.push(lib.to_string());
            }
        }
    }
    out
}

/// Debian package names from a Chrome for Testing `deb.deps` file (one
/// dependency per line, `name (>= version)`, alternatives split by `|` —
/// the first alternative is taken).
pub fn deb_packages(deb_deps: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in deb_deps.lines() {
        let first = line.split('|').next().unwrap_or("").trim();
        let name = first
            .split(|c: char| c.is_whitespace() || c == '(')
            .next()
            .unwrap_or("")
            .trim();
        let plausible = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '+' | '.' | ':'));
        if plausible && !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// What to tell a person whose new Chromium cannot load its libraries.
pub fn dependency_hint(libraries: &[String], deb_deps: Option<&str>) -> Option<String> {
    if libraries.is_empty() {
        return None;
    }
    let packages = deb_deps.map(deb_packages).unwrap_or_default();
    Some(if packages.is_empty() {
        "Install your distribution's Chromium dependencies (on Debian/Ubuntu, `sudo apt install \
         chromium` pulls them in, or run `npx playwright install-deps chromium`), then try again."
            .to_string()
    } else {
        format!(
            "Chromium needs system libraries this machine does not have. On Debian/Ubuntu, \
             install them with `sudo apt-get install -y {}` (or install `chromium` from your \
             distribution instead), then try again.",
            packages.join(" ")
        )
    })
}

// ---------------------------------------------------------------------------
// Cross-process lock
// ---------------------------------------------------------------------------

/// `<install dir>/.install.lock`, holding the installing worker's pid. A
/// lock whose pid is gone (or that is hours old) is a crash leftover and is
/// taken over.
struct InstallLock(PathBuf);

impl InstallLock {
    fn acquire(dir: &Path) -> Result<Self, String> {
        use std::io::Write;
        let path = dir.join(LOCK_FILE);
        for _ in 0..2 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    let _ = writeln!(file, "{}", std::process::id());
                    return Ok(Self(path));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if lock_is_stale(&path) {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    let holder = std::fs::read_to_string(&path).unwrap_or_default();
                    return Err(format!(
                        "another browser worker (pid {}) is installing Chromium into {} right \
                         now; wait for it to finish",
                        holder.trim(),
                        dir.display()
                    ));
                }
                Err(e) => return Err(format!("cannot create {}: {e}", path.display())),
            }
        }
        Err(format!("cannot take the install lock {}", path.display()))
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn lock_is_stale(path: &Path) -> bool {
    let old = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age > STALE_LOCK);
    if old {
        return true;
    }
    let Some(pid) = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok())
    else {
        // Unreadable or empty: a writer died between create and write.
        return true;
    };
    // This process never holds the file across jobs (the in-process job
    // slot serializes), so our own pid in it is a leftover.
    if pid == std::process::id() {
        return true;
    }
    !pid_alive(pid)
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 only checks whether the process exists.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(not(unix))]
fn pid_alive(_pid: u32) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
      "timestamp": "2026-10-01T09:01:20.000Z",
      "channels": {
        "Stable": {
          "channel": "Stable",
          "version": "141.0.7390.54",
          "revision": "1509326",
          "downloads": {
            "chrome": [
              {"platform": "linux-arm64", "url": "https://storage.googleapis.com/chrome-for-testing-public/141.0.7390.54/linux-arm64/chrome-linux-arm64.zip"},
              {"platform": "linux64", "url": "https://storage.googleapis.com/chrome-for-testing-public/141.0.7390.54/linux64/chrome-linux64.zip"},
              {"platform": "mac-arm64", "url": "https://storage.googleapis.com/chrome-for-testing-public/141.0.7390.54/mac-arm64/chrome-mac-arm64.zip"},
              {"platform": "mac-x64", "url": "https://storage.googleapis.com/chrome-for-testing-public/141.0.7390.54/mac-x64/chrome-mac-x64.zip"},
              {"platform": "win32", "url": "https://storage.googleapis.com/chrome-for-testing-public/141.0.7390.54/win32/chrome-win32.zip"},
              {"platform": "win64", "url": "https://storage.googleapis.com/chrome-for-testing-public/141.0.7390.54/win64/chrome-win64.zip"}
            ],
            "chrome-headless-shell": [
              {"platform": "linux64", "url": "https://storage.googleapis.com/chrome-for-testing-public/141.0.7390.54/linux64/chrome-headless-shell-linux64.zip"}
            ]
          }
        },
        "Beta": {
          "channel": "Beta",
          "version": "142.0.7444.3",
          "revision": "1522585",
          "downloads": {
            "chrome": [
              {"platform": "linux64", "url": "https://storage.googleapis.com/chrome-for-testing-public/142.0.7444.3/linux64/chrome-linux64.zip"}
            ]
          }
        }
      }
    }"#;

    #[test]
    fn picks_the_stable_full_chrome_build_for_each_platform() {
        let pick = pick_download(FIXTURE, "linux64").unwrap();
        assert_eq!(pick.version, "141.0.7390.54");
        assert!(
            pick.url.ends_with("/linux64/chrome-linux64.zip"),
            "{}",
            pick.url
        );
        for platform in ["linux-arm64", "mac-arm64", "mac-x64", "win64"] {
            let pick = pick_download(FIXTURE, platform).unwrap();
            assert!(pick
                .url
                .contains(&format!("/{platform}/chrome-{platform}.zip")));
        }
        let missing = pick_download(FIXTURE, "freebsd64").unwrap_err();
        assert!(missing.contains("freebsd64"), "{missing}");
    }

    #[test]
    fn manifest_problems_are_reported_not_panicked() {
        assert!(pick_download("not json", "linux64").is_err());
        assert!(pick_download(r#"{"channels":{}}"#, "linux64")
            .unwrap_err()
            .contains("Stable"));
        let odd_version = r#"{"channels":{"Stable":{"version":"../../x","downloads":{"chrome":[{"platform":"linux64","url":"https://x/y.zip"}]}}}}"#;
        assert!(pick_download(odd_version, "linux64")
            .unwrap_err()
            .contains("odd version"));
        let http = r#"{"channels":{"Stable":{"version":"1.2.3","downloads":{"chrome":[{"platform":"linux64","url":"http://x/y.zip"}]}}}}"#;
        assert!(pick_download(http, "linux64")
            .unwrap_err()
            .contains("non-https"));
    }

    #[test]
    fn progress_is_paced_by_time_and_bytes() {
        let total = Some(170_000_000);
        let quarter = Duration::from_millis(250);
        assert!(!should_emit(
            Duration::from_millis(100),
            0,
            100_000_000,
            total
        ));
        assert!(!should_emit(quarter, 0, 1_000_000, total));
        assert!(should_emit(quarter, 0, 1_700_000, total));
        assert!(should_emit(Duration::MAX, 0, 1_700_000, total));
        assert!(!should_emit(quarter, 0, 500_000, None));
        assert!(should_emit(quarter, 0, 2 << 20, None));
    }

    #[test]
    fn progress_payload_shape() {
        let mut p = InstallProgress::new("chromium-abc".to_string());
        p.phase = Phase::Downloading;
        p.bytes_done = 10;
        p.bytes_total = Some(100);
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["phase"], "downloading");
        assert_eq!(v["job_id"], "chromium-abc");
        assert_eq!(v["bytes_total"], 100);
        assert!(v.get("error").is_none());
        assert!(Phase::Done.is_terminal() && Phase::Failed.is_terminal());
        assert!(!Phase::Verifying.is_terminal());
    }

    #[test]
    fn library_hints_come_from_the_loader_error_and_deb_deps() {
        let stderr = "/tmp/x/chrome: error while loading shared libraries: libnss3.so: cannot \
                      open shared object file: No such file or directory\n";
        let libs = missing_libraries(stderr);
        assert_eq!(libs, vec!["libnss3.so".to_string()]);
        let deb = "libasound2 (>= 1.0.17)\nlibatk-bridge2.0-0 (>= 2.5.3)\nlibc6 (>= 2.17)\n\
                   libgbm1 (>= 17.1.0~rc2) | libgbm1-hwe\nlibnss3 (>= 3.35)\nlibnss3 (>= 3.35)\n";
        assert_eq!(
            deb_packages(deb),
            [
                "libasound2",
                "libatk-bridge2.0-0",
                "libc6",
                "libgbm1",
                "libnss3"
            ]
        );
        let hint = dependency_hint(&libs, Some(deb)).unwrap();
        assert!(
            hint.contains("sudo apt-get install -y libasound2"),
            "{hint}"
        );
        assert!(dependency_hint(&libs, None)
            .unwrap()
            .contains("install-deps"));
        assert_eq!(dependency_hint(&[], Some(deb)), None);
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "browser-install-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn extract_keeps_modes_and_reports_progress() {
        use std::io::Write;
        let dir = tmp_dir("extract");
        let zip_path = dir.join("chrome.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut w = zip::ZipWriter::new(file);
            let exec = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(0o755);
            w.add_directory("chrome-linux64/", exec).unwrap();
            w.start_file("chrome-linux64/chrome", exec).unwrap();
            w.write_all(b"#!/bin/sh\necho 'Google Chrome for Testing 1.2.3'\n")
                .unwrap();
            let data = zip::write::SimpleFileOptions::default().unix_permissions(0o644);
            w.start_file("chrome-linux64/deb.deps", data).unwrap();
            w.write_all(b"libnss3 (>= 3.35)\n").unwrap();
            w.finish().unwrap();
        }
        let out = dir.join("out");
        let mut seen = Vec::new();
        extract_zip(&zip_path, &out, |done, total| seen.push((done, total))).unwrap();
        let exe = out.join("chrome-linux64/chrome");
        assert!(exe.is_file());
        assert!(is_executable(&exe));
        assert_eq!(seen.len(), 2);
        assert_eq!(seen.last().unwrap().0, seen.last().unwrap().1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn extract_refuses_paths_that_escape() {
        use std::io::Write;
        let dir = tmp_dir("escape");
        let zip_path = dir.join("evil.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut w = zip::ZipWriter::new(file);
            w.start_file("../evil", zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(b"x").unwrap();
            w.finish().unwrap();
        }
        let err = extract_zip(&zip_path, &dir.join("out"), |_, _| {}).unwrap_err();
        assert!(err.contains("unsafe path"), "{err}");
        assert!(!dir.join("evil").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn symlinks_must_stay_inside_the_archive() {
        let rel = Path::new("a/b/link");
        let out = std::env::temp_dir().join("never-created-link");
        assert!(write_symlink("/etc/passwd", &out, rel).is_err());
        assert!(write_symlink("../../../x", &out, rel).is_err());
    }

    #[test]
    fn install_lock_is_exclusive_and_reclaims_leftovers() {
        let dir = tmp_dir("lock");
        let first = InstallLock::acquire(&dir).unwrap();
        // Our own pid in the file is a leftover by definition, so simulate
        // another live holder: pid 1 always exists on Unix.
        std::fs::write(dir.join(LOCK_FILE), "1\n").unwrap();
        #[cfg(unix)]
        assert!(InstallLock::acquire(&dir)
            .err()
            .unwrap()
            .contains("another browser worker"));
        drop(first);
        assert!(!dir.join(LOCK_FILE).exists());
        // A dead pid (and an empty file) is reclaimed.
        #[cfg(unix)]
        {
            std::fs::write(dir.join(LOCK_FILE), format!("{}\n", u32::MAX - 1)).unwrap();
            drop(InstallLock::acquire(&dir).unwrap());
        }
        std::fs::write(dir.join(LOCK_FILE), "").unwrap();
        drop(InstallLock::acquire(&dir).unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }
}
