//! The process table and the TCP listening sockets of this user's processes.
//!
//! The table comes from `sysinfo` (Linux and macOS alike). Listening sockets
//! come from `/proc/net/tcp{,6}` plus each process's fd links on Linux, and
//! from `lsof` on macOS. Both are snapshots: the collector re-scans on its
//! own cadence and whenever the panes' foreground processes change.

use std::collections::{HashMap, HashSet, VecDeque};

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::model::AddressFamily;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod parse;

pub use parse::{parse_lsof, parse_proc_net_tcp, NetEntry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: Option<u32>,
    pub name: String,
    pub argv: Vec<String>,
    pub cwd: Option<String>,
}

impl ProcInfo {
    /// Names a process answers to: its name, and the basenames of argv[0]
    /// and argv[1] (an interpreter's script, e.g. `node …/codex.js`).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.name.as_str())
            .chain(self.argv.first().map(|arg| basename(arg)))
            .chain(self.argv.get(1).map(|arg| basename(arg)))
    }

    pub fn argv0(&self) -> &str {
        self.argv.first().map(String::as_str).unwrap_or(&self.name)
    }
}

fn basename(arg: &str) -> &str {
    let base = arg.rsplit('/').next().unwrap_or(arg);
    base.strip_suffix(".js")
        .or_else(|| base.strip_suffix(".mjs"))
        .unwrap_or(base)
}

#[derive(Debug, Clone, Default)]
pub struct ProcTable {
    procs: HashMap<u32, ProcInfo>,
    children: HashMap<u32, Vec<u32>>,
}

impl ProcTable {
    pub fn from_procs(procs: impl IntoIterator<Item = ProcInfo>) -> Self {
        let procs: HashMap<u32, ProcInfo> = procs.into_iter().map(|p| (p.pid, p)).collect();
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for proc in procs.values() {
            if let Some(ppid) = proc.ppid.filter(|ppid| *ppid != proc.pid) {
                children.entry(ppid).or_default().push(proc.pid);
            }
        }
        for list in children.values_mut() {
            list.sort_unstable();
        }
        Self { procs, children }
    }

    pub fn get(&self, pid: u32) -> Option<&ProcInfo> {
        self.procs.get(&pid)
    }

    pub fn children(&self, pid: u32) -> &[u32] {
        self.children.get(&pid).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn len(&self) -> usize {
        self.procs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.procs.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &ProcInfo> {
        self.procs.values()
    }

    /// `pid` and everything below it, breadth first (shallowest first), with depth.
    pub fn subtree(&self, pid: u32, max_depth: usize) -> Vec<(u32, usize)> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([(pid, 0)]);
        while let Some((current, depth)) = queue.pop_front() {
            if !seen.insert(current) {
                continue;
            }
            out.push((current, depth));
            if depth < max_depth {
                queue.extend(
                    self.children(current)
                        .iter()
                        .map(|child| (*child, depth + 1)),
                );
            }
        }
        out
    }

    /// `pid` and its ancestors, nearest first.
    pub fn lineage(&self, pid: u32) -> Vec<u32> {
        let mut out = vec![pid];
        let mut current = pid;
        while let Some(parent) = self.get(current).and_then(|p| p.ppid) {
            if parent == 0 || out.contains(&parent) || out.len() > 128 {
                break;
            }
            out.push(parent);
            current = parent;
        }
        out
    }
}

/// A TCP socket in LISTEN state owned by `pid`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketListener {
    pub port: u16,
    pub address: String,
    pub family: AddressFamily,
    pub pid: u32,
}

/// The two environment variables that tie a process to iii.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IiiEnv {
    pub worker: Option<String>,
    pub engine_url: Option<String>,
}

pub struct Scanner {
    system: System,
    #[cfg(target_os = "linux")]
    inode_owners: HashMap<u64, u32>,
}

impl Default for Scanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Scanner {
    pub fn new() -> Self {
        Self {
            system: System::new(),
            #[cfg(target_os = "linux")]
            inode_owners: HashMap::new(),
        }
    }

    /// Refresh and copy the process table. Threads are left out. Blocking.
    pub fn processes(&mut self) -> ProcTable {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .without_tasks()
                .with_cmd(UpdateKind::Always)
                .with_cwd(UpdateKind::OnlyIfNotSet),
        );
        ProcTable::from_procs(
            self.system
                .processes()
                .values()
                .filter(|p| p.thread_kind().is_none())
                .map(|process| ProcInfo {
                    pid: process.pid().as_u32(),
                    ppid: process.parent().map(Pid::as_u32),
                    name: process.name().to_string_lossy().into_owned(),
                    argv: process
                        .cmd()
                        .iter()
                        .map(|arg| arg.to_string_lossy().into_owned())
                        .collect(),
                    cwd: process.cwd().map(|cwd| cwd.to_string_lossy().into_owned()),
                }),
        )
    }

    /// `III_WORKER_NAME` / `III_URL` of the given processes (only those this
    /// user may read). Blocking.
    pub fn iii_env(&mut self, pids: &[u32]) -> HashMap<u32, IiiEnv> {
        if pids.is_empty() {
            return HashMap::new();
        }
        let wanted: Vec<Pid> = pids.iter().map(|pid| Pid::from_u32(*pid)).collect();
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&wanted),
            false,
            ProcessRefreshKind::nothing()
                .without_tasks()
                .with_environ(UpdateKind::OnlyIfNotSet),
        );
        let mut out = HashMap::new();
        for pid in &wanted {
            let Some(process) = self.system.process(*pid) else {
                continue;
            };
            let mut env = IiiEnv::default();
            for entry in process.environ() {
                let entry = entry.to_string_lossy();
                if let Some(value) = entry.strip_prefix("III_WORKER_NAME=") {
                    env.worker = Some(value.to_string()).filter(|v| !v.is_empty());
                } else if let Some(value) = entry.strip_prefix("III_URL=") {
                    env.engine_url = Some(value.to_string()).filter(|v| !v.is_empty());
                }
            }
            if env != IiiEnv::default() {
                out.insert(pid.as_u32(), env);
            }
        }
        out
    }

    /// TCP listening sockets with their owning pid. Blocking.
    #[cfg(target_os = "linux")]
    pub fn listeners(&mut self, table: &ProcTable) -> Result<Vec<SocketListener>, String> {
        linux::listeners(table, &mut self.inode_owners)
    }

    #[cfg(target_os = "macos")]
    pub fn listeners(&mut self, _table: &ProcTable) -> Result<Vec<SocketListener>, String> {
        macos::listeners()
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub fn listeners(&mut self, _table: &ProcTable) -> Result<Vec<SocketListener>, String> {
        Err("listening ports are not supported on this platform".to_string())
    }
}

pub const PORTS_SUPPORTED: bool = cfg!(any(target_os = "linux", target_os = "macos"));

#[cfg(test)]
mod tests {
    use super::*;

    pub fn proc(pid: u32, ppid: u32, name: &str, argv: &[&str]) -> ProcInfo {
        ProcInfo {
            pid,
            ppid: Some(ppid),
            name: name.to_string(),
            argv: argv.iter().map(|a| a.to_string()).collect(),
            cwd: None,
        }
    }

    #[test]
    fn subtree_is_breadth_first_with_depth() {
        let table = ProcTable::from_procs([
            proc(10, 1, "zsh", &["-zsh"]),
            proc(11, 10, "node", &["node", "/x/bin/codex.js"]),
            proc(12, 11, "codex", &["codex"]),
            proc(13, 10, "vim", &["vim"]),
        ]);
        assert_eq!(
            table.subtree(10, 8),
            vec![(10, 0), (11, 1), (13, 1), (12, 2)]
        );
        assert_eq!(table.subtree(10, 1), vec![(10, 0), (11, 1), (13, 1)]);
        assert_eq!(table.lineage(12), vec![12, 11, 10, 1]);
    }

    #[test]
    fn names_include_interpreter_scripts() {
        let p = proc(
            11,
            10,
            "node",
            &["node", "/usr/lib/node_modules/@openai/codex/bin/codex.js"],
        );
        assert_eq!(p.names().collect::<Vec<_>>(), vec!["node", "node", "codex"]);
    }

    #[test]
    fn scanning_this_machine_finds_this_process() {
        let mut scanner = Scanner::new();
        let table = scanner.processes();
        assert!(table.get(std::process::id()).is_some());
    }
}
