//! `iii compose` stacks and iii roles from the process table.
//!
//! The compose daemon titles itself `iii:c:<namespace>` and the engine it
//! manages `iii:e:<namespace>` (the full namespace stays in argv[0] when the
//! kernel name is truncated). Each container runs as `sh -c "<run>"` under
//! the daemon with `III_WORKER_NAME` in its environment, so a worker is the
//! daemon's subtree grouped by that name.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::model::{ComposeEngine, ComposeStack, ComposeWorker, ListenerRole};
use crate::procs::{IiiEnv, ProcInfo, ProcTable, SocketListener};

const DAEMON_PREFIX: &str = "iii:c:";
const ENGINE_PREFIX: &str = "iii:e:";
/// The engine's default WebSocket port range (`49134`, a second stack `49135`…).
const ENGINE_WS_PORTS: std::ops::RangeInclusive<u16> = 49_134..=49_199;

fn titled(proc: &ProcInfo, prefix: &str) -> Option<String> {
    [proc.argv0(), proc.name.as_str()]
        .into_iter()
        .find_map(|name| name.split_whitespace().next()?.strip_prefix(prefix))
        .map(str::to_string)
        .filter(|namespace| !namespace.is_empty())
}

/// Namespace of an `iii compose` daemon process.
pub fn daemon_namespace(proc: &ProcInfo) -> Option<String> {
    titled(proc, DAEMON_PREFIX)
}

/// Namespace of a compose-managed engine process.
pub fn engine_namespace(proc: &ProcInfo) -> Option<String> {
    titled(proc, ENGINE_PREFIX)
}

/// A standalone engine is the `iii` binary itself listening on a socket.
fn is_engine(proc: &ProcInfo) -> bool {
    engine_namespace(proc).is_some() || proc.name == "iii"
}

/// Pids whose iii environment is worth reading: compose descendants and
/// socket owners.
pub fn env_candidates(table: &ProcTable, listeners: &[SocketListener]) -> Vec<u32> {
    let mut pids: BTreeSet<u32> = listeners.iter().map(|listener| listener.pid).collect();
    for proc in table.iter() {
        if daemon_namespace(proc).is_some() {
            pids.extend(table.subtree(proc.pid, 8).into_iter().map(|(pid, _)| pid));
        }
    }
    pids.into_iter().collect()
}

fn compose_file(proc: &ProcInfo) -> Option<String> {
    let mut args = proc.argv.iter().skip(1);
    let mut file = None;
    while let Some(arg) = args.next() {
        if arg == "-f" || arg == "--file" {
            file = args.next().cloned();
        } else if let Some(value) = arg.strip_prefix("--file=") {
            file = Some(value.to_string());
        }
    }
    let base = proc.cwd.as_deref().map(std::path::Path::new);
    match (file, base) {
        (Some(file), Some(base)) => Some(base.join(file).to_string_lossy().into_owned()),
        (Some(file), None) => Some(file),
        (None, Some(base)) => {
            let default = base.join("worker-compose.yaml");
            default
                .is_file()
                .then(|| default.to_string_lossy().into_owned())
        }
        (None, None) => None,
    }
}

fn port_of(url: &str) -> Option<u16> {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next()?;
    authority.rsplit_once(':')?.1.parse().ok()
}

fn sorted_ports(ports: impl IntoIterator<Item = u16>) -> Vec<u16> {
    ports
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn ports_by_pid(listeners: &[SocketListener]) -> HashMap<u32, Vec<u16>> {
    let mut map: HashMap<u32, BTreeSet<u16>> = HashMap::new();
    for listener in listeners {
        map.entry(listener.pid).or_default().insert(listener.port);
    }
    map.into_iter()
        .map(|(pid, ports)| (pid, ports.into_iter().collect()))
        .collect()
}

/// Every running compose daemon, its engine and its workers (location is
/// filled in by the caller, who knows the panes).
pub fn stacks(
    table: &ProcTable,
    ports: &HashMap<u32, Vec<u16>>,
    env: &HashMap<u32, IiiEnv>,
) -> Vec<ComposeStack> {
    let mut out = Vec::new();
    for daemon in table.iter() {
        let Some(namespace) = daemon_namespace(daemon) else {
            continue;
        };
        let mut engine = None;
        let mut groups: BTreeMap<String, (u32, BTreeSet<u16>)> = BTreeMap::new();
        let mut engine_url = None;

        for &child in table.children(daemon.pid) {
            let Some(proc) = table.get(child) else {
                continue;
            };
            let subtree = table.subtree(child, 8);
            let subtree_ports = || {
                subtree
                    .iter()
                    .flat_map(|(pid, _)| ports.get(pid).into_iter().flatten().copied())
            };
            if engine_namespace(proc).is_some() {
                engine = Some((child, sorted_ports(subtree_ports())));
                continue;
            }
            // A container: name every process by its nearest named ancestor
            // inside this container, the container's own name otherwise.
            let mut owner_of: HashMap<u32, String> = HashMap::new();
            for (pid, _) in &subtree {
                let named = env.get(pid).and_then(|e| e.worker.clone());
                if engine_url.is_none() {
                    engine_url = env.get(pid).and_then(|e| e.engine_url.clone());
                }
                let owner = named.unwrap_or_else(|| {
                    table
                        .get(*pid)
                        .and_then(|p| p.ppid)
                        .and_then(|parent| owner_of.get(&parent).cloned())
                        .unwrap_or_else(|| proc.name.clone())
                });
                let group = groups
                    .entry(owner.clone())
                    .or_insert((*pid, BTreeSet::new()));
                group
                    .1
                    .extend(ports.get(pid).into_iter().flatten().copied());
                owner_of.insert(*pid, owner);
            }
        }

        let engine = engine.map(|(pid, ports)| {
            let declared = engine_url
                .as_deref()
                .and_then(port_of)
                .filter(|port| ports.contains(port));
            let ws_port = declared.or_else(|| {
                ports
                    .iter()
                    .copied()
                    .find(|port| ENGINE_WS_PORTS.contains(port))
            });
            ComposeEngine {
                pid,
                ports,
                ws_port,
            }
        });
        out.push(ComposeStack {
            namespace,
            pid: daemon.pid,
            project_dir: daemon.cwd.clone(),
            compose_file: compose_file(daemon),
            engine,
            workers: groups
                .into_iter()
                .map(|(name, (pid, ports))| ComposeWorker {
                    name,
                    pid,
                    ports: ports.into_iter().collect(),
                })
                .collect(),
            location: None,
        });
    }
    out.sort_by(|a, b| (&a.namespace, a.pid).cmp(&(&b.namespace, b.pid)));
    out
}

/// What kind of iii process owns a socket, and its worker / compose names.
pub fn classify_owner(
    table: &ProcTable,
    pid: u32,
    env: &HashMap<u32, IiiEnv>,
) -> (ListenerRole, Option<String>, Option<String>) {
    let lineage = table.lineage(pid);
    let compose = lineage
        .iter()
        .filter_map(|ancestor| table.get(*ancestor))
        .find_map(daemon_namespace);
    let worker = lineage
        .iter()
        .take_while(|ancestor| {
            table
                .get(**ancestor)
                .is_none_or(|p| daemon_namespace(p).is_none())
        })
        .find_map(|ancestor| env.get(ancestor).and_then(|e| e.worker.clone()));
    let role = match table.get(pid) {
        Some(proc) if daemon_namespace(proc).is_some() => ListenerRole::Compose,
        Some(proc) if is_engine(proc) => ListenerRole::Engine,
        _ if worker.is_some() => ListenerRole::Worker,
        _ => ListenerRole::Other,
    };
    (role, worker, compose)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AddressFamily;

    fn proc(pid: u32, ppid: u32, name: &str, argv: &[&str]) -> ProcInfo {
        ProcInfo {
            pid,
            ppid: Some(ppid),
            name: name.to_string(),
            argv: argv.iter().map(|a| a.to_string()).collect(),
            cwd: None,
        }
    }

    fn listen(pid: u32, port: u16) -> SocketListener {
        SocketListener {
            port,
            address: "0.0.0.0".into(),
            family: AddressFamily::Ipv4,
            pid,
        }
    }

    fn worker_env(name: &str) -> IiiEnv {
        IiiEnv {
            worker: Some(name.to_string()),
            engine_url: Some("ws://127.0.0.1:49135".to_string()),
        }
    }

    /// The tree `iii compose --up` left on a real machine: an engine, a
    /// worker started directly and two through `sh -c`.
    fn stack() -> (ProcTable, Vec<SocketListener>, HashMap<u32, IiiEnv>) {
        let mut daemon = proc(
            100,
            50,
            "iii:c:default",
            &["iii:c:default", "compose", "--up"],
        );
        daemon.cwd = Some("/work/onboarding".into());
        let table = ProcTable::from_procs([
            proc(50, 1, "zsh", &["-zsh"]),
            daemon,
            proc(101, 100, "iii:e:default", &["iii:e:default"]),
            proc(102, 100, "http", &["/pkgs/http"]),
            proc(103, 100, "sh", &["sh", "-c", "cargo run"]),
            proc(104, 103, "ade", &["target/debug/ade"]),
            proc(105, 100, "sh", &["sh", "-c", "exec bash run.sh"]),
            proc(106, 105, "secrets", &["secrets"]),
        ]);
        let listeners = vec![
            listen(101, 49135),
            listen(101, 3112),
            listen(102, 3111),
            listen(104, 3114),
        ];
        let env = HashMap::from([
            (102, worker_env("http")),
            (103, worker_env("ade")),
            (104, worker_env("ade")),
            (105, worker_env("secrets")),
            (106, worker_env("secrets")),
        ]);
        (table, listeners, env)
    }

    #[test]
    fn compose_stack_groups_workers_and_finds_the_engine_port() {
        let (table, listeners, env) = stack();
        let stacks = stacks(&table, &ports_by_pid(&listeners), &env);
        assert_eq!(stacks.len(), 1);
        let stack = &stacks[0];
        assert_eq!(stack.namespace, "default");
        assert_eq!(stack.project_dir.as_deref(), Some("/work/onboarding"));
        let engine = stack.engine.as_ref().unwrap();
        assert_eq!((engine.pid, engine.ws_port), (101, Some(49135)));
        assert_eq!(engine.ports, vec![3112, 49135]);
        let workers: Vec<(&str, u32, Vec<u16>)> = stack
            .workers
            .iter()
            .map(|w| (w.name.as_str(), w.pid, w.ports.clone()))
            .collect();
        assert_eq!(
            workers,
            vec![
                ("ade", 103, vec![3114]),
                ("http", 102, vec![3111]),
                ("secrets", 105, vec![])
            ]
        );
    }

    #[test]
    fn owners_are_classified() {
        let (table, _, env) = stack();
        assert_eq!(
            classify_owner(&table, 104, &env),
            (
                ListenerRole::Worker,
                Some("ade".into()),
                Some("default".into())
            )
        );
        assert_eq!(
            classify_owner(&table, 101, &env),
            (ListenerRole::Engine, None, Some("default".into()))
        );
        assert_eq!(
            classify_owner(&table, 50, &env),
            (ListenerRole::Other, None, None)
        );
    }

    #[test]
    fn truncated_titles_keep_the_namespace_from_argv0() {
        let p = proc(
            7,
            1,
            "iii:c:my~1a2b3c",
            &["iii:c:my-long-namespace", "compose"],
        );
        assert_eq!(daemon_namespace(&p).as_deref(), Some("my-long-namespace"));
        assert_eq!(port_of("ws://127.0.0.1:49134/"), Some(49134));
        assert_eq!(port_of("ws://localhost"), None);
    }

    #[test]
    fn compose_file_comes_from_the_flag() {
        let mut p = proc(
            7,
            1,
            "iii:c:x",
            &["iii:c:x", "compose", "--up", "-f", "stack.yaml"],
        );
        p.cwd = Some("/proj".into());
        assert_eq!(compose_file(&p).as_deref(), Some("/proj/stack.yaml"));
    }
}
