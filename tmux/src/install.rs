//! Installing tmux on the engine's computer (simple-installs D1–D7).
//!
//! [`plan`] is a pure function of a [`Host`]: what to try, in order. A system
//! package manager when this user may run it, then the project's official
//! prebuilt binary (`tmux/tmux-builds`) into `~/.local/bin`. [`Installer`] runs
//! the plan in the background and answers `tmux::install::status`. What counts
//! after an attempt, whatever it said, is whether tmux then resolves the normal
//! way and starts; otherwise the next one runs.

use std::collections::{BTreeMap, VecDeque};
use std::error::Error as _;
use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use crate::binary::{self, is_executable_file};
use crate::collector::{Collector, ConsoleClients};
use crate::config::WorkerConfig;
use crate::tmux_cli::{Socket, Tmux};

/// The pinned prebuilt: `tmux-builds` publishes no checksum file, so the pin
/// makes this worker's release the trust decision (D4).
const PREBUILT_VERSION: &str = "3.7c";
const PREBUILTS: [Pin; 4] = [
    Pin {
        os: "linux",
        arch: "x86_64",
        platform: "linux-x86_64",
        sha256: "cc56bd1cc873eb6089c615f0496b072385bda8a6d944069f38564a5d49c128aa",
    },
    Pin {
        os: "linux",
        arch: "aarch64",
        platform: "linux-arm64",
        sha256: "29dcb978da2a4b0cf6790f9e004865dac239a3ba03bbf776dddacce23d03831b",
    },
    Pin {
        os: "macos",
        arch: "aarch64",
        platform: "macos-arm64",
        sha256: "0a763dd0380aa980d239509654da1bc7455843706a3c050f6709c8cd2e13d12d",
    },
    Pin {
        os: "macos",
        arch: "x86_64",
        platform: "macos-x86_64",
        sha256: "1c21f9ade964e4a539be05ffe3a4e95854ab76a0a9c4c05d0ffbe7fc6c4d783f",
    },
];

/// The compiled `xterm-256color` terminfo entry, from ncurses 6.6. The worker's
/// terminals use this type; a system without a terminfo database (Alpine with
/// no `ncurses-terminfo-base`) gets it in `~/.terminfo` with the prebuilt
/// (D27).
const XTERM_256COLOR: &[u8] = include_bytes!("xterm-256color.terminfo");
const TERMINAL_TYPE: &str = "xterm-256color";

const TOOLS: [&str; 8] = [
    "apt-get", "dnf", "pacman", "zypper", "apk", "brew", "port", "sudo",
];
/// Not `yum`: a system with yum and no dnf (RHEL 7, Amazon Linux 2) installs
/// tmux 1.8, which this worker can't drive, and which would then come before
/// the official build on `PATH` (D30).
const LINUX_MANAGERS: [InstallMethod; 5] = [
    InstallMethod::Apt,
    InstallMethod::Dnf,
    InstallMethod::Pacman,
    InstallMethod::Zypper,
    InstallMethod::Apk,
];
/// What a package manager's children need on `PATH`, whatever the worker got:
/// `dpkg` refuses to run without the `sbin` folders.
const SYSTEM_DIRS: [&str; 6] = [
    "/usr/local/sbin",
    "/usr/local/bin",
    "/usr/sbin",
    "/usr/bin",
    "/sbin",
    "/bin",
];

const SUDO_PROBE_LIMIT: Duration = Duration::from_secs(5);
const UPDATE_LIMIT: Duration = Duration::from_secs(3 * 60);
const INSTALL_LIMIT: Duration = Duration::from_secs(10 * 60);
const CONNECT_LIMIT: Duration = Duration::from_secs(15);
const STALL_LIMIT: Duration = Duration::from_secs(30);
const DOWNLOAD_LIMIT: Duration = Duration::from_secs(5 * 60);
/// A first run on macOS waits for the system's scan of the new file.
const NEW_FILE_PROBE_LIMIT: Duration = Duration::from_secs(30);
const START_CHECK_LIMIT: Duration = Duration::from_secs(10);
const TERM_GRACE: Duration = Duration::from_secs(5);
/// Longer than any run lasts, so a file this old can't be another process's.
const LEFTOVER_AFTER: Duration = Duration::from_secs(60 * 60);
const READERS_GRACE: Duration = Duration::from_secs(2);
const MAX_DOWNLOAD: u64 = 64 * 1024 * 1024;
const TEMP_PREFIX: &str = ".tmux-install-";

const MAX_LOG_LINES: usize = 1000;
const MAX_LINE_CHARS: usize = 500;
const MAX_RAW_LINE: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstallState {
    /// No install ran since the worker started.
    #[default]
    Idle,
    /// An install is going on.
    Running,
    /// tmux is installed and resolves.
    Succeeded,
    /// Every way to install it failed.
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstallStep {
    /// Looking at the computer.
    Checking,
    /// Refreshing the package manager's list.
    Updating,
    /// Fetching the official build.
    Downloading,
    /// Comparing the download with its checksum.
    Verifying,
    /// A package manager or the official build is putting tmux in place.
    Installing,
    /// Checking that tmux resolves.
    Finishing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstallMethod {
    /// `apt-get`.
    Apt,
    /// `dnf`.
    Dnf,
    /// `pacman`.
    Pacman,
    /// `zypper`.
    Zypper,
    /// `apk`.
    Apk,
    /// Homebrew's `brew`.
    Homebrew,
    /// MacPorts' `port`.
    Macports,
    /// The tmux project's official build, into `~/.local/bin`.
    Prebuilt,
}

impl InstallMethod {
    fn name(self) -> &'static str {
        match self {
            Self::Apt => "apt",
            Self::Dnf => "dnf",
            Self::Pacman => "pacman",
            Self::Zypper => "zypper",
            Self::Apk => "apk",
            Self::Homebrew => "Homebrew",
            Self::Macports => "MacPorts",
            Self::Prebuilt => "the official build",
        }
    }

    fn tool(self) -> &'static str {
        match self {
            Self::Apt => "apt-get",
            Self::Dnf => "dnf",
            Self::Pacman => "pacman",
            Self::Zypper => "zypper",
            Self::Apk => "apk",
            Self::Homebrew => "brew",
            Self::Macports => "port",
            Self::Prebuilt => "",
        }
    }
}

/// Where an install is. `tmux::install` and `tmux::install::status` both answer this.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InstallStatus {
    /// `idle` also answers a worker that restarted mid-install.
    pub state: InstallState,
    /// The step a running install is on.
    pub step: Option<InstallStep>,
    /// What to show for that step, e.g. `Downloading tmux…`.
    pub label: Option<String>,
    /// True facts about the step, to alternate with the label.
    pub hints: Vec<String>,
    /// 0 to 1, while the step knows how far it is.
    pub progress: Option<f64>,
    /// The way being tried, or the one that worked. Null when tmux was already there.
    pub method: Option<InstallMethod>,
    /// Where tmux is, once it resolves.
    pub path: Option<String>,
    /// Its version, once it resolves.
    pub version: Option<String>,
    /// Why it failed, one sentence for people.
    pub error: Option<String>,
    /// Empty unless the state is `failed`: every command and its output.
    pub log: Vec<String>,
}

/// What [`plan`] needs to know about this computer.
#[derive(Debug, Clone, Default)]
pub struct Host {
    /// `std::env::consts::OS`.
    pub os: String,
    /// `std::env::consts::ARCH`.
    pub arch: String,
    pub root: bool,
    /// `sudo -n true` works.
    pub sudo: bool,
    /// The tools found, by name.
    pub tools: BTreeMap<String, PathBuf>,
    pub home: Option<PathBuf>,
}

impl Host {
    pub async fn detect() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|home| home.is_absolute());
        let dirs = tool_dirs(std::env::var_os("PATH").as_deref(), home.as_deref());
        let tools: BTreeMap<String, PathBuf> = TOOLS
            .iter()
            .filter_map(|tool| {
                let found = dirs
                    .iter()
                    .map(|dir| dir.join(tool))
                    .find(|path| is_executable_file(path))?;
                Some(((*tool).to_string(), found))
            })
            .collect();
        // SAFETY: geteuid has no preconditions and cannot fail.
        let root = unsafe { libc::geteuid() } == 0;
        let sudo = match tools.get("sudo") {
            Some(sudo) if !root => passwordless_sudo(sudo).await,
            _ => false,
        };
        Self {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            root,
            sudo,
            tools,
            home,
        }
    }

    fn describe(&self) -> String {
        let who = match (self.root, self.sudo) {
            (true, _) => "root",
            (false, true) => "a user with passwordless sudo",
            (false, false) => "a user without passwordless sudo",
        };
        let tools = self.tools.keys().cloned().collect::<Vec<_>>().join(", ");
        format!(
            "{} {}, running as {who}, found: {}",
            self.os,
            self.arch,
            if tools.is_empty() { "none" } else { &tools }
        )
    }
}

/// Where to look for a package manager. Only absolute folders: these tools run
/// as root, and an empty or relative `PATH` entry means the current folder.
fn tool_dirs(path_var: Option<&OsStr>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = path_var
        .map(|value| std::env::split_paths(value).collect())
        .unwrap_or_default();
    dirs.extend(binary::fallback_dirs(home));
    dirs.extend(["/usr/sbin", "/sbin"].map(PathBuf::from));
    dirs.retain(|dir| dir.is_absolute());
    dirs
}

async fn passwordless_sudo(sudo: &Path) -> bool {
    let probe = Command::new(sudo)
        .args(["-n", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status();
    matches!(
        tokio::time::timeout(SUDO_PROBE_LIMIT, probe).await,
        Ok(Ok(status)) if status.success()
    )
}

struct Pin {
    os: &'static str,
    arch: &'static str,
    platform: &'static str,
    sha256: &'static str,
}

/// One process of a package manager's attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedCommand {
    pub step: InstallStep,
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub limit: Duration,
    /// A failed `apt-get update` doesn't stop the install: the old list may do.
    pub required: bool,
}

/// The official build, and where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prebuilt {
    pub url: String,
    pub sha256: String,
    pub target: PathBuf,
    pub home: PathBuf,
    pub connect_limit: Duration,
    pub stall_limit: Duration,
    pub download_limit: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attempt {
    Manager {
        method: InstallMethod,
        commands: Vec<PlannedCommand>,
    },
    Prebuilt(Prebuilt),
}

impl Attempt {
    fn method(&self) -> InstallMethod {
        match self {
            Self::Manager { method, .. } => *method,
            Self::Prebuilt(_) => InstallMethod::Prebuilt,
        }
    }
}

/// What to try, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub attempts: Vec<Attempt>,
    /// Log lines: what this computer is and what was left out.
    pub notes: Vec<String>,
    /// The sentence once the attempts run out, when the prebuilt can't be one
    /// of them. The prebuilt's failures have sentences of their own.
    pub otherwise: Option<String>,
}

enum Privilege<'a> {
    Root,
    Sudo(&'a Path),
}

fn planned(
    step: InstallStep,
    limit: Duration,
    required: bool,
    tool: &Path,
    args: &[&str],
    env: &[(&str, &str)],
    privilege: Option<&Privilege<'_>>,
) -> PlannedCommand {
    let tool = tool.to_string_lossy().into_owned();
    let args = args.iter().map(|arg| arg.to_string());
    let pairs = env.iter().map(|(k, v)| (k.to_string(), v.to_string()));
    match privilege {
        // sudo resets the environment, so it goes on the command line (D2).
        Some(Privilege::Sudo(sudo)) => {
            let mut argv = vec![
                sudo.to_string_lossy().into_owned(),
                "-n".to_string(),
                "env".to_string(),
            ];
            argv.extend(pairs.map(|(k, v)| format!("{k}={v}")));
            argv.push(tool);
            argv.extend(args);
            PlannedCommand {
                step,
                argv,
                env: Vec::new(),
                limit,
                required,
            }
        }
        _ => PlannedCommand {
            step,
            argv: std::iter::once(tool).chain(args).collect(),
            env: pairs.collect(),
            limit,
            required,
        },
    }
}

fn manager_commands(
    method: InstallMethod,
    tool: &Path,
    privilege: Option<&Privilege<'_>>,
) -> Vec<PlannedCommand> {
    let install = |args: &[&str], env: &[(&str, &str)]| {
        planned(
            InstallStep::Installing,
            INSTALL_LIMIT,
            true,
            tool,
            args,
            env,
            privilege,
        )
    };
    let update = |args: &[&str], env: &[(&str, &str)]| {
        planned(
            InstallStep::Updating,
            UPDATE_LIMIT,
            false,
            tool,
            args,
            env,
            privilege,
        )
    };
    const DEBIAN: &[(&str, &str)] = &[("DEBIAN_FRONTEND", "noninteractive")];
    match method {
        // A stale list is why this refreshes first: the mirrors drop the versions
        // an old list names.
        InstallMethod::Apt => vec![
            update(&["update"], DEBIAN),
            install(&["install", "-y", "tmux"], DEBIAN),
        ],
        // Never `-Sy`: a refreshed database with one package from it is a partial
        // upgrade, which Arch doesn't support. A stale or empty database fails,
        // and the official build takes over (D50).
        InstallMethod::Pacman => vec![install(&["-S", "--noconfirm", "--needed", "tmux"], &[])],
        InstallMethod::Dnf => vec![install(&["install", "-y", "tmux"], &[])],
        InstallMethod::Zypper => vec![
            install(&["--non-interactive", "install", "tmux"], &[]),
            // openSUSE's tmux keeps its sockets in /run/tmux, which its rpm
            // and tmpfiles make on a booted system but not in a container (D28).
            planned(
                InstallStep::Installing,
                UPDATE_LIMIT,
                false,
                Path::new("/bin/mkdir"),
                &["-p", "-m", "1777", "/run/tmux"],
                &[],
                privilege,
            ),
        ],
        InstallMethod::Apk => vec![install(
            &["add", "--no-progress", "--update-cache", "tmux"],
            &[],
        )],
        InstallMethod::Homebrew => vec![install(
            &["install", "tmux"],
            &[
                ("HOMEBREW_NO_AUTO_UPDATE", "1"),
                ("HOMEBREW_NO_INSTALL_CLEANUP", "1"),
                ("HOMEBREW_NO_ENV_HINTS", "1"),
                ("NONINTERACTIVE", "1"),
            ],
        )],
        InstallMethod::Macports => vec![install(&["-N", "install", "tmux"], &[])],
        InstallMethod::Prebuilt => Vec::new(),
    }
}

/// What to try on `host` (D1).
pub fn plan(host: &Host) -> Plan {
    let mut plan = Plan::default();
    plan.notes.push(host.describe());
    let privilege = if host.root {
        Some(Privilege::Root)
    } else if host.sudo {
        host.tools.get("sudo").map(|sudo| Privilege::Sudo(sudo))
    } else {
        None
    };

    let manager = match host.os.as_str() {
        "linux" => {
            let found = LINUX_MANAGERS
                .iter()
                .find_map(|method| Some((*method, host.tools.get(method.tool())?)));
            match (&found, &privilege) {
                (Some((method, _)), None) => plan.notes.push(format!(
                    "{} needs root or passwordless sudo, which this user doesn't have.",
                    method.tool()
                )),
                (None, _) => plan
                    .notes
                    .push("No known package manager here.".to_string()),
                _ => {}
            }
            found.filter(|_| privilege.is_some())
        }
        "macos" => {
            let brew = host.tools.get("brew").filter(|_| {
                // Homebrew refuses root.
                if host.root {
                    plan.notes.push("Homebrew doesn't run as root.".to_string());
                }
                !host.root
            });
            let port = host.tools.get("port").filter(|_| {
                if privilege.is_none() {
                    plan.notes.push(
                        "MacPorts needs root or passwordless sudo, which this user doesn't have."
                            .to_string(),
                    );
                }
                privilege.is_some()
            });
            brew.map(|brew| (InstallMethod::Homebrew, brew))
                .or(port.map(|port| (InstallMethod::Macports, port)))
        }
        other => {
            plan.otherwise = Some(format!(
                "This worker can't install tmux on {other}. Install it with this system's own tools."
            ));
            return plan;
        }
    };

    let manager_name = manager.map(|(method, _)| method.name());
    if let Some((method, tool)) = manager {
        let privilege = match method {
            InstallMethod::Homebrew => None,
            _ => privilege.as_ref(),
        };
        plan.attempts.push(Attempt::Manager {
            method,
            commands: manager_commands(method, tool, privilege),
        });
    }

    let pin = PREBUILTS
        .iter()
        .find(|pin| pin.os == host.os && pin.arch == host.arch);
    match (pin, &host.home) {
        (Some(pin), Some(home)) => plan.attempts.push(Attempt::Prebuilt(Prebuilt {
            url: format!(
                "https://github.com/tmux/tmux-builds/releases/download/v{PREBUILT_VERSION}/tmux-{PREBUILT_VERSION}-{}.tar.gz",
                pin.platform
            ),
            sha256: pin.sha256.to_string(),
            target: home.join(".local/bin/tmux"),
            home: home.clone(),
            connect_limit: CONNECT_LIMIT,
            stall_limit: STALL_LIMIT,
            download_limit: DOWNLOAD_LIMIT,
        })),
        (Some(_), None) => {
            plan.otherwise = Some(match manager_name {
                Some(name) => format!(
                    "{name} couldn't install tmux, and HOME isn't set for the engine, so there's no folder for the official build. Set HOME and start the engine again."
                ),
                None => "HOME isn't set for the engine, so there's no folder to put tmux in. Set HOME and start the engine again.".to_string(),
            });
        }
        (None, _) => {
            plan.otherwise = Some(match (manager_name, ask_an_admin(host)) {
                (Some(name), _) => format!(
                    "{name} couldn't install tmux, and there's no ready-made tmux for this processor."
                ),
                (None, Some(command)) => format!(
                    "There's no ready-made tmux for this processor, and installing it needs an administrator. Ask one to run: {command}"
                ),
                (None, None) => "There's no ready-made tmux for this processor and no package manager here this worker knows. Ask an administrator to install tmux.".to_string(),
            });
        }
    }
    plan
}

fn ask_an_admin(host: &Host) -> Option<String> {
    let command = match host.os.as_str() {
        "linux" => {
            let method = LINUX_MANAGERS
                .iter()
                .find(|method| host.tools.contains_key(method.tool()))?;
            match method {
                InstallMethod::Apt => "sudo apt-get install -y tmux",
                InstallMethod::Dnf => "sudo dnf install -y tmux",
                InstallMethod::Pacman => "sudo pacman -S --noconfirm tmux",
                InstallMethod::Zypper => "sudo zypper --non-interactive install tmux",
                InstallMethod::Apk => "sudo apk add tmux",
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(command.to_string())
}

/// Why this configuration can't be helped by an install, in a sentence. Only
/// a tool named `tmux` comes out of one, so any other name is out too.
fn blocked(os: &str, binary: &str) -> Option<String> {
    if !matches!(os, "linux" | "macos") {
        return Some(format!(
            "This worker can't install tmux on {os}. Install it with this system's own tools."
        ));
    }
    if binary != "tmux" {
        return Some(format!(
            "This worker is set to use \"{binary}\" instead of tmux, so installing tmux wouldn't help. Change tmux_binary in its configuration."
        ));
    }
    None
}

/// With no official build for this processor only a package manager can help
/// (D52). Whether this user may run one takes a process, and without it the
/// install still answers with the command to ask an administrator to run.
fn can_try(os: &str, arch: &str, binary: &str, manager_here: impl FnOnce() -> bool) -> bool {
    let official_build = PREBUILTS.iter().any(|pin| pin.os == os && pin.arch == arch);
    blocked(os, binary).is_none() && (official_build || manager_here())
}

fn manager_here() -> bool {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let dirs = tool_dirs(std::env::var_os("PATH").as_deref(), home.as_deref());
    LINUX_MANAGERS.iter().any(|method| {
        dirs.iter()
            .any(|dir| is_executable_file(&dir.join(method.tool())))
    })
}

/// `tmux::status.installable` for an unavailable tmux. Starts no process, and
/// looks at files only on a processor without an official build, so it costs
/// `tmux::status` next to nothing.
pub fn installable(binary: &str) -> bool {
    can_try(
        std::env::consts::OS,
        std::env::consts::ARCH,
        binary,
        manager_here,
    )
}

/// Cleans a process's bytes into log lines: a carriage return starts its line
/// over, so a progress bar leaves only its last state.
#[derive(Default)]
struct LineCleaner {
    line: Vec<u8>,
    after_cr: bool,
}

impl LineCleaner {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for &byte in bytes {
            if byte == b'\n' {
                self.after_cr = false;
                lines.extend(clean_line(&self.line));
                self.line.clear();
                continue;
            }
            if self.after_cr {
                self.line.clear();
                self.after_cr = false;
            }
            if byte == b'\r' {
                self.after_cr = true;
            } else if self.line.len() < MAX_RAW_LINE {
                self.line.push(byte);
            }
        }
        lines
    }

    fn finish(&mut self) -> Option<String> {
        let line = clean_line(&self.line);
        self.line.clear();
        line
    }
}

fn clean_line(raw: &[u8]) -> Option<String> {
    static ANSI: OnceLock<Regex> = OnceLock::new();
    let ansi = ANSI.get_or_init(|| {
        Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)?|\x1b[ -/]*[0-~]?")
            .expect("the ANSI pattern compiles")
    });
    let text = String::from_utf8_lossy(raw);
    let text = ansi.replace_all(&text, "");
    let text: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\t')
        .collect();
    let text = text.trim_end();
    if text.trim_start().is_empty() {
        return None;
    }
    if text.chars().count() > MAX_LINE_CHARS {
        let cut: String = text.chars().take(MAX_LINE_CHARS).collect();
        return Some(format!("{cut}…"));
    }
    Some(text.to_string())
}

/// The last [`MAX_LOG_LINES`] lines: the end of a log says why it failed.
#[derive(Default)]
struct CappedLog {
    lines: VecDeque<String>,
    dropped: usize,
}

impl CappedLog {
    fn push(&mut self, line: String) {
        if self.lines.len() == MAX_LOG_LINES {
            self.lines.pop_front();
            self.dropped += 1;
        }
        self.lines.push_back(line);
    }

    fn snapshot(&self) -> Vec<String> {
        let marker =
            (self.dropped > 0).then(|| format!("… {} earlier lines left out", self.dropped));
        marker
            .into_iter()
            .chain(self.lines.iter().cloned())
            .collect()
    }
}

#[derive(Default)]
struct Shared {
    status: InstallStatus,
    log: CappedLog,
}

/// The state a run writes and `tmux::install::status` reads.
#[derive(Clone, Default)]
struct Progress(Arc<Mutex<Shared>>);

impl Progress {
    fn with<T>(&self, change: impl FnOnce(&mut Shared) -> T) -> T {
        change(&mut self.0.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// Starts a new run unless one is on (D7).
    fn begin(&self) -> bool {
        self.with(|shared| {
            if shared.status.state == InstallState::Running {
                return false;
            }
            *shared = Shared::default();
            shared.status.state = InstallState::Running;
            shared.status.step = Some(InstallStep::Checking);
            shared.status.label = Some("Checking this computer…".to_string());
            shared.status.hints = vec!["Looking for a package manager".to_string()];
            true
        })
    }

    fn step(&self, step: InstallStep, label: &str, hints: Vec<String>) {
        self.with(|shared| {
            shared.status.step = Some(step);
            shared.status.label = Some(label.to_string());
            shared.status.hints = hints;
            shared.status.progress = None;
        });
    }

    fn method(&self, method: InstallMethod) {
        self.with(|shared| shared.status.method = Some(method));
    }

    fn progress(&self, progress: f64) {
        self.with(|shared| shared.status.progress = Some(progress));
    }

    /// Whatever the text and wherever it came from, it goes in cleaned and capped.
    fn log(&self, text: impl AsRef<str>) {
        for line in text
            .as_ref()
            .lines()
            .filter_map(|line| clean_line(line.as_bytes()))
        {
            tracing::debug!(target: "tmux::install", "{line}");
            self.with(|shared| shared.log.push(line));
        }
    }

    fn lines(&self) -> Vec<String> {
        self.with(|shared| shared.log.snapshot())
    }

    fn status(&self) -> InstallStatus {
        self.with(|shared| shared.status.clone())
    }

    fn finish(&self, outcome: Result<Installed, String>) {
        self.with(|shared| {
            let method = shared.status.method;
            shared.status = match outcome {
                Ok(installed) => InstallStatus {
                    state: InstallState::Succeeded,
                    method: installed.method,
                    path: Some(installed.path),
                    version: Some(installed.version),
                    ..InstallStatus::default()
                },
                Err(error) => InstallStatus {
                    state: InstallState::Failed,
                    method,
                    error: Some(error),
                    log: shared.log.snapshot(),
                    ..InstallStatus::default()
                },
            };
        });
    }
}

/// A file removed on drop unless it was moved into place.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// An attempt that didn't work: the sentence for people, what goes in the log.
#[derive(Debug)]
struct Failure {
    sentence: String,
    detail: String,
}

impl Failure {
    fn new(sentence: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            sentence: sentence.into(),
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Installed {
    method: Option<InstallMethod>,
    path: String,
    version: String,
}

fn display_command(command: &PlannedCommand) -> String {
    command
        .env
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .chain(command.argv.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn extended_path(current: Option<&OsStr>) -> OsString {
    let mut dirs: Vec<PathBuf> = current
        .map(|value| std::env::split_paths(value).collect())
        .unwrap_or_default();
    // These children run as root: no entry that means the current folder.
    dirs.retain(|dir| dir.is_absolute());
    for dir in SYSTEM_DIRS.map(PathBuf::from) {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    std::env::join_paths(dirs).unwrap_or_else(|_| current.unwrap_or_default().to_owned())
}

fn pump<R>(mut reader: R, progress: Progress) -> tokio::task::JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut cleaner = LineCleaner::default();
        let mut buffer = [0u8; 4096];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(read) => cleaner
                    .push(&buffer[..read])
                    .into_iter()
                    .for_each(|line| progress.log(line)),
            }
        }
        if let Some(line) = cleaner.finish() {
            progress.log(line);
        }
    })
}

fn signal_group(pid: Option<u32>, signal: i32) {
    if let Some(pid) = pid {
        // SAFETY: kill has no memory preconditions; the group was made for
        // this child by `process_group(0)`.
        unsafe { libc::kill(-(pid as i32), signal) };
    }
}

/// Anyone left in the group, even one this user can't signal.
fn group_alive(pid: Option<u32>) -> bool {
    pid.is_some_and(|pid| {
        // SAFETY: as in `signal_group`; signal 0 only asks.
        let answer = unsafe { libc::kill(-(pid as i32), 0) };
        answer == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    })
}

/// Runs one process: no stdin, its own process group, a time limit, its lines
/// in the log. A process group that outlives the limit gets a chance to clean
/// up, then SIGKILL, whoever in it is left: a helper that ignores SIGTERM would
/// run on beside the next attempt. A daemon it left behind after ending by
/// itself doesn't hold up the run.
async fn run_command(planned: &PlannedCommand, progress: &Progress) -> Result<(), String> {
    progress.log(format!("$ {}", display_command(planned)));
    let mut command = Command::new(&planned.argv[0]);
    command
        .args(&planned.argv[1..])
        .envs(planned.env.iter().map(|(key, value)| (key, value)))
        .env("PATH", extended_path(std::env::var_os("PATH").as_deref()))
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|error| format!("couldn't start {}: {error}", planned.argv[0]))?;
    let pid = child.id();
    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        readers.push(pump(stdout, progress.clone()));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(pump(stderr, progress.clone()));
    }

    let result = match tokio::time::timeout(planned.limit, child.wait()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => Err(format!("{} ended with {status}", planned.argv[0])),
        Ok(Err(error)) => Err(format!("waiting for {}: {error}", planned.argv[0])),
        Err(_) => {
            signal_group(pid, libc::SIGTERM);
            let grace = tokio::time::Instant::now() + TERM_GRACE;
            let _ = tokio::time::timeout_at(grace, child.wait()).await;
            while group_alive(pid) && tokio::time::Instant::now() < grace {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if group_alive(pid) {
                signal_group(pid, libc::SIGKILL);
            }
            let _ = child.start_kill();
            Err(format!(
                "{} took longer than {} seconds and was stopped",
                planned.argv[0],
                planned.limit.as_secs_f32()
            ))
        }
    };
    let aborts: Vec<_> = readers.iter().map(|reader| reader.abort_handle()).collect();
    let drained = futures_join(readers);
    if tokio::time::timeout(READERS_GRACE, drained).await.is_err() {
        aborts.iter().for_each(|abort| abort.abort());
    }
    result
}

async fn futures_join(readers: Vec<tokio::task::JoinHandle<()>>) {
    for reader in readers {
        let _ = reader.await;
    }
}

fn reqwest_detail(error: &reqwest::Error) -> String {
    let mut detail = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        detail.push_str(": ");
        detail.push_str(&cause.to_string());
        source = cause.source();
    }
    detail
}

fn network_failure(error: &reqwest::Error) -> Failure {
    let sentence = if error.is_timeout() {
        "The tmux download took too long. Check this computer's internet connection and try again."
    } else {
        "Couldn't download tmux from GitHub. Check this computer's internet connection and try again."
    };
    Failure::new(sentence, reqwest_detail(error))
}

async fn download(prebuilt: &Prebuilt, progress: &Progress) -> Result<Vec<u8>, Failure> {
    let client = reqwest::Client::builder()
        .connect_timeout(prebuilt.connect_limit)
        .read_timeout(prebuilt.stall_limit)
        .timeout(prebuilt.download_limit)
        .user_agent(concat!("iii-tmux-worker/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| network_failure(&error))?;
    progress.log(format!("Downloading {}", prebuilt.url));
    let mut response = client
        .get(&prebuilt.url)
        .send()
        .await
        .map_err(|error| network_failure(&error))?;
    let status = response.status();
    if !status.is_success() {
        return Err(Failure::new(
            format!(
                "GitHub answered {} for the tmux download. Try again in a little while.",
                status.as_u16()
            ),
            format!("{} answered {status}", prebuilt.url),
        ));
    }
    let total = response.content_length().filter(|total| *total > 0);
    let too_big = || {
        Failure::new(
            "The tmux download is far bigger than it should be, so it was stopped.",
            format!("more than {MAX_DOWNLOAD} bytes"),
        )
    };
    if total.is_some_and(|total| total > MAX_DOWNLOAD) {
        return Err(too_big());
    }
    let mut bytes = Vec::with_capacity(total.unwrap_or(0) as usize);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| network_failure(&error))?
    {
        bytes.extend_from_slice(&chunk);
        if bytes.len() as u64 > MAX_DOWNLOAD {
            return Err(too_big());
        }
        if let Some(total) = total {
            progress.progress((bytes.len() as f64 / total as f64).min(1.0));
        }
    }
    progress.log(format!("Downloaded {} bytes", bytes.len()));
    Ok(bytes)
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Writes the archive's `tmux` to `to`, a new file only this user can touch.
fn extract_tmux(archive: &[u8], to: &Path) -> std::io::Result<()> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in archive.entries()? {
        let entry = entry?;
        if entry.header().entry_type().is_file()
            && entry.path()?.file_name() == Some(OsStr::new("tmux"))
        {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o700)
                .open(to)?;
            std::io::copy(&mut entry.take(MAX_DOWNLOAD), &mut file)?;
            return file.flush();
        }
    }
    Err(std::io::Error::new(
        ErrorKind::InvalidData,
        "the archive holds no tmux",
    ))
}

/// Moves `temp` to `target` atomically, and never over a file: a hard link
/// fails when the target exists, where a rename would replace it (D4).
fn place(temp: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::set_permissions(temp, std::fs::Permissions::from_mode(0o755))?;
    link_new(temp, target)
}

fn link_new(temp: &Path, target: &Path) -> std::io::Result<()> {
    match std::fs::hard_link(temp, target) {
        Ok(()) => std::fs::remove_file(temp),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Err(error),
        Err(_) if target.symlink_metadata().is_ok() => {
            Err(std::io::Error::from(ErrorKind::AlreadyExists))
        }
        // A file system without hard links.
        Err(_) => std::fs::rename(temp, target),
    }
}

fn already_there(target: &Path) -> Failure {
    Failure::new(
        format!(
            "There's already a file at {} that isn't a working tmux, so it was left alone. Move it away and try again.",
            target.display()
        ),
        format!("{} exists; it is never overwritten", target.display()),
    )
}

/// The `-V` check on a file just written. One that another process's fork
/// still holds open for a moment answers "text file busy".
async fn probe_new_file(path: &Path) -> Result<String, String> {
    let busy = format!("os error {}", libc::ETXTBSY);
    let mut tries = 0;
    loop {
        match binary::probe_within(path, NEW_FILE_PROBE_LIMIT).await {
            Err(error) if error.contains(&busy) && tries < 10 => {
                tries += 1;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            result => return result,
        }
    }
}

async fn blocking<T, F>(work: F) -> std::io::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> std::io::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|error| Err(std::io::Error::other(error.to_string())))
}

/// What a killed run left in `dir`: regular files of exactly the temp file's
/// name, and only old ones, because another process may be installing right now.
fn sweep_leftovers(dir: &Path, older_than: Duration, progress: &Progress) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let ours = name
            .to_str()
            .and_then(|name| name.strip_prefix(TEMP_PREFIX))
            .is_some_and(|id| id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()));
        // `DirEntry::metadata` doesn't follow a symlink.
        let old = entry.metadata().is_ok_and(|meta| {
            meta.is_file()
                && meta
                    .modified()
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age > older_than)
        });
        if ours && old && std::fs::remove_file(entry.path()).is_ok() {
            progress.log(format!(
                "Removed {}, left by a run that was killed",
                entry.path().display()
            ));
        }
    }
}

async fn run_prebuilt(
    prebuilt: &Prebuilt,
    progress: &Progress,
    after: Option<InstallMethod>,
) -> Result<(), Failure> {
    let target = prebuilt.target.clone();
    if let Some(dir) = target.parent() {
        sweep_leftovers(dir, LEFTOVER_AFTER, progress);
    }
    if target.symlink_metadata().is_ok() {
        return Err(already_there(&target));
    }

    let mut hints: Vec<String> = after
        .map(|method| {
            format!(
                "{} couldn't install it, so this uses the official build",
                method.name()
            )
        })
        .into_iter()
        .collect();
    hints.push("From the tmux project's official builds".to_string());
    hints.push(format!("tmux {PREBUILT_VERSION}"));
    progress.step(InstallStep::Downloading, "Downloading tmux…", hints);
    let archive = download(prebuilt, progress).await?;

    progress.step(
        InstallStep::Verifying,
        "Checking the download…",
        vec!["Making sure it's the official build".to_string()],
    );
    let sha256 = hex_sha256(&archive);
    if sha256 != prebuilt.sha256 {
        return Err(Failure::new(
            "The tmux download didn't match its checksum, so it wasn't installed. Try again in a little while.",
            format!("expected sha256 {}, got {sha256}", prebuilt.sha256),
        ));
    }

    let dir = target
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| prebuilt.home.clone());
    progress.step(
        InstallStep::Installing,
        "Installing tmux…",
        vec![
            format!("Into {}", dir.display()),
            "No administrator password needed".to_string(),
        ],
    );
    let unwritable = |error: std::io::Error| {
        Failure::new(
            format!(
                "Couldn't write to {}. Check that the folder can be written to and the disk has room.",
                dir.display()
            ),
            error.to_string(),
        )
    };
    let temp = TempFile(dir.join(format!("{TEMP_PREFIX}{}", uuid::Uuid::new_v4().simple())));
    let (mkdir, written) = (dir.clone(), temp.0.clone());
    blocking(move || {
        std::fs::create_dir_all(&mkdir)?;
        extract_tmux(&archive, &written)
    })
    .await
    .map_err(unwritable)?;

    let version = probe_new_file(&temp.0).await.map_err(|error| {
        Failure::new("The downloaded tmux doesn't run on this computer.", error)
    })?;
    progress.log(format!("The new file answers: tmux {version}"));

    let (from, to) = (temp.0.clone(), target.clone());
    blocking(move || place(&from, &to))
        .await
        .map_err(|error| match error.kind() {
            ErrorKind::AlreadyExists => already_there(&target),
            _ => unwritable(error),
        })?;
    progress.log(format!("Installed {}", target.display()));

    match ensure_terminal_type(&prebuilt.home) {
        Ok(Some(written)) => progress.log(format!(
            "No terminfo entry for {TERMINAL_TYPE} here, so wrote {}",
            written.display()
        )),
        Ok(None) => {}
        Err(error) => progress.log(format!(
            "Couldn't write the terminfo entry for {TERMINAL_TYPE}: {error}"
        )),
    }
    Ok(())
}

/// The folders ncurses reads terminfo from, as the prebuilt was built to.
fn terminfo_dirs(
    home: &Path,
    terminfo: Option<&OsStr>,
    terminfo_dirs: Option<&OsStr>,
) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = terminfo.map(PathBuf::from).into_iter().collect();
    dirs.push(home.join(".terminfo"));
    if let Some(value) = terminfo_dirs {
        dirs.extend(std::env::split_paths(value));
    }
    dirs.extend(["/etc/terminfo", "/lib/terminfo", "/usr/share/terminfo"].map(PathBuf::from));
    // Relative to whatever the current folder is, so it says nothing about the
    // terminals to come.
    dirs.retain(|dir| dir.is_absolute());
    dirs
}

fn has_terminal_type(dirs: &[PathBuf]) -> bool {
    // macOS names the letter's folder by its hex code.
    dirs.iter().any(|dir| {
        dir.join("x").join(TERMINAL_TYPE).is_file() || dir.join("78").join(TERMINAL_TYPE).is_file()
    })
}

/// The static tmux can't attach without an entry for the type the worker's
/// terminals use; every command without a terminal works anyway, so nothing
/// else would notice (D27). `Some` when it wrote the file.
fn ensure_terminal_type(home: &Path) -> std::io::Result<Option<PathBuf>> {
    let dirs = terminfo_dirs(
        home,
        std::env::var_os("TERMINFO").as_deref(),
        std::env::var_os("TERMINFO_DIRS").as_deref(),
    );
    if has_terminal_type(&dirs) {
        return Ok(None);
    }
    let dir = home.join(".terminfo/x");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(TERMINAL_TYPE);
    let written = write_new_file(&path, |file| file.write_all(XTERM_256COLOR))?;
    Ok(written.then_some(path))
}

/// Writes `path` in one step, `true` when this wrote it: a kill or a full disk
/// leaves no half a file that a later run takes for the real one (D27), and a
/// file that's there stays.
fn write_new_file(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> std::io::Result<bool> {
    let temp =
        TempFile(path.with_file_name(format!("{TEMP_PREFIX}{}", uuid::Uuid::new_v4().simple())));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp.0)?;
    write(&mut file)?;
    file.sync_all()?;
    match link_new(&temp.0, path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error),
    }
}

async fn run_manager(
    method: InstallMethod,
    commands: &[PlannedCommand],
    progress: &Progress,
) -> Result<(), Failure> {
    for command in commands {
        match command.step {
            InstallStep::Updating => progress.step(
                command.step,
                "Updating the package list…",
                vec![format!("Using {}", method.name())],
            ),
            _ => progress.step(
                command.step,
                &format!("Installing tmux with {}…", method.name()),
                vec![
                    "Using this computer's package manager".to_string(),
                    "This can take a few minutes".to_string(),
                ],
            ),
        }
        match run_command(command, progress).await {
            Ok(()) => {}
            Err(reason) if !command.required => {
                progress.log(format!("{reason}. Carrying on."));
            }
            Err(reason) => {
                return Err(Failure::new(
                    format!("{} couldn't install tmux.", method.name()),
                    reason,
                ))
            }
        }
    }
    Ok(())
}

/// Why a tmux an attempt left behind doesn't count.
#[derive(Debug)]
enum Unverified {
    /// It doesn't resolve: the next attempt may help.
    Missing(String),
    /// It resolves and can't start. No attempt helps: `PATH` comes first, so
    /// a second tmux in `~/.local/bin` would never be picked.
    CantStart { path: String, error: String },
}

fn cant_start(progress: &Progress, path: &str, error: &str) -> String {
    progress.log(format!("{path} can't start a server: {error}"));
    format!("tmux is installed at {path}, but it can't start on this computer.")
}

/// Runs the plan until an attempt ends with a tmux that `verify` accepts.
async fn execute<V, F>(
    plan: &Plan,
    progress: &Progress,
    verify: V,
) -> Result<(InstallMethod, String, String), String>
where
    V: Fn() -> F,
    F: Future<Output = Result<(String, String), Unverified>>,
{
    for note in &plan.notes {
        progress.log(note.as_str());
    }
    let mut failed: Option<(InstallMethod, String)> = None;
    for (index, attempt) in plan.attempts.iter().enumerate() {
        let method = attempt.method();
        progress.method(method);
        let outcome = match attempt {
            Attempt::Manager { commands, .. } => run_manager(method, commands, progress).await,
            Attempt::Prebuilt(prebuilt) => {
                run_prebuilt(prebuilt, progress, failed.as_ref().map(|(m, _)| *m)).await
            }
        };
        if outcome.is_ok() {
            progress.step(
                InstallStep::Finishing,
                "Finishing up…",
                vec!["Making sure tmux runs".to_string()],
            );
        }
        // After a failure too: a package manager can fail after installing tmux,
        // and another process (the button and `--install-tmux`, two engines on
        // one home) may have just put it there (D51).
        let failure = match (verify().await, outcome) {
            (Ok((path, version)), outcome) => {
                if let Err(failure) = outcome {
                    progress.log(failure.detail);
                    progress.log(format!("{} said it failed, but tmux works.", method.name()));
                }
                return Ok((method, path, version));
            }
            (Err(Unverified::CantStart { path, error }), outcome) => {
                if let Err(failure) = outcome {
                    progress.log(failure.detail);
                }
                return Err(cant_start(progress, &path, &error));
            }
            (Err(Unverified::Missing(_)), Err(failure)) => failure,
            (Err(Unverified::Missing(error)), Ok(())) => Failure::new(
                format!("{} finished, but tmux still can't be found.", method.name()),
                format!("tmux doesn't resolve after {}: {error}", method.name()),
            ),
        };
        progress.log(failure.detail.as_str());
        progress.log(failure.sentence.as_str());
        tracing::warn!(method = method.name(), detail = %failure.detail, "tmux install attempt failed");
        failed = Some((method, failure.sentence));
        if let Some(next) = plan.attempts.get(index + 1) {
            progress.log(format!(
                "{} didn't work, trying {} next.",
                method.name(),
                next.method().name()
            ));
        }
    }
    Err(plan
        .otherwise
        .clone()
        .or(failed.map(|(_, sentence)| sentence))
        .unwrap_or_else(|| "tmux couldn't be installed.".to_string()))
}

/// The private server of [`start_check`]. Never the configured socket: an
/// install must not touch a person's tmux server, and this one ends in
/// `kill-server`.
fn check_server(path: &Path) -> Tmux {
    let name = uuid::Uuid::new_v4().simple().to_string();
    Tmux {
        path: path.to_path_buf(),
        socket: Socket::Name(format!("iii-check-{}", &name[..8])),
        timeout: START_CHECK_LIMIT,
        version: String::new(),
    }
}

/// A real server on a private socket: `-V` can't tell that a tmux answers but
/// can't start here, and every command of the worker's would then fail (D28).
async fn start_check(path: &Path) -> Result<(), String> {
    let tmux = check_server(path);
    let started = tmux
        .run([
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "check",
            "sleep 5",
        ])
        .await;
    let socket = tmux.run(["display-message", "-p", "#{socket_path}"]).await;
    let _ = tmux.run(["kill-server"]).await;
    if let Ok(socket) = socket {
        remove_check_socket(&tmux.socket, Path::new(socket.trim()));
    }
    started.map(drop).map_err(|error| error.to_string())
}

/// tmux leaves its socket file behind. Only the check's own goes, whatever the
/// binary answered: a person's `default` socket sits in the same folder.
fn remove_check_socket(server: &Socket, answered: &Path) {
    let own = matches!(server, Socket::Name(name) if answered.file_name() == Some(name.as_ref()));
    if own {
        let _ = std::fs::remove_file(answered);
    }
}

/// Looks for tmux again, the normal way, as `(path, version)`.
async fn resolved(collector: &Collector) -> Result<(String, String), String> {
    let found = collector.resolve_tmux_again().await?;
    Ok((found.path.to_string_lossy().into_owned(), found.version))
}

/// Nothing to install, but Try again after a tmux that can't start must not
/// turn into a success with nothing changed (D28).
async fn already_there_check(
    progress: &Progress,
    path: String,
    version: String,
) -> Result<Installed, String> {
    progress.log(format!("tmux {version} is already at {path}"));
    if let Err(error) = start_check(Path::new(&path)).await {
        return Err(cant_start(progress, &path, &error));
    }
    Ok(Installed {
        method: None,
        path,
        version,
    })
}

async fn install(progress: &Progress, collector: &Collector) -> Result<Installed, String> {
    let binary = collector.config().tmux_binary.clone();
    if let Some(sentence) = blocked(std::env::consts::OS, &binary) {
        return Err(sentence);
    }
    if let Ok((path, version)) = resolved(collector).await {
        return already_there_check(progress, path, version).await;
    }
    let plan = plan(&Host::detect().await);
    let verify = || async {
        let (path, version) = resolved(collector).await.map_err(Unverified::Missing)?;
        match start_check(Path::new(&path)).await {
            Ok(()) => Ok((path, version)),
            Err(error) => Err(Unverified::CantStart { path, error }),
        }
    };
    let (method, path, version) = execute(&plan, progress, verify).await?;
    Ok(Installed {
        method: Some(method),
        path,
        version,
    })
}

/// The one install of this worker (D7).
#[derive(Default)]
pub struct Installer {
    progress: Progress,
}

impl Installer {
    pub fn status(&self) -> InstallStatus {
        self.progress.status()
    }

    /// Every command and its output so far.
    pub fn log(&self) -> Vec<String> {
        self.progress.lines()
    }

    /// Starts an install, or joins the one that runs, and answers at once.
    /// `on_change` runs when it ended.
    pub fn start(
        &self,
        collector: Arc<Collector>,
        on_change: Arc<dyn Fn() + Send + Sync>,
    ) -> InstallStatus {
        if self.progress.begin() {
            let progress = self.progress.clone();
            tokio::spawn(async move {
                tracing::info!("tmux install started");
                let run = {
                    let progress = progress.clone();
                    tokio::spawn(async move { install(&progress, &collector).await })
                };
                let outcome = run.await.unwrap_or_else(|error| {
                    Err(format!("The install stopped unexpectedly: {error}"))
                });
                match &outcome {
                    Ok(installed) => {
                        tracing::info!(method = ?installed.method, path = %installed.path, version = %installed.version, "tmux installed")
                    }
                    Err(error) => tracing::warn!(%error, "tmux install failed"),
                }
                progress.finish(outcome);
                on_change();
            });
        }
        self.status()
    }
}

/// `--install-tmux`: the same run with no engine. Prints the steps and the log
/// to stderr, the final status as JSON on the last line of stdout, and answers
/// the process's exit code.
pub async fn run_cli() -> i32 {
    let collector = Arc::new(Collector::new(
        WorkerConfig::default().into_shared(),
        ConsoleClients::default(),
    ));
    let installer = Installer::default();
    installer.start(collector, Arc::new(|| {}));
    let mut shown = None;
    let status = loop {
        let status = installer.status();
        if status.state != InstallState::Running {
            break status;
        }
        if status.label != shown {
            eprintln!("{}", status.label.as_deref().unwrap_or_default());
            shown = status.label;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    for line in installer.log() {
        eprintln!("  {line}");
    }
    println!(
        "{}",
        serde_json::to_string(&status).unwrap_or_else(|_| "{}".to_string())
    );
    i32::from(status.state != InstallState::Succeeded)
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    use super::*;

    fn host(os: &str, arch: &str, root: bool, sudo: bool, tools: &[&str]) -> Host {
        Host {
            os: os.to_string(),
            arch: arch.to_string(),
            root,
            sudo,
            tools: tools
                .iter()
                .map(|tool| (tool.to_string(), PathBuf::from(format!("/t/{tool}"))))
                .collect(),
            home: Some(PathBuf::from("/h")),
        }
    }

    fn methods(plan: &Plan) -> Vec<InstallMethod> {
        plan.attempts.iter().map(Attempt::method).collect()
    }

    fn commands(plan: &Plan) -> Vec<String> {
        plan.attempts
            .iter()
            .flat_map(|attempt| match attempt {
                Attempt::Manager { commands, .. } => commands.iter().map(display_command).collect(),
                Attempt::Prebuilt(_) => Vec::new(),
            })
            .collect()
    }

    fn prebuilt_of(plan: &Plan) -> &Prebuilt {
        plan.attempts
            .iter()
            .find_map(|attempt| match attempt {
                Attempt::Prebuilt(prebuilt) => Some(prebuilt),
                _ => None,
            })
            .expect("the plan has the prebuilt")
    }

    #[test]
    fn each_linux_manager_as_root_runs_directly_then_the_prebuilt() {
        let cases: [(&str, InstallMethod, &[&str]); 5] = [
            (
                "apt-get",
                InstallMethod::Apt,
                &[
                    "DEBIAN_FRONTEND=noninteractive /t/apt-get update",
                    "DEBIAN_FRONTEND=noninteractive /t/apt-get install -y tmux",
                ],
            ),
            ("dnf", InstallMethod::Dnf, &["/t/dnf install -y tmux"]),
            (
                "pacman",
                InstallMethod::Pacman,
                &["/t/pacman -S --noconfirm --needed tmux"],
            ),
            (
                "zypper",
                InstallMethod::Zypper,
                &[
                    "/t/zypper --non-interactive install tmux",
                    "/bin/mkdir -p -m 1777 /run/tmux",
                ],
            ),
            (
                "apk",
                InstallMethod::Apk,
                &["/t/apk add --no-progress --update-cache tmux"],
            ),
        ];
        for (tool, method, expected) in cases {
            let plan = plan(&host("linux", "x86_64", true, false, &[tool]));
            assert_eq!(
                methods(&plan),
                vec![method, InstallMethod::Prebuilt],
                "{tool}"
            );
            assert_eq!(commands(&plan), expected, "{tool}");
            assert_eq!(plan.otherwise, None, "{tool}");
        }
    }

    #[test]
    fn sudo_goes_non_interactive_with_the_environment_on_its_command_line() {
        let plan = plan(&host("linux", "x86_64", false, true, &["apt-get", "sudo"]));
        assert_eq!(
            methods(&plan),
            vec![InstallMethod::Apt, InstallMethod::Prebuilt]
        );
        assert_eq!(
            commands(&plan),
            vec![
                "/t/sudo -n env DEBIAN_FRONTEND=noninteractive /t/apt-get update",
                "/t/sudo -n env DEBIAN_FRONTEND=noninteractive /t/apt-get install -y tmux",
            ]
        );
    }

    #[test]
    fn without_root_or_sudo_a_manager_is_left_out_and_the_log_says_so() {
        for tools in [&["apt-get"][..], &["apt-get", "sudo"][..]] {
            let plan = plan(&host("linux", "x86_64", false, false, tools));
            assert_eq!(methods(&plan), vec![InstallMethod::Prebuilt]);
            assert!(plan
                .notes
                .iter()
                .any(|note| note.contains("apt-get needs root")));
        }
    }

    #[test]
    fn the_first_linux_manager_wins_and_only_that_one_runs() {
        let plan = plan(&host(
            "linux",
            "x86_64",
            true,
            false,
            &["apk", "zypper", "pacman", "dnf", "apt-get"],
        ));
        assert_eq!(
            methods(&plan),
            vec![InstallMethod::Apt, InstallMethod::Prebuilt]
        );
        let plan = super::plan(&host("linux", "x86_64", true, false, &["pacman", "dnf"]));
        assert_eq!(
            methods(&plan),
            vec![InstallMethod::Dnf, InstallMethod::Prebuilt]
        );
    }

    #[test]
    fn a_system_with_only_yum_takes_the_official_build() {
        // yum alone means RHEL 7 or Amazon Linux 2: tmux 1.8 (D30).
        let plan = plan(&host("linux", "x86_64", true, false, &["yum"]));
        assert_eq!(methods(&plan), vec![InstallMethod::Prebuilt]);
    }

    #[test]
    fn pacman_never_refreshes_its_database_for_one_package() {
        // A refreshed database with one package from it is a partial upgrade (D50).
        for (root, sudo) in [(true, false), (false, true)] {
            let plan = plan(&host("linux", "x86_64", root, sudo, &["pacman", "sudo"]));
            let commands = commands(&plan);
            assert!(
                commands.iter().all(|line| !line.contains("-Sy")),
                "{commands:?}"
            );
            assert_eq!(commands.len(), 1, "{commands:?}");
            assert!(commands[0].ends_with("/t/pacman -S --noconfirm --needed tmux"));
        }
    }

    #[test]
    fn every_program_a_plan_runs_is_named_by_its_full_path() {
        let sudo = PathBuf::from("/t/sudo");
        for method in LINUX_MANAGERS {
            for privilege in [Privilege::Root, Privilege::Sudo(&sudo)] {
                for command in manager_commands(method, Path::new("/t/x"), Some(&privilege)) {
                    let program = command
                        .argv
                        .iter()
                        .find(|arg| {
                            !arg.ends_with("/sudo")
                                && !["-n", "env"].contains(&arg.as_str())
                                && !arg.contains('=')
                        })
                        .unwrap();
                    assert!(
                        Path::new(program).is_absolute(),
                        "{}",
                        display_command(&command)
                    );
                }
            }
        }
    }

    #[test]
    fn every_supported_arch_gets_its_pinned_build() {
        let cases = [
            ("linux", "x86_64", "linux-x86_64", "cc56bd1c"),
            ("linux", "aarch64", "linux-arm64", "29dcb978"),
            ("macos", "aarch64", "macos-arm64", "0a763dd0"),
            ("macos", "x86_64", "macos-x86_64", "1c21f9ad"),
        ];
        for (os, arch, platform, sha) in cases {
            let plan = plan(&host(os, arch, false, false, &[]));
            assert_eq!(methods(&plan), vec![InstallMethod::Prebuilt]);
            let prebuilt = prebuilt_of(&plan);
            assert_eq!(
                prebuilt.url,
                format!("https://github.com/tmux/tmux-builds/releases/download/v3.7c/tmux-3.7c-{platform}.tar.gz")
            );
            assert!(prebuilt.sha256.starts_with(sha), "{platform}");
            assert_eq!(prebuilt.sha256.len(), 64, "{platform}");
            assert_eq!(prebuilt.target, PathBuf::from("/h/.local/bin/tmux"));
        }
    }

    #[test]
    fn macos_uses_homebrew_as_the_user_without_sudo() {
        let plan = plan(&host("macos", "aarch64", false, false, &["brew"]));
        assert_eq!(
            methods(&plan),
            vec![InstallMethod::Homebrew, InstallMethod::Prebuilt]
        );
        assert_eq!(
            commands(&plan),
            vec!["HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALL_CLEANUP=1 HOMEBREW_NO_ENV_HINTS=1 NONINTERACTIVE=1 /t/brew install tmux"]
        );
        // Even with sudo at hand.
        let plan = super::plan(&host("macos", "aarch64", false, true, &["brew", "sudo"]));
        assert_eq!(
            commands(&plan),
            vec!["HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALL_CLEANUP=1 HOMEBREW_NO_ENV_HINTS=1 NONINTERACTIVE=1 /t/brew install tmux"]
        );
    }

    #[test]
    fn macos_without_brew_takes_the_prebuilt() {
        let plan = plan(&host("macos", "x86_64", false, false, &[]));
        assert_eq!(methods(&plan), vec![InstallMethod::Prebuilt]);
    }

    #[test]
    fn homebrew_never_runs_as_root() {
        let plan = plan(&host("macos", "aarch64", true, false, &["brew"]));
        assert_eq!(methods(&plan), vec![InstallMethod::Prebuilt]);
        assert!(plan.notes.iter().any(|note| note.contains("root")));
    }

    #[test]
    fn macports_needs_root_or_sudo_and_comes_after_homebrew() {
        let root = plan(&host("macos", "aarch64", true, false, &["port"]));
        assert_eq!(
            methods(&root),
            vec![InstallMethod::Macports, InstallMethod::Prebuilt]
        );
        assert_eq!(commands(&root), vec!["/t/port -N install tmux"]);

        let sudo = plan(&host("macos", "aarch64", false, true, &["port", "sudo"]));
        assert_eq!(
            commands(&sudo),
            vec!["/t/sudo -n env /t/port -N install tmux"]
        );

        let neither = plan(&host("macos", "aarch64", false, false, &["port"]));
        assert_eq!(methods(&neither), vec![InstallMethod::Prebuilt]);
        assert!(neither.notes.iter().any(|note| note.contains("MacPorts")));

        let both = plan(&host(
            "macos",
            "aarch64",
            false,
            true,
            &["brew", "port", "sudo"],
        ));
        assert_eq!(
            methods(&both),
            vec![InstallMethod::Homebrew, InstallMethod::Prebuilt]
        );
    }

    #[test]
    fn armv7_only_has_a_manager_and_says_what_to_ask_an_admin() {
        let root = plan(&host("linux", "arm", true, false, &["apt-get"]));
        assert_eq!(methods(&root), vec![InstallMethod::Apt]);
        assert_eq!(
            root.otherwise.as_deref(),
            Some("apt couldn't install tmux, and there's no ready-made tmux for this processor.")
        );

        let user = plan(&host("linux", "arm", false, false, &["apt-get"]));
        assert!(user.attempts.is_empty());
        assert_eq!(
            user.otherwise.as_deref(),
            Some("There's no ready-made tmux for this processor, and installing it needs an administrator. Ask one to run: sudo apt-get install -y tmux")
        );

        let nothing = plan(&host("linux", "arm", false, false, &[]));
        assert!(nothing.attempts.is_empty());
        assert!(nothing
            .otherwise
            .as_deref()
            .unwrap()
            .contains("Ask an administrator to install tmux"));
    }

    #[test]
    fn an_unset_home_is_a_sentence() {
        let mut bare = host("linux", "x86_64", false, false, &[]);
        bare.home = None;
        let plan_alone = plan(&bare);
        assert!(plan_alone.attempts.is_empty());
        assert!(plan_alone
            .otherwise
            .as_deref()
            .unwrap()
            .starts_with("HOME isn't set"));

        let mut root = host("linux", "x86_64", true, false, &["dnf"]);
        root.home = None;
        let with_manager = plan(&root);
        assert_eq!(methods(&with_manager), vec![InstallMethod::Dnf]);
        assert!(with_manager
            .otherwise
            .as_deref()
            .unwrap()
            .starts_with("dnf couldn't install tmux, and HOME isn't set"));
    }

    #[test]
    fn an_unsupported_os_has_no_attempts() {
        let plan = plan(&host("freebsd", "x86_64", true, false, &["apt-get"]));
        assert!(plan.attempts.is_empty());
        assert!(plan.otherwise.as_deref().unwrap().contains("freebsd"));
    }

    #[test]
    fn only_a_bare_tmux_on_a_supported_os_can_be_helped() {
        assert_eq!(blocked("linux", "tmux"), None);
        assert_eq!(blocked("macos", "tmux"), None);
        assert!(blocked("freebsd", "tmux").is_some());
        assert!(blocked("linux", "/opt/tmux/bin/tmux").is_some());
        assert!(blocked("linux", "~/bin/tmux").is_some());
        assert!(blocked("linux", "tmux-next").is_some());
    }

    #[test]
    fn an_install_can_try_only_where_something_could_work() {
        let no_need_to_look = || -> bool { panic!("this processor has an official build") };
        assert!(can_try("linux", "x86_64", "tmux", no_need_to_look));
        assert!(can_try("linux", "aarch64", "tmux", no_need_to_look));
        assert!(can_try("macos", "aarch64", "tmux", no_need_to_look));
        assert!(can_try("macos", "x86_64", "tmux", no_need_to_look));
        // No official build: a package manager or nothing (D52).
        assert!(can_try("linux", "arm", "tmux", || true));
        assert!(!can_try("linux", "arm", "tmux", || false));
        assert!(!can_try("linux", "powerpc64", "tmux", || false));
        assert!(!can_try("freebsd", "x86_64", "tmux", || true));
        assert!(!can_try("linux", "x86_64", "/opt/tmux/bin/tmux", || true));
    }

    #[test]
    fn every_manager_command_stays_non_interactive() {
        for method in LINUX_MANAGERS {
            for command in manager_commands(method, Path::new("/t/x"), Some(&Privilege::Root))
                .into_iter()
                .filter(|command| command.argv[0] == "/t/x")
            {
                let line = display_command(&command);
                let quiet = match method {
                    InstallMethod::Apt => line.contains("DEBIAN_FRONTEND=noninteractive"),
                    InstallMethod::Dnf => line.contains(" -y"),
                    InstallMethod::Pacman => line.contains("--noconfirm"),
                    InstallMethod::Zypper => line.contains("--non-interactive"),
                    InstallMethod::Apk => line.contains("--no-progress"),
                    _ => false,
                };
                assert!(quiet, "{line}");
            }
        }
    }

    #[test]
    fn a_tiny_path_gets_the_system_folders_behind_what_it_has() {
        let extended = extended_path(Some(OsStr::new("/a:/usr/bin")));
        assert_eq!(
            extended.to_string_lossy(),
            "/a:/usr/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/sbin:/bin"
        );
        assert!(extended_path(None).to_string_lossy().contains("/usr/sbin"));
    }

    #[test]
    fn what_runs_as_root_never_looks_in_the_current_folder() {
        let dirs = tool_dirs(Some(OsStr::new(":rel:/a:./b")), Some(Path::new("")));
        assert!(dirs.iter().all(|dir| dir.is_absolute()), "{dirs:?}");
        assert!(dirs.contains(&PathBuf::from("/a")));
        assert!(dirs.contains(&PathBuf::from("/usr/sbin")));

        let extended = extended_path(Some(OsStr::new(":rel:/a:./b")));
        let entries: Vec<PathBuf> = std::env::split_paths(&extended).collect();
        assert!(entries.iter().all(|dir| dir.is_absolute()), "{entries:?}");
        assert_eq!(entries[0], PathBuf::from("/a"));
    }

    fn cleaned(bytes: &[u8]) -> Vec<String> {
        let mut cleaner = LineCleaner::default();
        let mut lines = cleaner.push(bytes);
        lines.extend(cleaner.finish());
        lines
    }

    #[test]
    fn the_log_loses_colours_and_progress_bars() {
        assert_eq!(cleaned(b"\x1b[1;32mok\x1b[0m\n"), vec!["ok"]);
        assert_eq!(cleaned(b"10%\r50%\r100%\ndone\n"), vec!["100%", "done"]);
        assert_eq!(cleaned(b"line\r\nnext\r\n"), vec!["line", "next"]);
        assert_eq!(cleaned(b"\x1b]0;title\x07text\n"), vec!["text"]);
        assert_eq!(cleaned(b"\x1b(Bplain\x07\n"), vec!["plain"]);
        assert_eq!(cleaned(b"\n   \n\x1b[K\n"), Vec::<String>::new());
        assert_eq!(cleaned(b"no newline"), vec!["no newline"]);
        assert_eq!(cleaned(b"bar 10%\rbar 20%\r"), vec!["bar 20%"]);
    }

    #[test]
    fn a_line_split_between_reads_stays_one_line() {
        let mut cleaner = LineCleaner::default();
        assert!(cleaner.push(b"hel").is_empty());
        assert_eq!(cleaner.push(b"lo\r").len(), 0);
        assert_eq!(cleaner.push(b"\nnext"), vec!["hello"]);
        assert_eq!(cleaner.finish().as_deref(), Some("next"));
    }

    #[test]
    fn a_long_line_is_cut_and_the_log_keeps_its_end() {
        let line = "x".repeat(MAX_LINE_CHARS + 50);
        let out = cleaned(format!("{line}\n").as_bytes());
        assert_eq!(out[0].chars().count(), MAX_LINE_CHARS + 1);
        assert!(out[0].ends_with('…'));

        let mut log = CappedLog::default();
        for index in 0..MAX_LOG_LINES + 5 {
            log.push(format!("{index}"));
        }
        let lines = log.snapshot();
        assert_eq!(lines.len(), MAX_LOG_LINES + 1);
        assert_eq!(lines[0], "… 5 earlier lines left out");
        assert_eq!(lines.last().unwrap(), &format!("{}", MAX_LOG_LINES + 4));
    }

    #[test]
    fn a_second_start_during_a_run_joins_it() {
        let progress = Progress::default();
        assert!(progress.begin());
        assert!(!progress.begin());
        assert_eq!(progress.status().state, InstallState::Running);
        progress.log("an old line");
        progress.finish(Err("It broke.".to_string()));
        let failed = progress.status();
        assert_eq!(failed.state, InstallState::Failed);
        assert_eq!(failed.error.as_deref(), Some("It broke."));
        assert_eq!(failed.log, vec!["an old line"]);
        // A new run starts clean.
        assert!(progress.begin());
        assert!(progress.status().log.is_empty());
        assert!(progress.lines().is_empty());
    }

    #[test]
    fn the_log_only_goes_out_on_a_failure() {
        let progress = Progress::default();
        assert!(progress.begin());
        progress.log("a line");
        assert!(progress.status().log.is_empty());
        progress.finish(Ok(Installed {
            method: Some(InstallMethod::Apt),
            path: "/usr/bin/tmux".to_string(),
            version: "3.7c".to_string(),
        }));
        let done = progress.status();
        assert_eq!(done.state, InstallState::Succeeded);
        assert_eq!(done.method, Some(InstallMethod::Apt));
        assert_eq!(done.path.as_deref(), Some("/usr/bin/tmux"));
        assert!(done.log.is_empty());
    }

    fn sh(script: &str, limit: Duration, required: bool) -> PlannedCommand {
        PlannedCommand {
            step: InstallStep::Installing,
            argv: vec!["/bin/sh".to_string(), "-c".to_string(), script.to_string()],
            env: Vec::new(),
            limit,
            required,
        }
    }

    fn manager(method: InstallMethod, commands: Vec<PlannedCommand>) -> Attempt {
        Attempt::Manager { method, commands }
    }

    const LONG: Duration = Duration::from_secs(30);

    #[tokio::test]
    async fn stdout_and_stderr_reach_the_log_and_the_exit_status_matters() {
        let progress = Progress::default();
        run_command(&sh("echo out; echo err >&2", LONG, true), &progress)
            .await
            .unwrap();
        let lines = progress.lines();
        assert!(lines[0].starts_with("$ /bin/sh -c"));
        assert!(lines.contains(&"out".to_string()));
        assert!(lines.contains(&"err".to_string()));

        let error = run_command(&sh("exit 7", LONG, true), &progress)
            .await
            .unwrap_err();
        assert!(error.contains("7"), "{error}");
    }

    #[tokio::test]
    async fn a_process_gets_no_stdin_and_cant_wait_for_one() {
        let progress = Progress::default();
        run_command(
            &sh("read answer; echo \"got [$answer]\"", LONG, true),
            &progress,
        )
        .await
        .unwrap();
        assert!(progress.lines().contains(&"got []".to_string()));
    }

    #[tokio::test]
    async fn the_time_limit_stops_a_process_and_what_it_started() {
        let dir = temp_dir();
        let marker = dir.join("child-lived");
        let script = format!("(sleep 3; touch {}) & sleep 60", marker.display());
        let progress = Progress::default();
        let started = std::time::Instant::now();
        let error = run_command(&sh(&script, Duration::from_millis(200), true), &progress)
            .await
            .unwrap_err();
        assert!(error.contains("was stopped"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(10));
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert!(!marker.exists(), "the process group outlived the limit");
    }

    #[tokio::test]
    async fn a_helper_that_ignores_sigterm_doesnt_outlive_the_limit() {
        let dir = temp_dir();
        let beats = dir.join("beats");
        // SIGTERM ends the direct child at once and not its helper.
        let script = format!(
            "(trap '' TERM; while :; do echo beat >> {}; sleep 0.1; done) & sleep 60",
            beats.display()
        );
        let error = run_command(
            &sh(&script, Duration::from_millis(300), true),
            &Progress::default(),
        )
        .await
        .unwrap_err();
        assert!(error.contains("was stopped"), "{error}");
        let count = || std::fs::read_to_string(&beats).unwrap().lines().count();
        let at_the_end = count();
        assert!(at_the_end > 0, "the helper never ran");
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert_eq!(count(), at_the_end, "the helper outlived the limit");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_daemon_holding_the_pipes_doesnt_hold_up_the_run() {
        let progress = Progress::default();
        let started = std::time::Instant::now();
        run_command(&sh("sleep 30 & exit 0", LONG, true), &progress)
            .await
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "iii-tmux-install-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn archive_with(name: &str, contents: &[u8]) -> Vec<u8> {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, name, contents).unwrap();
        tar.into_inner().unwrap().finish().unwrap()
    }

    const FAKE_TMUX: &[u8] = b"#!/bin/sh\necho 'tmux 3.7c'\n";

    /// Serves one body to everyone. `headers` follows the status line.
    fn serve(status: &'static str, headers: String, body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/tmux.tar.gz", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|read| read > 2) {
                    line.clear();
                }
                let _ = write!(stream, "HTTP/1.1 {status}\r\n{headers}\r\n");
                let _ = stream.write_all(&body);
            }
        });
        url
    }

    fn serve_archive(archive: Vec<u8>) -> String {
        let headers = format!("Content-Length: {}\r\nConnection: close\r\n", archive.len());
        serve("200 OK", headers, archive)
    }

    fn prebuilt_at(home: &Path, url: String, sha256: String) -> Prebuilt {
        Prebuilt {
            url,
            sha256,
            target: home.join(".local/bin/tmux"),
            home: home.to_path_buf(),
            connect_limit: Duration::from_secs(5),
            stall_limit: Duration::from_secs(5),
            download_limit: Duration::from_secs(20),
        }
    }

    fn files_in(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn the_prebuilt_lands_checked_with_its_mode_and_no_leftovers() {
        let home = temp_dir();
        let archive = archive_with("tmux", FAKE_TMUX);
        let sha = hex_sha256(&archive);
        let prebuilt = prebuilt_at(&home, serve_archive(archive), sha);
        let progress = Progress::default();
        assert!(progress.begin());
        run_prebuilt(&prebuilt, &progress, None).await.unwrap();

        let target = home.join(".local/bin/tmux");
        assert_eq!(std::fs::read(&target).unwrap(), FAKE_TMUX);
        let mode = target.metadata().unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
        assert_eq!(files_in(&home.join(".local/bin")), vec!["tmux"]);
        // The last write of the bar was the whole file.
        assert_eq!(progress.status().step, Some(InstallStep::Installing));
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn the_progress_follows_content_length() {
        let archive = archive_with("tmux", &vec![7u8; 400_000]);
        let progress = Progress::default();
        let sha = hex_sha256(&archive);
        let home = temp_dir();
        let prebuilt = prebuilt_at(&home, serve_archive(archive.clone()), sha);
        download(&prebuilt, &progress).await.unwrap();
        assert_eq!(progress.status().progress, Some(1.0));

        // No Content-Length: no bar.
        let progress = Progress::default();
        let prebuilt = prebuilt_at(
            &home,
            serve("200 OK", "Connection: close\r\n".to_string(), archive),
            String::new(),
        );
        download(&prebuilt, &progress).await.unwrap();
        assert_eq!(progress.status().progress, None);
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn a_wrong_checksum_puts_nothing_anywhere() {
        let home = temp_dir();
        let archive = archive_with("tmux", FAKE_TMUX);
        let prebuilt = prebuilt_at(&home, serve_archive(archive), "0".repeat(64));
        let failure = run_prebuilt(&prebuilt, &Progress::default(), None)
            .await
            .unwrap_err();
        assert!(failure.sentence.contains("checksum"), "{failure:?}");
        assert!(!home.join(".local").exists());
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn a_file_at_the_target_is_never_overwritten() {
        let home = temp_dir();
        let bin = home.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("tmux"), b"someone else's").unwrap();
        let archive = archive_with("tmux", FAKE_TMUX);
        let sha = hex_sha256(&archive);
        let prebuilt = prebuilt_at(&home, serve_archive(archive), sha);
        let failure = run_prebuilt(&prebuilt, &Progress::default(), None)
            .await
            .unwrap_err();
        assert!(failure.sentence.contains("left alone"), "{failure:?}");
        assert_eq!(std::fs::read(bin.join("tmux")).unwrap(), b"someone else's");
        assert_eq!(files_in(&bin), vec!["tmux"]);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn placing_never_replaces_even_when_the_file_appears_late() {
        let dir = temp_dir();
        let (temp, target) = (dir.join("temp"), dir.join("target"));
        std::fs::write(&temp, b"new").unwrap();
        std::fs::write(&target, b"old").unwrap();
        let error = place(&temp, &target).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&target).unwrap(), b"old");

        std::fs::remove_file(&target).unwrap();
        place(&temp, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(!temp.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_download_that_doesnt_answer_like_tmux_stays_out() {
        let home = temp_dir();
        let archive = archive_with("tmux", b"#!/bin/sh\necho 'not it'\n");
        let sha = hex_sha256(&archive);
        let prebuilt = prebuilt_at(&home, serve_archive(archive), sha);
        let failure = run_prebuilt(&prebuilt, &Progress::default(), None)
            .await
            .unwrap_err();
        assert!(failure.sentence.contains("doesn't run"), "{failure:?}");
        assert_eq!(files_in(&home.join(".local/bin")), Vec::<String>::new());
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn an_archive_without_tmux_stays_out() {
        let home = temp_dir();
        let archive = archive_with("something-else", FAKE_TMUX);
        let sha = hex_sha256(&archive);
        let prebuilt = prebuilt_at(&home, serve_archive(archive), sha);
        let failure = run_prebuilt(&prebuilt, &Progress::default(), None)
            .await
            .unwrap_err();
        assert!(failure.detail.contains("no tmux"), "{failure:?}");
        assert_eq!(files_in(&home.join(".local/bin")), Vec::<String>::new());
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn network_troubles_are_sentences() {
        let home = temp_dir();
        let missing = prebuilt_at(
            &home,
            serve(
                "404 Not Found",
                "Content-Length: 0\r\nConnection: close\r\n".to_string(),
                Vec::new(),
            ),
            String::new(),
        );
        let failure = run_prebuilt(&missing, &Progress::default(), None)
            .await
            .unwrap_err();
        assert!(
            failure.sentence.contains("GitHub answered 404"),
            "{failure:?}"
        );

        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/x", closed.local_addr().unwrap());
        drop(closed);
        let refused = prebuilt_at(&home, url, String::new());
        let failure = run_prebuilt(&refused, &Progress::default(), None)
            .await
            .unwrap_err();
        assert!(
            failure.sentence.contains("internet connection"),
            "{failure:?}"
        );
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn a_stalled_download_is_cut_by_its_limit() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/x", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|read| read > 2) {
                line.clear();
            }
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\npartial"
            );
            std::thread::sleep(Duration::from_secs(10));
        });
        let home = temp_dir();
        let mut prebuilt = prebuilt_at(&home, url, String::new());
        prebuilt.stall_limit = Duration::from_millis(200);
        let started = std::time::Instant::now();
        let failure = download(&prebuilt, &Progress::default()).await.unwrap_err();
        assert!(failure.sentence.contains("took too long"), "{failure:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(home);
    }

    fn resolves() -> Result<(String, String), Unverified> {
        Ok(("/found/tmux".to_string(), "3.7c".to_string()))
    }

    /// What verifying finds: tmux, once `target` is there.
    fn when_there(target: &Path) -> Result<(String, String), Unverified> {
        if target.exists() {
            resolves()
        } else {
            Err(Unverified::Missing("not found".to_string()))
        }
    }

    #[tokio::test]
    async fn a_failing_manager_falls_through_to_the_prebuilt_and_the_log_says_so() {
        let home = temp_dir();
        let archive = archive_with("tmux", FAKE_TMUX);
        let sha = hex_sha256(&archive);
        let plan = Plan {
            attempts: vec![
                manager(
                    InstallMethod::Apt,
                    vec![sh("echo broken; exit 100", LONG, true)],
                ),
                Attempt::Prebuilt(prebuilt_at(&home, serve_archive(archive), sha)),
            ],
            ..Plan::default()
        };
        let target = home.join(".local/bin/tmux");
        let progress = Progress::default();
        let verify = || {
            let found = when_there(&target);
            async move { found }
        };
        let (method, path, _) = execute(&plan, &progress, verify).await.unwrap();
        assert_eq!(method, InstallMethod::Prebuilt);
        assert_eq!(path, "/found/tmux");
        let lines = progress.lines();
        assert!(lines.contains(&"broken".to_string()));
        assert!(lines
            .iter()
            .any(|line| line.contains("apt couldn't install tmux.")));
        assert!(lines.contains(&"apt didn't work, trying the official build next.".to_string()));
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn a_manager_that_succeeded_without_tmux_is_a_fall_through() {
        let home = temp_dir();
        let archive = archive_with("tmux", FAKE_TMUX);
        let sha = hex_sha256(&archive);
        let plan = Plan {
            attempts: vec![
                manager(InstallMethod::Dnf, vec![sh("exit 0", LONG, true)]),
                Attempt::Prebuilt(prebuilt_at(&home, serve_archive(archive), sha)),
            ],
            ..Plan::default()
        };
        let target = home.join(".local/bin/tmux");
        let progress = Progress::default();
        let verify = || {
            let found = when_there(&target);
            async move { found }
        };
        let (method, ..) = execute(&plan, &progress, verify).await.unwrap();
        assert_eq!(method, InstallMethod::Prebuilt);
        assert!(progress
            .lines()
            .iter()
            .any(|line| line.contains("tmux doesn't resolve after dnf: not found")));
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn a_manager_that_works_ends_the_run() {
        let plan = Plan {
            attempts: vec![manager(
                InstallMethod::Pacman,
                vec![sh("exit 0", LONG, true)],
            )],
            ..Plan::default()
        };
        let progress = Progress::default();
        let (method, ..) = execute(&plan, &progress, || async { resolves() })
            .await
            .unwrap();
        assert_eq!(method, InstallMethod::Pacman);
    }

    #[tokio::test]
    async fn a_failed_update_doesnt_stop_the_install() {
        let plan = Plan {
            attempts: vec![manager(
                InstallMethod::Apt,
                vec![
                    sh("exit 100", LONG, false),
                    sh("echo installed", LONG, true),
                ],
            )],
            ..Plan::default()
        };
        let progress = Progress::default();
        let (method, ..) = execute(&plan, &progress, || async { resolves() })
            .await
            .unwrap();
        assert_eq!(method, InstallMethod::Apt);
        let lines = progress.lines();
        assert!(lines.iter().any(|line| line.contains("Carrying on.")));
        assert!(lines.contains(&"installed".to_string()));
    }

    #[tokio::test]
    async fn when_everything_fails_the_error_is_one_sentence_and_the_log_has_the_rest() {
        let home = temp_dir();
        let bin = home.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("tmux"), b"not tmux").unwrap();
        let plan = Plan {
            attempts: vec![
                manager(
                    InstallMethod::Apk,
                    vec![sh("echo 'ERROR: no'; exit 1", LONG, true)],
                ),
                Attempt::Prebuilt(prebuilt_at(
                    &home,
                    "http://127.0.0.1:9/x".to_string(),
                    String::new(),
                )),
            ],
            notes: vec!["linux x86_64".to_string()],
            otherwise: None,
        };
        let progress = Progress::default();
        let verify = || async { Err(Unverified::Missing("no tmux".to_string())) };
        let error = execute(&plan, &progress, verify).await.unwrap_err();
        assert!(error.contains("left alone"), "{error}");
        assert!(!error.contains('\n'));
        let lines = progress.lines();
        assert_eq!(lines[0], "linux x86_64");
        assert!(lines.contains(&"ERROR: no".to_string()));
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn a_tmux_that_cant_start_ends_the_run_with_its_reason_in_the_log() {
        let home = temp_dir();
        let plan = Plan {
            attempts: vec![
                manager(InstallMethod::Zypper, vec![sh("exit 0", LONG, true)]),
                Attempt::Prebuilt(prebuilt_at(
                    &home,
                    "http://127.0.0.1:9/x".to_string(),
                    String::new(),
                )),
            ],
            ..Plan::default()
        };
        let progress = Progress::default();
        let verify = || async {
            Err(Unverified::CantStart {
                path: "/usr/bin/tmux".to_string(),
                error: "couldn't create directory /run/tmux/0".to_string(),
            })
        };
        let error = execute(&plan, &progress, verify).await.unwrap_err();
        assert_eq!(
            error,
            "tmux is installed at /usr/bin/tmux, but it can't start on this computer."
        );
        assert!(progress
            .lines()
            .iter()
            .any(|line| line.contains("couldn't create directory /run/tmux/0")));
        // The official build would never be picked over it.
        assert!(!home.join(".local").exists());
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn a_manager_that_fails_after_installing_tmux_counts() {
        let home = temp_dir();
        let plan = Plan {
            attempts: vec![
                manager(InstallMethod::Dnf, vec![sh("exit 1", LONG, true)]),
                Attempt::Prebuilt(prebuilt_at(
                    &home,
                    "http://127.0.0.1:9/x".to_string(),
                    String::new(),
                )),
            ],
            ..Plan::default()
        };
        let progress = Progress::default();
        let (method, path, _) = execute(&plan, &progress, || async { resolves() })
            .await
            .unwrap();
        assert_eq!(method, InstallMethod::Dnf);
        assert_eq!(path, "/found/tmux");
        assert!(progress
            .lines()
            .contains(&"dnf said it failed, but tmux works.".to_string()));
        // No second tmux that `PATH` would never pick.
        assert!(!home.join(".local").exists());
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn a_failed_manager_whose_tmux_cant_start_ends_the_run() {
        let home = temp_dir();
        let plan = Plan {
            attempts: vec![
                manager(InstallMethod::Zypper, vec![sh("exit 1", LONG, true)]),
                Attempt::Prebuilt(prebuilt_at(
                    &home,
                    "http://127.0.0.1:9/x".to_string(),
                    String::new(),
                )),
            ],
            ..Plan::default()
        };
        let verify = || async {
            Err(Unverified::CantStart {
                path: "/usr/bin/tmux".to_string(),
                error: "no".to_string(),
            })
        };
        let error = execute(&plan, &Progress::default(), verify)
            .await
            .unwrap_err();
        assert!(error.contains("can't start"), "{error}");
        assert!(!home.join(".local").exists());
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn a_second_install_that_finds_the_first_ones_tmux_succeeds() {
        let home = temp_dir();
        let bin = home.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("tmux"), FAKE_TMUX).unwrap();
        let plan = Plan {
            attempts: vec![Attempt::Prebuilt(prebuilt_at(
                &home,
                "http://127.0.0.1:9/x".to_string(),
                String::new(),
            ))],
            ..Plan::default()
        };
        let (method, ..) = execute(&plan, &Progress::default(), || async { resolves() })
            .await
            .unwrap();
        assert_eq!(method, InstallMethod::Prebuilt);
        assert_eq!(std::fs::read(bin.join("tmux")).unwrap(), FAKE_TMUX);
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn an_install_that_loses_the_race_to_place_the_file_succeeds() {
        let home = temp_dir();
        let bin = home.join(".local/bin");
        let archive = archive_with("tmux", FAKE_TMUX);
        let sha = hex_sha256(&archive);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/x", listener.local_addr().unwrap());
        let winner = bin.clone();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|read| read > 2) {
                line.clear();
            }
            // The other process lands its tmux while this one downloads.
            std::fs::create_dir_all(&winner).unwrap();
            std::fs::write(winner.join("tmux"), b"the winner's").unwrap();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                archive.len()
            );
            let _ = stream.write_all(&archive);
        });
        let plan = Plan {
            attempts: vec![Attempt::Prebuilt(prebuilt_at(&home, url, sha))],
            ..Plan::default()
        };
        let target = bin.join("tmux");
        let verify = || {
            let found = when_there(&target);
            async move { found }
        };
        execute(&plan, &Progress::default(), verify).await.unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"the winner's");
        assert_eq!(files_in(&bin), vec!["tmux"]);
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn what_a_tmux_says_when_it_cant_start_goes_in_the_log_capped_and_clean() {
        let long = format!("\x1b[31m{}\nthe next line", "x".repeat(MAX_LINE_CHARS * 3));
        let plan = Plan {
            attempts: vec![manager(
                InstallMethod::Zypper,
                vec![sh("exit 0", LONG, true)],
            )],
            ..Plan::default()
        };
        let progress = Progress::default();
        let verify = || {
            let error = long.clone();
            async move {
                Err(Unverified::CantStart {
                    path: "/usr/bin/tmux".to_string(),
                    error,
                })
            }
        };
        execute(&plan, &progress, verify).await.unwrap_err();
        let lines = progress.lines();
        assert!(lines
            .iter()
            .all(|line| line.chars().count() <= MAX_LINE_CHARS + 1 && !line.contains('\x1b')));
        assert!(lines.contains(&"the next line".to_string()), "{lines:?}");
    }

    #[tokio::test]
    async fn a_tmux_already_there_only_counts_when_it_starts() {
        let dir = temp_dir();
        let broken = dir.join("broken");
        runnable_script(&broken, "#!/bin/sh\necho 'no room' >&2\nexit 1\n");
        let progress = Progress::default();
        let error = already_there_check(
            &progress,
            broken.to_string_lossy().into_owned(),
            "3.7c".to_string(),
        )
        .await
        .unwrap_err();
        assert!(error.contains("can't start"), "{error}");
        assert!(progress.lines().iter().any(|line| line.contains("no room")));

        let (fine, calls) = recording_tmux(&dir);
        let installed = already_there_check(
            &Progress::default(),
            fine.to_string_lossy().into_owned(),
            "3.7c".to_string(),
        )
        .await
        .unwrap();
        assert_eq!(installed.method, None);
        // Its check is on a private server, like any other.
        let calls = std::fs::read_to_string(calls).unwrap();
        assert!(
            calls
                .lines()
                .all(|line| line.starts_with("-u -L iii-check-")),
            "{calls}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_start_check_removes_its_own_socket() {
        let dir = temp_dir();
        let fake = dir.join("tmux");
        let seen = dir.join("seen");
        // Like tmux, it leaves the socket behind when its server ends.
        let body = format!(
            "#!/bin/sh\nsocket={0}/$3\ncase \"$*\" in\n*new-session*) : > \"$socket\"; echo \"$socket\" > {1};;\n*display-message*) echo \"$socket\";;\nesac\n",
            dir.display(),
            seen.display()
        );
        runnable_script(&fake, &body);
        start_check(&fake).await.unwrap();
        let socket = std::fs::read_to_string(&seen).unwrap();
        assert!(socket.contains("iii-check-"), "{socket}");
        assert!(!Path::new(socket.trim()).exists(), "{socket}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_start_check_runs_a_server_and_leaves_nothing_behind() {
        let Ok(real) = binary::resolve("tmux").await else {
            eprintln!("skipped: no tmux here");
            return;
        };
        let sockets = || {
            // SAFETY: getuid has no preconditions and cannot fail.
            let uid = unsafe { libc::getuid() };
            let tmp = std::env::var("TMUX_TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
            files_in(Path::new(&format!("{tmp}/tmux-{uid}")))
                .into_iter()
                .filter(|name| name.starts_with("iii-check-"))
                .count()
        };
        let before = sockets();
        start_check(&real.path).await.unwrap();
        assert_eq!(sockets(), before);
    }

    /// A script at `path` that can be run: a fork elsewhere in the test process
    /// may hold a new file open for a moment ("text file busy").
    fn runnable_script(path: &Path, body: &str) {
        std::fs::write(path, body).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        for _ in 0..100 {
            match std::process::Command::new(path).arg("-V").output() {
                Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => return,
            }
        }
    }

    /// A tmux that writes down how it was called and answers `-V`.
    fn recording_tmux(dir: &Path) -> (PathBuf, PathBuf) {
        let (fake, calls) = (dir.join("tmux"), dir.join("calls"));
        let body = format!(
            "#!/bin/sh\necho \"$*\" >> {}\n[ \"$1\" = -V ] && echo 'tmux 3.7c'\nexit 0\n",
            calls.display()
        );
        runnable_script(&fake, &body);
        let _ = std::fs::remove_file(&calls);
        (fake, calls)
    }

    #[tokio::test]
    async fn the_start_check_reports_what_tmux_said() {
        let dir = temp_dir();
        let fake = dir.join("tmux");
        runnable_script(
            &fake,
            "#!/bin/sh\necho \"couldn't create directory /run/tmux/0\" >&2\nexit 1\n",
        );
        let error = start_check(&fake).await.unwrap_err();
        assert!(error.contains("couldn't create directory"), "{error}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_start_check_only_ever_talks_to_its_own_private_server() {
        let dir = temp_dir();
        let (fake, calls) = recording_tmux(&dir);
        start_check(&fake).await.unwrap();
        let calls = std::fs::read_to_string(calls).unwrap();
        let lines: Vec<&str> = calls.lines().collect();
        assert_eq!(lines.len(), 3, "{calls}");
        let socket = lines[0].split(' ').nth(2).unwrap();
        assert!(socket.starts_with("iii-check-"), "{calls}");
        for line in &lines {
            assert!(
                line.starts_with(&format!("-u -L {socket} ")),
                "a call reached another server: {line}"
            );
        }
        assert!(lines[2].ends_with(" kill-server"), "{calls}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_start_check_removes_no_socket_but_its_own() {
        let dir = temp_dir();
        let someone_elses = dir.join("default");
        std::fs::write(&someone_elses, b"").unwrap();
        let fake = dir.join("tmux");
        runnable_script(
            &fake,
            &format!("#!/bin/sh\necho {}\n", someone_elses.display()),
        );
        start_check(&fake).await.unwrap();
        assert!(someone_elses.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn looking_for_tmux_again_talks_to_no_server() {
        let dir = temp_dir();
        let (fake, calls) = recording_tmux(&dir);
        let config = WorkerConfig {
            tmux_binary: fake.to_string_lossy().into_owned(),
            ..WorkerConfig::default()
        };
        let collector = Collector::new(config.into_shared(), ConsoleClients::default());
        let (path, version) = resolved(&collector).await.unwrap();
        assert_eq!(
            (path, version.as_str()),
            (fake.to_string_lossy().into_owned(), "3.7c")
        );
        assert_eq!(std::fs::read_to_string(calls).unwrap(), "-V\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_plan_without_attempts_answers_its_sentence() {
        let plan = Plan {
            otherwise: Some("Ask an administrator.".to_string()),
            ..Plan::default()
        };
        let error = execute(&plan, &Progress::default(), || async { resolves() })
            .await
            .unwrap_err();
        assert_eq!(error, "Ask an administrator.");
    }

    #[test]
    fn the_terminal_type_is_written_only_where_none_is_found() {
        let home = temp_dir();
        let dirs = terminfo_dirs(&home, None, None);
        assert_eq!(dirs[0], home.join(".terminfo"));
        assert!(!has_terminal_type(std::slice::from_ref(&dirs[0])));

        let macos_style = home.join("macos");
        std::fs::create_dir_all(macos_style.join("78")).unwrap();
        std::fs::write(macos_style.join("78").join(TERMINAL_TYPE), b"x").unwrap();
        assert!(has_terminal_type(std::slice::from_ref(&macos_style)));

        let linux_style = terminfo_dirs(
            &home,
            Some(macos_style.as_os_str()),
            Some(OsStr::new("/a:/b")),
        );
        assert_eq!(linux_style[0], macos_style);
        assert!(linux_style.contains(&PathBuf::from("/a")));
        assert!(linux_style.contains(&PathBuf::from("/usr/share/terminfo")));
        assert!(has_terminal_type(&linux_style));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn terminfo_settings_that_arent_absolute_are_ignored() {
        let home = temp_dir();
        let dirs = terminfo_dirs(
            &home,
            Some(OsStr::new("")),
            Some(OsStr::new(":rel:./x:/abs")),
        );
        assert!(dirs.iter().all(|dir| dir.is_absolute()), "{dirs:?}");
        assert_eq!(dirs[0], home.join(".terminfo"));
        assert!(dirs.contains(&PathBuf::from("/abs")));
        let relative = terminfo_dirs(&home, Some(OsStr::new("rel")), None);
        assert_eq!(relative[0], home.join(".terminfo"));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_write_that_fails_leaves_no_half_a_file() {
        let dir = temp_dir();
        let path = dir.join("entry");
        write_new_file(&path, |file| {
            file.write_all(b"half")?;
            Err(std::io::Error::other("disk full"))
        })
        .unwrap_err();
        assert_eq!(files_in(&dir), Vec::<String>::new());

        assert!(write_new_file(&path, |file| file.write_all(b"whole")).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), b"whole");
        assert_eq!(files_in(&dir), vec!["entry"]);

        // Never over what's there.
        assert!(!write_new_file(&path, |file| file.write_all(b"other")).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), b"whole");
        assert_eq!(files_in(&dir), vec!["entry"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn made_old(path: &Path) {
        let hours_ago = std::time::SystemTime::now() - LEFTOVER_AFTER * 2;
        std::fs::File::open(path)
            .unwrap()
            .set_modified(hours_ago)
            .unwrap();
    }

    #[test]
    fn only_old_temp_files_of_this_exact_name_are_swept() {
        let dir = temp_dir();
        let id = "0123456789abcdef0123456789abcdef";
        let leftover = dir.join(format!("{TEMP_PREFIX}{id}"));
        let not_ours = [
            format!("{TEMP_PREFIX}notes"),
            format!("{TEMP_PREFIX}{id}x"),
            format!("{TEMP_PREFIX}{}", "z".repeat(32)),
            "tmux".to_string(),
        ];
        std::fs::write(&leftover, b"x").unwrap();
        made_old(&leftover);
        for name in &not_ours {
            std::fs::write(dir.join(name), b"x").unwrap();
            made_old(&dir.join(name));
        }
        // Another process's, right now.
        let recent = format!("{TEMP_PREFIX}{}", "f".repeat(32));
        std::fs::write(dir.join(&recent), b"x").unwrap();
        // A folder of that name.
        let folder = format!("{TEMP_PREFIX}{}", "a".repeat(32));
        std::fs::create_dir(dir.join(&folder)).unwrap();
        made_old(&dir.join(&folder));

        sweep_leftovers(&dir, LEFTOVER_AFTER, &Progress::default());

        let mut left = files_in(&dir);
        left.sort();
        let mut expected: Vec<String> = not_ours.to_vec();
        expected.extend([recent, folder]);
        expected.sort();
        assert_eq!(left, expected);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_run_sweeps_what_a_killed_run_left() {
        let home = temp_dir();
        let bin = home.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let leftover = bin.join(format!("{TEMP_PREFIX}{}", "1".repeat(32)));
        std::fs::write(&leftover, b"x").unwrap();
        made_old(&leftover);
        let archive = archive_with("tmux", FAKE_TMUX);
        let sha = hex_sha256(&archive);
        let prebuilt = prebuilt_at(&home, serve_archive(archive), sha);
        run_prebuilt(&prebuilt, &Progress::default(), None)
            .await
            .unwrap();
        assert_eq!(files_in(&bin), vec!["tmux"]);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn the_terminal_type_is_a_real_compiled_entry() {
        // ncurses' 32-bit binary format, whose names start with the type.
        assert_eq!(&XTERM_256COLOR[..2], &[0x1e, 0x02]);
        assert_eq!(&XTERM_256COLOR[12..26], b"xterm-256color");
    }
}
