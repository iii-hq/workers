//! Linux listening sockets: LISTEN rows of `/proc/net/tcp{,6}` give socket
//! inodes, and `/proc/<pid>/fd/*` links (`socket:[<inode>]`) give their
//! owners. Only this user's processes have readable fd tables, which is
//! exactly the set worth showing.
//!
//! Reading every fd link of every process is the expensive part, so owners
//! are cached by inode and the fd walk only runs when a LISTEN inode has no
//! live cached owner.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::{parse_proc_net_tcp, ProcTable, SocketListener};
use crate::model::AddressFamily;

fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

fn socket_inodes(pid: u32) -> Vec<u64> {
    let Ok(entries) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let target = std::fs::read_link(entry.path()).ok()?;
            let target = target.to_str()?;
            target
                .strip_prefix("socket:[")?
                .strip_suffix(']')?
                .parse()
                .ok()
        })
        .collect()
}

pub fn open_files(pids: &[u32], keep: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = pids
        .iter()
        .filter_map(|pid| std::fs::read_dir(format!("/proc/{pid}/fd")).ok())
        .flat_map(|entries| entries.flatten())
        .filter_map(|entry| std::fs::read_link(entry.path()).ok())
        .filter(|target| target.is_absolute() && keep(target))
        .collect();
    out.sort();
    out.dedup();
    out
}

pub fn environ_var(pid: u32, key: &str) -> Option<String> {
    let environ = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    environ.split(|byte| *byte == 0).find_map(|entry| {
        let value = entry.strip_prefix(key.as_bytes())?.strip_prefix(b"=")?;
        Some(String::from_utf8_lossy(value).into_owned())
    })
}

pub fn start_ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // `pid (comm) state ppid …`: comm may hold spaces and parentheses, so
    // count fields after its closing parenthesis; starttime is field 22.
    let (_, rest) = stat.rsplit_once(')')?;
    rest.split_whitespace().nth(19)?.parse().ok()
}

pub fn listeners(
    table: &ProcTable,
    owners: &mut HashMap<u64, u32>,
) -> Result<Vec<SocketListener>, String> {
    let uid = current_uid();
    let mut rows = Vec::new();
    for (path, family) in [
        ("/proc/net/tcp", AddressFamily::Ipv4),
        ("/proc/net/tcp6", AddressFamily::Ipv6),
    ] {
        match std::fs::read_to_string(path) {
            Ok(content) => rows.extend(parse_proc_net_tcp(&content, family)),
            Err(error) if family == AddressFamily::Ipv6 => {
                tracing::debug!(%error, "no IPv6 socket table");
            }
            Err(error) => return Err(format!("cannot read {path}: {error}")),
        }
    }
    rows.retain(|row| uid == 0 || row.uid == uid);

    let live: HashSet<u64> = rows.iter().map(|row| row.inode).collect();
    owners.retain(|inode, pid| live.contains(inode) && table.get(*pid).is_some());
    if live.iter().any(|inode| !owners.contains_key(inode)) {
        owners.clear();
        for proc in table.iter() {
            for inode in socket_inodes(proc.pid) {
                if live.contains(&inode) {
                    // A socket inherited across fork shows up in several fd
                    // tables; the lowest pid is the one that opened it.
                    owners
                        .entry(inode)
                        .and_modify(|owner| *owner = (*owner).min(proc.pid))
                        .or_insert(proc.pid);
                }
            }
        }
    }

    let mut out: Vec<SocketListener> = rows
        .into_iter()
        .filter_map(|row| {
            Some(SocketListener {
                pid: *owners.get(&row.inode)?,
                port: row.port,
                address: row.address,
                family: row.family,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        (a.port, a.family as u8, &a.address).cmp(&(b.port, b.family as u8, &b.address))
    });
    out.dedup();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_socket_this_test_listens_on() {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        let mut scanner = super::super::Scanner::new();
        let table = scanner.processes();
        let mut owners = HashMap::new();
        let found = listeners(&table, &mut owners).unwrap();
        let mine = found
            .iter()
            .find(|listener| listener.port == port)
            .expect("the test's own listener is reported");
        assert_eq!(mine.pid, std::process::id());
        assert_eq!(mine.address, "127.0.0.1");
        // A second scan reuses the cached owners.
        let again = listeners(&table, &mut owners).unwrap();
        assert!(again.iter().any(|listener| listener.port == port));
    }

    #[test]
    fn reads_this_process_files_environment_and_start() {
        let path =
            std::env::temp_dir().join(format!("tmux-open-files-{}.jsonl", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        let found = open_files(&[std::process::id()], |p| p == path.as_path());
        drop(file);
        let _ = std::fs::remove_file(&path);
        assert_eq!(found, vec![path]);

        assert_eq!(
            environ_var(std::process::id(), "PATH"),
            std::env::var("PATH").ok()
        );
        assert_eq!(
            environ_var(std::process::id(), "TMUX_WORKER_NO_SUCH_VAR"),
            None
        );
        assert!(start_ticks(std::process::id()).is_some_and(|ticks| ticks > 0));
    }
}
