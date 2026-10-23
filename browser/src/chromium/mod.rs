//! Which Chromium the worker launches, and the copy it can install itself.
//!
//! One resolver answers "which binary?" for every caller — the interactive
//! launcher, `browser::doctor`, `browser::chromium::status`, and the
//! scrapling browser tiers — so what the doctor reports is what launches.
//! Search order, first hit wins:
//!
//! 1. the configured `executable` (authoritative: when set, nothing else is
//!    tried, so a typo is reported instead of silently replaced);
//! 2. the `CHROME` environment variable;
//! 3. a system install: Chrome/Chromium/Edge by name on `PATH`, then the
//!    platform's usual install locations;
//! 4. the copy `browser::chromium::install` downloaded (`current.json` in the
//!    install dir);
//! 5. the newest Playwright, then Puppeteer, Chromium already on the machine.
//!
//! A system install stays ahead of the managed download, so a Chrome the
//! person installs later wins without anyone deleting anything.

pub mod install;

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;

/// Stable marker in every "no Chromium on this machine" error, so a UI can
/// recognise it and offer the install.
pub const MISSING_CODE: &str = "chromium_missing";
/// Prefix of the error `browser::chromium::install` returns on a platform
/// with no Chrome for Testing build.
pub const UNSUPPORTED_CODE: &str = "chromium_install_unsupported";
/// Overrides the root the managed Chromium is kept under (`<root>/chrome`).
pub const CACHE_DIR_ENV: &str = "III_BROWSER_CACHE_DIR";
/// The Chrome for Testing archives are 190–210 MB depending on platform.
pub const APPROX_DOWNLOAD_MB: u64 = 200;
/// The install's pointer file, inside the install dir.
pub const CURRENT_FILE: &str = "current.json";

/// Where the resolved binary came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// The worker config `executable`.
    Configured,
    /// The `CHROME` environment variable.
    Env,
    /// A system Chrome/Chromium/Edge.
    System,
    /// The copy `browser::chromium::install` downloaded.
    Managed,
    /// A Playwright-installed Chromium.
    Playwright,
    /// A Puppeteer-installed Chrome.
    Puppeteer,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Configured => "configured",
            Source::Env => "env",
            Source::System => "system",
            Source::Managed => "managed",
            Source::Playwright => "playwright",
            Source::Puppeteer => "puppeteer",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub path: PathBuf,
    pub source: Source,
}

/// Everything the resolver reads from the machine, gathered up front so the
/// search itself is a pure function of it (and tests can aim it at temp
/// dirs).
#[derive(Debug, Clone, Default)]
pub struct SearchEnv {
    /// Config `executable`, when non-empty.
    pub configured: Option<PathBuf>,
    /// `$CHROME`, when non-empty.
    pub chrome_env: Option<PathBuf>,
    /// `$PATH`, split.
    pub path_dirs: Vec<PathBuf>,
    /// Well-known absolute install locations, in order.
    pub system_paths: Vec<PathBuf>,
    /// The managed install dir (`<cache root>/chrome`).
    pub install_dir: PathBuf,
    /// Playwright's browsers dir (`ms-playwright`).
    pub playwright_dir: Option<PathBuf>,
    /// Puppeteer's cache dir (holds `chrome/<platform>-<version>/`).
    pub puppeteer_dir: Option<PathBuf>,
}

/// Command names looked up on `PATH`, in order. Mirrors chromiumoxide's own
/// auto-detection (stable channels only) plus `google-chrome`.
const PATH_NAMES: &[&str] = &[
    "chrome",
    "chrome-browser",
    "google-chrome-stable",
    "google-chrome",
    "chromium",
    "chromium-browser",
    "msedge",
    "microsoft-edge",
    "microsoft-edge-stable",
];

#[cfg(target_os = "macos")]
const SYSTEM_PATHS: &[&str] = &[
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
];

#[cfg(all(unix, not(target_os = "macos")))]
const SYSTEM_PATHS: &[&str] = &[
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/usr/bin/microsoft-edge",
    "/usr/bin/microsoft-edge-stable",
    "/opt/google/chrome/chrome",
    "/opt/chromium.org/chromium/chrome",
    "/snap/bin/chromium",
];

#[cfg(windows)]
const SYSTEM_PATHS: &[&str] = &[
    "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
    "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
    "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
];

/// Where a Chromium sits inside a Playwright `chromium-<rev>/` or Puppeteer
/// `<platform>-<version>/` folder, newest layout first.
#[cfg(target_os = "macos")]
const BUNDLED_LAYOUTS: &[&str] = &[
    "chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
    "chrome-mac-x64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
    "chrome-mac/Chromium.app/Contents/MacOS/Chromium",
];

#[cfg(all(unix, not(target_os = "macos")))]
const BUNDLED_LAYOUTS: &[&str] = &[
    "chrome-linux64/chrome",
    "chrome-linux-arm64/chrome",
    "chrome-linux/chrome",
];

#[cfg(windows)]
const BUNDLED_LAYOUTS: &[&str] = &["chrome-win64\\chrome.exe", "chrome-win\\chrome.exe"];

fn non_empty_env(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).filter(|v| !v.is_empty())
}

fn home_dir() -> Option<PathBuf> {
    non_empty_env("HOME")
        .or_else(|| non_empty_env("USERPROFILE"))
        .map(PathBuf::from)
}

/// The per-user cache base: `$XDG_CACHE_HOME` or `~/.cache` (Windows:
/// `%LOCALAPPDATA%`).
fn user_cache_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        if let Some(local) = non_empty_env("LOCALAPPDATA") {
            return Some(PathBuf::from(local));
        }
    }
    non_empty_env("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home_dir().map(|h| h.join(".cache")))
}

/// Root of the worker's own downloads: `$III_BROWSER_CACHE_DIR`, else
/// `${XDG_CACHE_HOME:-~/.cache}/iii/browser`.
pub fn cache_root() -> PathBuf {
    if let Some(dir) = non_empty_env(CACHE_DIR_ENV) {
        return iii_worker_paths::resolve_path(PathBuf::from(dir));
    }
    user_cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("iii")
        .join("browser")
}

/// Where `browser::chromium::install` keeps Chromium: `<cache root>/chrome`,
/// one `<version>/` folder per install plus `current.json`.
pub fn install_dir() -> PathBuf {
    cache_root().join("chrome")
}

fn playwright_dir() -> Option<PathBuf> {
    if let Some(dir) = non_empty_env("PLAYWRIGHT_BROWSERS_PATH") {
        // "0" means "inside node_modules", which we cannot find from here.
        return (dir != "0").then(|| PathBuf::from(dir));
    }
    if cfg!(target_os = "macos") {
        return home_dir().map(|h| h.join("Library/Caches/ms-playwright"));
    }
    user_cache_dir().map(|c| c.join("ms-playwright"))
}

fn puppeteer_dir() -> Option<PathBuf> {
    non_empty_env("PUPPETEER_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|h| h.join(".cache").join("puppeteer")))
}

impl SearchEnv {
    /// The search as this process sees the machine right now.
    pub fn current(cfg: &WorkerConfig) -> Self {
        let mut system_paths: Vec<PathBuf> = SYSTEM_PATHS.iter().map(PathBuf::from).collect();
        if cfg!(target_os = "macos") {
            if let Some(home) = home_dir() {
                system_paths
                    .push(home.join("Applications/Google Chrome.app/Contents/MacOS/Google Chrome"));
            }
        }
        if cfg!(windows) {
            if let Some(local) = non_empty_env("LOCALAPPDATA") {
                system_paths
                    .push(PathBuf::from(local).join("Google\\Chrome\\Application\\chrome.exe"));
            }
        }
        Self {
            configured: (!cfg.executable.is_empty()).then(|| PathBuf::from(&cfg.executable)),
            chrome_env: non_empty_env("CHROME").map(PathBuf::from),
            path_dirs: non_empty_env("PATH")
                .map(|p| std::env::split_paths(&p).collect())
                .unwrap_or_default(),
            system_paths,
            install_dir: install_dir(),
            playwright_dir: playwright_dir(),
            puppeteer_dir: puppeteer_dir(),
        }
    }

    /// Run the search. `None` when nothing usable exists — including when a
    /// configured `executable` is missing (it is never silently replaced).
    pub fn resolve(&self) -> Option<Resolved> {
        let hit = |path: PathBuf, source| Some(Resolved { path, source });
        if let Some(configured) = &self.configured {
            return if is_executable(configured) {
                hit(configured.clone(), Source::Configured)
            } else {
                None
            };
        }
        if let Some(env) = self.chrome_env.as_ref().filter(|p| is_executable(p)) {
            return hit(env.clone(), Source::Env);
        }
        for name in PATH_NAMES {
            for dir in &self.path_dirs {
                let candidate = dir.join(exe_name(name));
                if is_executable(&candidate) {
                    return hit(candidate, Source::System);
                }
            }
        }
        if let Some(path) = self.system_paths.iter().find(|p| is_executable(p)) {
            return hit(path.clone(), Source::System);
        }
        if let Some(path) = managed_executable(&self.install_dir) {
            return hit(path, Source::Managed);
        }
        if let Some(path) = self.playwright_dir.as_deref().and_then(newest_playwright) {
            return hit(path, Source::Playwright);
        }
        if let Some(path) = self.puppeteer_dir.as_deref().and_then(newest_puppeteer) {
            return hit(path, Source::Puppeteer);
        }
        None
    }

    /// The places [`resolve`](Self::resolve) looks, in order, for a "not
    /// found" report.
    pub fn searched(&self) -> Vec<String> {
        if let Some(configured) = &self.configured {
            return vec![format!("config `executable`: {}", configured.display())];
        }
        let mut out = vec!["$CHROME".to_string()];
        out.push(format!("PATH: {}", PATH_NAMES.join(", ")));
        out.extend(self.system_paths.iter().map(|p| p.display().to_string()));
        out.push(self.install_dir.join(CURRENT_FILE).display().to_string());
        if let Some(dir) = &self.playwright_dir {
            out.push(dir.join("chromium-*").display().to_string());
        }
        if let Some(dir) = &self.puppeteer_dir {
            out.push(dir.join("chrome").join("*").display().to_string());
        }
        out
    }
}

/// The Chromium the worker would launch for `cfg`, whatever the configured
/// engine (scrapling's browser tiers always drive Chromium).
pub fn resolve_executable(cfg: &WorkerConfig) -> Option<Resolved> {
    SearchEnv::current(cfg).resolve()
}

fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// A regular file this user may run (Unix: some execute bit set).
pub fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

// ---------------------------------------------------------------------------
// The managed install
// ---------------------------------------------------------------------------

/// `current.json`: which downloaded version the resolver uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentInstall {
    /// Chrome for Testing version, also the folder name under the install dir.
    pub version: String,
    /// Chrome for Testing platform (`linux64`, `mac-arm64`, ...).
    pub platform: String,
    /// The binary, relative to `<install dir>/<version>/`.
    pub executable: String,
    pub installed_at: i64,
}

impl CurrentInstall {
    /// The binary this record points at, or `None` when the record could
    /// escape the install dir.
    pub fn executable_path(&self, install_dir: &Path) -> Option<PathBuf> {
        if !is_plain_component(&self.version) {
            return None;
        }
        let rel = Path::new(&self.executable);
        let safe = !self.executable.is_empty()
            && rel.is_relative()
            && rel
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)));
        safe.then(|| install_dir.join(&self.version).join(rel))
    }
}

/// One path segment: no separators, not `.`/`..`, not hidden.
fn is_plain_component(s: &str) -> bool {
    !s.is_empty() && !s.starts_with('.') && !s.contains(['/', '\\']) && s != ".."
}

pub fn read_current(install_dir: &Path) -> Option<CurrentInstall> {
    let raw = std::fs::read_to_string(install_dir.join(CURRENT_FILE)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Write `current.json` atomically (temp file + rename), so a reader never
/// sees half a record.
pub fn write_current(install_dir: &Path, current: &CurrentInstall) -> std::io::Result<()> {
    std::fs::create_dir_all(install_dir)?;
    let tmp = install_dir.join(format!(".{CURRENT_FILE}.{}.tmp", std::process::id()));
    let body = serde_json::to_vec_pretty(current).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, install_dir.join(CURRENT_FILE))
}

/// The managed binary named by `current.json`, when it is still there.
pub fn managed_executable(install_dir: &Path) -> Option<PathBuf> {
    read_current(install_dir)?
        .executable_path(install_dir)
        .filter(|p| is_executable(p))
}

// ---------------------------------------------------------------------------
// Playwright / Puppeteer caches
// ---------------------------------------------------------------------------

/// Dotted version as numbers for ordering (`131.0.6778.85`); junk parts
/// count as 0.
fn version_key(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|p| p.parse::<u64>().unwrap_or(0))
        .collect()
}

fn first_layout(dir: &Path) -> Option<PathBuf> {
    BUNDLED_LAYOUTS
        .iter()
        .map(|rel| dir.join(rel))
        .find(|p| is_executable(p))
}

/// Newest `chromium-<revision>/` in a Playwright browsers dir (headless
/// shells, `chromium_headless_shell-*`, are skipped: they have no window).
pub fn newest_playwright(dir: &Path) -> Option<PathBuf> {
    let mut revisions: Vec<(u64, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let rev = name.strip_prefix("chromium-")?.parse::<u64>().ok()?;
            Some((rev, e.path()))
        })
        .collect();
    revisions.sort_by_key(|r| std::cmp::Reverse(r.0));
    revisions.iter().find_map(|(_, path)| first_layout(path))
}

/// Newest `chrome/<platform>-<version>/` in a Puppeteer cache dir.
pub fn newest_puppeteer(dir: &Path) -> Option<PathBuf> {
    let mut builds: Vec<(Vec<u64>, PathBuf)> = std::fs::read_dir(dir.join("chrome"))
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let (_, version) = name.split_once('-')?;
            Some((version_key(version), e.path()))
        })
        .collect();
    builds.sort_by(|a, b| b.0.cmp(&a.0));
    builds.iter().find_map(|(_, path)| first_layout(path))
}

// ---------------------------------------------------------------------------
// Platforms
// ---------------------------------------------------------------------------

/// The Chrome for Testing platform for an `(os, arch)` pair as
/// `std::env::consts` spells them, or `None` when there is no build.
pub fn cft_platform(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Some("linux64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        ("macos", "aarch64") => Some("mac-arm64"),
        ("macos", "x86_64") => Some("mac-x64"),
        ("windows", "x86_64") => Some("win64"),
        _ => None,
    }
}

/// This machine's Chrome for Testing platform.
pub fn host_platform() -> Option<&'static str> {
    cft_platform(std::env::consts::OS, std::env::consts::ARCH)
}

/// Why there is no download for `(os, arch)`, in words for a person.
pub fn unsupported_reason(os: &str, arch: &str) -> String {
    let how = match os {
        "linux" => {
            "install Chromium from your distribution (for example `sudo apt install \
                    chromium`)"
        }
        _ => "install Google Chrome or Chromium yourself",
    };
    format!(
        "Chrome for Testing has no build for {os}/{arch}; {how}, and if it is not found, point \
         the worker config `executable` at it"
    )
}

/// The binary inside an extracted Chrome for Testing archive.
pub fn cft_executable(platform: &str) -> Option<&'static str> {
    match platform {
        "linux64" => Some("chrome-linux64/chrome"),
        "linux-arm64" => Some("chrome-linux-arm64/chrome"),
        "mac-arm64" => Some(
            "chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
        ),
        "mac-x64" => Some(
            "chrome-mac-x64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
        ),
        "win64" => Some("chrome-win64/chrome.exe"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The sandbox
// ---------------------------------------------------------------------------

/// Linux only: whether this kernel keeps unprivileged user namespaces —
/// Chromium's sandbox — from a binary that has no AppArmor profile of its
/// own (Ubuntu 23.10+ sets `apparmor_restrict_unprivileged_userns`; some
/// distributions turn the namespaces off outright).
pub fn userns_sandbox_blocked() -> bool {
    #[cfg(target_os = "linux")]
    {
        let read = |path: &str| {
            std::fs::read_to_string(path)
                .ok()
                .map(|raw| raw.trim().to_string())
        };
        read("/proc/sys/kernel/apparmor_restrict_unprivileged_userns").as_deref() == Some("1")
            || read("/proc/sys/kernel/unprivileged_userns_clone").as_deref() == Some("0")
            || read("/proc/sys/user/max_user_namespaces").as_deref() == Some("0")
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Whether a Chromium from `source` must run with `--no-sandbox` when
/// `userns_blocked`: the copies in user caches (the download, Playwright,
/// Puppeteer) have no AppArmor profile or SUID helper, so with the
/// namespaces blocked they abort with "No usable sandbox!" — the same reason
/// Playwright launches them that way. A system install keeps its sandbox
/// (its package ships the profile), and so does anything configured.
pub fn runs_without_sandbox(source: Source, userns_blocked: bool) -> bool {
    userns_blocked
        && matches!(
            source,
            Source::Managed | Source::Playwright | Source::Puppeteer
        )
}

/// [`runs_without_sandbox`] for this machine.
pub fn needs_no_sandbox(resolved: &Resolved) -> bool {
    runs_without_sandbox(resolved.source, userns_sandbox_blocked())
}

/// What `browser::doctor` says when sessions run unsandboxed.
pub const NO_SANDBOX_HOW: &str = "install Chromium or Google Chrome from your distribution (its \
     package ships the AppArmor profile the sandbox needs), or allow unprivileged user \
     namespaces; until then the downloaded Chromium runs with --no-sandbox";

// ---------------------------------------------------------------------------
// The "not found" story
// ---------------------------------------------------------------------------

/// What to do when no Chromium exists: one sentence an agent can relay and
/// a person can act on.
pub fn install_advice() -> String {
    format!(
        "install it from the ADE (Set up the harness → Browser → Download Chromium), call \
         browser::chromium::install (about {APPROX_DOWNLOAD_MB} MB, kept in {}), install \
         Google Chrome or Chromium, or point the worker config `executable` at a browser binary",
        install_dir().display()
    )
}

/// The launcher's error when nothing was found. Starts with
/// [`MISSING_CODE`] so UIs can match it.
pub fn missing_error(cfg: &WorkerConfig) -> String {
    if !cfg.executable.is_empty() {
        return format!(
            "configured executable '{}' does not exist or is not executable; fix or clear the \
             worker config `executable`",
            cfg.executable
        );
    }
    format!(
        "{MISSING_CODE}: no Chromium or Chrome found on this machine. To fix it, {}.",
        install_advice()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);

    impl Tmp {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "browser-chromium-{tag}-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4().simple()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn touch_exe(path: &Path) -> PathBuf {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path.to_path_buf()
    }

    fn empty_env(root: &Path) -> SearchEnv {
        SearchEnv {
            install_dir: root.join("managed"),
            ..SearchEnv::default()
        }
    }

    fn install_managed(install_dir: &Path, version: &str) -> PathBuf {
        let platform = host_platform().unwrap_or("linux64");
        let rel = cft_executable(platform).unwrap();
        let exe = touch_exe(&install_dir.join(version).join(rel));
        write_current(
            install_dir,
            &CurrentInstall {
                version: version.to_string(),
                platform: platform.to_string(),
                executable: rel.to_string(),
                installed_at: 1,
            },
        )
        .unwrap();
        exe
    }

    #[test]
    fn nothing_found_is_none_and_lists_what_was_searched() {
        let tmp = Tmp::new("none");
        let env = empty_env(&tmp.0);
        assert_eq!(env.resolve(), None);
        let searched = env.searched();
        assert!(
            searched.iter().any(|s| s.starts_with("PATH:")),
            "{searched:?}"
        );
        assert!(
            searched.iter().any(|s| s.ends_with(CURRENT_FILE)),
            "{searched:?}"
        );
    }

    #[test]
    fn configured_executable_is_authoritative() {
        let tmp = Tmp::new("configured");
        let mut env = empty_env(&tmp.0);
        let system = touch_exe(&tmp.0.join("bin").join(exe_name("chromium")));
        env.path_dirs = vec![tmp.0.join("bin")];
        env.configured = Some(tmp.0.join("missing-chrome"));
        // Missing configured binary: no fallback to the system one.
        assert_eq!(env.resolve(), None);
        let configured = touch_exe(&tmp.0.join("my-chrome"));
        env.configured = Some(configured.clone());
        assert_eq!(
            env.resolve(),
            Some(Resolved {
                path: configured,
                source: Source::Configured
            })
        );
        env.configured = None;
        assert_eq!(env.resolve().unwrap().path, system);
    }

    #[test]
    fn resolver_order_env_system_managed_playwright_puppeteer() {
        let tmp = Tmp::new("order");
        let mut env = empty_env(&tmp.0);

        // Puppeteer only.
        let pptr_dir = tmp.0.join("puppeteer");
        let layout = BUNDLED_LAYOUTS[0];
        let old = touch_exe(&pptr_dir.join("chrome/linux-120.0.6099.109").join(layout));
        let new = touch_exe(&pptr_dir.join("chrome/linux-131.0.6778.85").join(layout));
        env.puppeteer_dir = Some(pptr_dir);
        let found = env.resolve().unwrap();
        assert_eq!(found.source, Source::Puppeteer);
        assert_eq!(found.path, new);
        assert_ne!(found.path, old);

        // Playwright beats Puppeteer; newest revision wins, shells skipped.
        let pw_dir = tmp.0.join("ms-playwright");
        touch_exe(&pw_dir.join("chromium-1100").join(layout));
        let pw_new = touch_exe(&pw_dir.join("chromium-1200").join(layout));
        touch_exe(&pw_dir.join("chromium_headless_shell-1300").join(layout));
        env.playwright_dir = Some(pw_dir);
        let found = env.resolve().unwrap();
        assert_eq!((found.source, found.path), (Source::Playwright, pw_new));

        // The managed download beats both caches.
        let managed = install_managed(&env.install_dir, "141.0.7390.54");
        let found = env.resolve().unwrap();
        assert_eq!((found.source, found.path), (Source::Managed, managed));

        // A system install beats the managed download.
        let sys = touch_exe(&tmp.0.join("opt/chrome"));
        env.system_paths = vec![tmp.0.join("opt/nothing"), sys.clone()];
        let found = env.resolve().unwrap();
        assert_eq!((found.source, found.path), (Source::System, sys));

        // PATH names come before the fixed locations.
        let on_path = touch_exe(&tmp.0.join("bin").join(exe_name("google-chrome-stable")));
        env.path_dirs = vec![tmp.0.join("empty"), tmp.0.join("bin")];
        let found = env.resolve().unwrap();
        assert_eq!((found.source, found.path), (Source::System, on_path));

        // $CHROME beats everything but the config.
        let from_env = touch_exe(&tmp.0.join("env-chrome"));
        env.chrome_env = Some(from_env.clone());
        let found = env.resolve().unwrap();
        assert_eq!((found.source, found.path), (Source::Env, from_env));
    }

    #[cfg(unix)]
    #[test]
    fn non_executable_files_and_dirs_do_not_count() {
        let tmp = Tmp::new("noexec");
        let mut env = empty_env(&tmp.0);
        let plain = tmp.0.join("chrome");
        std::fs::write(&plain, b"").unwrap();
        std::fs::create_dir_all(tmp.0.join("dir-chrome")).unwrap();
        env.system_paths = vec![plain, tmp.0.join("dir-chrome")];
        assert_eq!(env.resolve(), None);
    }

    #[test]
    fn current_json_round_trips_and_rejects_escapes() {
        let tmp = Tmp::new("current");
        let dir = tmp.0.join("chrome");
        assert_eq!(read_current(&dir), None);
        let exe = install_managed(&dir, "141.0.7390.54");
        let current = read_current(&dir).unwrap();
        assert_eq!(current.version, "141.0.7390.54");
        assert_eq!(managed_executable(&dir), Some(exe.clone()));

        // A record pointing outside the install dir is ignored.
        for (version, executable) in [
            ("..", "chrome"),
            ("141", "../../etc/passwd"),
            ("141", "/usr/bin/chromium"),
            (".hidden", "chrome"),
            ("a/b", "chrome"),
        ] {
            let bad = CurrentInstall {
                version: version.to_string(),
                platform: "linux64".to_string(),
                executable: executable.to_string(),
                installed_at: 0,
            };
            assert_eq!(bad.executable_path(&dir), None, "{version} {executable}");
        }

        // A record whose binary is gone resolves to nothing.
        std::fs::remove_file(&exe).unwrap();
        assert_eq!(managed_executable(&dir), None);

        // Garbage in current.json is not an error, just no install.
        std::fs::write(dir.join(CURRENT_FILE), b"{not json").unwrap();
        assert_eq!(read_current(&dir), None);
    }

    #[test]
    fn platform_mapping() {
        assert_eq!(cft_platform("linux", "x86_64"), Some("linux64"));
        assert_eq!(cft_platform("macos", "aarch64"), Some("mac-arm64"));
        assert_eq!(cft_platform("macos", "x86_64"), Some("mac-x64"));
        assert_eq!(cft_platform("windows", "x86_64"), Some("win64"));
        assert_eq!(cft_platform("linux", "aarch64"), Some("linux-arm64"));
        assert_eq!(cft_platform("freebsd", "x86_64"), None);
        assert_eq!(cft_platform("windows", "aarch64"), None);
        assert_eq!(cft_platform("linux", "riscv64"), None);
        assert!(unsupported_reason("linux", "riscv64").contains("apt install chromium"));
        assert!(unsupported_reason("freebsd", "x86_64").contains("freebsd/x86_64"));
        for platform in ["linux64", "linux-arm64", "mac-arm64", "mac-x64", "win64"] {
            let rel = cft_executable(platform).unwrap();
            assert!(rel.starts_with(&format!("chrome-{platform}/")), "{rel}");
        }
        assert_eq!(cft_executable("win32"), None);
    }

    #[test]
    fn missing_error_carries_the_marker_and_the_way_out() {
        let msg = missing_error(&WorkerConfig::default());
        assert!(msg.starts_with(MISSING_CODE), "{msg}");
        assert!(msg.contains("browser::chromium::install"), "{msg}");
        assert!(msg.contains("Set up the harness"), "{msg}");
        let configured = missing_error(&WorkerConfig {
            executable: "/nope/chrome".to_string(),
            ..WorkerConfig::default()
        });
        assert!(!configured.contains(MISSING_CODE), "{configured}");
        assert!(configured.contains("/nope/chrome"), "{configured}");
    }

    #[test]
    fn only_cache_copies_drop_the_sandbox_and_only_when_blocked() {
        for source in [Source::Managed, Source::Playwright, Source::Puppeteer] {
            assert!(runs_without_sandbox(source, true), "{source:?}");
            assert!(!runs_without_sandbox(source, false), "{source:?}");
        }
        for source in [Source::System, Source::Configured, Source::Env] {
            assert!(!runs_without_sandbox(source, true), "{source:?}");
        }
    }

    #[test]
    fn version_ordering_is_numeric() {
        assert!(version_key("131.0.6778.85") > version_key("99.0.4844.51"));
        assert!(version_key("131.0.6778.85") > version_key("131.0.6778.9"));
    }
}
