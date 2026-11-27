//! Run here like the cloud (spec §6): seven stages on the workspace's files,
//! the production compose file, the cloud's engine version, a local Postgres
//! for a database resource. Teardown always runs.
//!
//! Verified on iii 0.27.0 (a private engine, `engine::functions::list`):
//! - `registered-triggers::list` (the cloud's own call, iii-cloud
//!   `crates/core/src/engine.rs:16`) does not exist on a 0.27 engine: the
//!   trigger builtins were removed in 0.26 (iii `engine/src/missing_function_hints.rs:75`,
//!   `engine/tests/builtin_functions_e2e.rs:105`). An http route is an observer
//!   of `http::on-request` with `args {method, path}`, so the routes stage reads
//!   `engine::observers::list {function_id: "http::on-request"}`.
//! - `engine::workers::list` rows carry `id name namespace runtime status
//!   version isolation connected_at_ms function_count active_invocations os
//!   ip_address description`; engine-internal workers have `runtime: "engine"`.
//! - The cloud's healthcheck asks each replica of the http group
//!   `http://<addr><path>` with the path as set, any 2xx passes (iii-cloud
//!   `crates/platform/src/health.rs:80-142`): the path is never rewritten for
//!   `web.api.prefix`, strip or not.
//! - A managed Postgres is written as `{"databases": {"<alias>": {"url": ...}}}`
//!   in the database worker's config (`crates/control-plane/src/stages.rs:1232`).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::process::{Child, Command};
use tokio::sync::watch;

use crate::check::{default_not_covered, selected, update_check};
use crate::ctx::{ActiveRun, Ctx, new_id, now, short};
use crate::error::{CloudError, Res, cut};
use crate::local::{
    Manifest, database_binding, install_command, lock_for, runtime_base, worker_runtime,
};
use crate::tool;
use crate::types::*;

const MAX_RUNS_KEPT: usize = 20;
const TAIL: usize = 40;
/// Lines of a container's own log kept in a failed workers stage.
const CONTAINER_TAIL: usize = 10;
/// Seconds in the workers stage with no new worker before the run says who it waits on.
const NO_PROGRESS_S: u64 = 60;
/// What a `check {run: true}` call waits for a run, under the harness's 300 s dispatch timeout.
pub const CALL_WAIT: Duration = Duration::from_secs(240);
/// Where `iii compose` keeps the run's namespace, under the compose file's folder.
const COMPOSE_STATE: &str = ".iii/compose/iii-cloud-run";
/// How long one external command of the teardown or of `recover` (docker, `iii kill`) may take.
fn cleanup_timeout() -> Duration {
    // The env name is for tests, which cannot wait 30 s for a hung command.
    std::env::var("III_CLOUD_CLEANUP_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(Duration::from_secs(30), Duration::from_millis)
}

/// `CALL_WAIT` on the test tick's scale.
pub fn call_wait(ctx: &Ctx) -> Duration {
    ctx.secs(CALL_WAIT.as_secs())
}

/// When the process began, as `ps` prints it: with its pid, what names that very
/// process (a pid is reused after a reboot). Same on Linux and macOS.
pub fn proc_start(pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

/// Remove a container; true when it is gone (a name docker does not know counts).
async fn remove_container(name: &str) -> bool {
    let args = ["rm", "-f", name];
    let rm = tool::run("docker", &args, None, None, "DOCKER_MISSING");
    match tokio::time::timeout(cleanup_timeout(), rm).await {
        Ok(Ok(out)) => out.ok() || out.stderr.contains("No such container"),
        _ => false,
    }
}

/// The containers dead in this look and in the one before; `before` becomes this look's.
pub fn confirm_exited(before: &mut Vec<String>, now: Vec<String>) -> Vec<String> {
    let confirmed = now.iter().filter(|n| before.contains(n)).cloned().collect();
    *before = now;
    confirmed
}

/// Whether no process has this pid (unix; elsewhere nothing says so).
fn pid_gone(pid: u64) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only checks that the process exists.
        let rc = unsafe { libc::kill(pid as i32, 0) };
        rc != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// Kills a process group (the pid of its leader) with SIGKILL when dropped, so a
/// cancelled stage leaves nothing writing into the workspace. `.0 = None` disarms it.
struct GroupKill(Option<u32>);

impl Drop for GroupKill {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0 {
            // SAFETY: a signal to the group of a child this run started.
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
        }
    }
}

struct Fail {
    code: String,
    message: String,
}

fn fail(code: &str, message: impl Into<String>) -> Fail {
    Fail {
        code: code.into(),
        message: message.into(),
    }
}

struct Done {
    status: StageStatus,
    detail: Option<String>,
}

fn passed(detail: impl Into<Option<String>>) -> Result<Done, Fail> {
    Ok(Done {
        status: StageStatus::Passed,
        detail: detail.into(),
    })
}

// ------------------------------------------------------------ pure rules

pub fn parse_duration(text: &str) -> Option<Duration> {
    let text = text.trim();
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    let n: u64 = digits.parse().ok()?;
    match &text[digits.len()..] {
        "ms" => Some(Duration::from_millis(n)),
        "s" | "" => Some(Duration::from_secs(n)),
        "m" => Some(Duration::from_secs(n * 60)),
        "h" => Some(Duration::from_secs(n * 3600)),
        _ => None,
    }
}

/// `${NAME}`, `${NAME:-default}` and `${NAME:default}` from `env`; unset is empty.
pub fn interpolate(text: &str, env: &[(String, String)]) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = &after[..end];
        let (name, default) = match inner.split_once(":-").or_else(|| inner.split_once(':')) {
            Some((n, d)) => (n, d),
            None => (inner, ""),
        };
        out.push_str(
            env.iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
                .unwrap_or(default),
        );
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// The path the healthcheck asks: as set, with its leading `/`. The cloud asks
/// each replica directly, so `web.api.prefix` and `strip` change nothing.
pub fn healthcheck_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

/// With a kept prefix (`strip: false`) every route must sit under it.
pub fn routes_outside_prefix(routes: &[String], prefix: Option<&str>, strip: bool) -> Vec<String> {
    let Some(prefix) = prefix.filter(|_| !strip) else {
        return vec![];
    };
    let prefix = prefix.trim_end_matches('/');
    routes
        .iter()
        .filter(|r| {
            let path = r.split_once(' ').map(|(_, p)| p).unwrap_or(r);
            let path = format!("/{}", path.trim_start_matches('/'));
            path != prefix && !path.starts_with(&format!("{prefix}/"))
        })
        .cloned()
        .collect()
}

/// A database the run starts: its container, alias and the URL it is given.
pub struct Db {
    pub container: String,
    pub alias: String,
    pub url: String,
}

/// The run copy of the compose file (spec §6): the engine and the worker
/// manager move to `engine_port`, every `package://http` container moves to
/// its free port on 127.0.0.1, and every database alias points at its local
/// Postgres. Nothing else changes.
pub fn rewrite_run_copy(
    yaml: &Value,
    engine_port: u16,
    http_ports: &[(String, u16)],
    dbs: &[Db],
    extra_port: &mut dyn FnMut() -> u16,
) -> Result<Value, String> {
    validate_compose(yaml)?;
    let mut out = yaml.clone();
    let root = out.as_object_mut().ok_or("the compose file is not a map")?;
    let engine = root.entry("engine").or_insert_with(|| json!({}));
    if engine.is_null() {
        *engine = json!({});
    }
    engine["url"] = json!(format!("ws://127.0.0.1:{engine_port}"));
    if !engine.get("workers").is_some_and(Value::is_object) {
        engine["workers"] = json!({});
    }
    engine["workers"]["iii-worker-manager"] = json!({ "host": "127.0.0.1", "port": engine_port });
    // Every other engine worker that listens moves too, with or without a
    // port in the file (iii-stream defaults to 3112): the run never meets the
    // harness's ports.
    if let Some(workers) = engine["workers"].as_object_mut() {
        // Only iii-stream listens by default (3112); iii-http-functions takes no host or port.
        workers.entry("iii-stream").or_insert_with(|| json!({}));
        for (name, cfg) in workers.iter_mut() {
            let listens = name == "iii-stream" || cfg.get("port").is_some();
            if name != "iii-worker-manager" && listens {
                if !cfg.is_object() {
                    *cfg = json!({});
                }
                cfg["port"] = json!(extra_port());
                cfg["host"] = json!("127.0.0.1");
            }
        }
    }
    if let Some(containers) = root.get_mut("containers").and_then(Value::as_object_mut) {
        for (name, port) in http_ports {
            if let Some(c) = containers.get_mut(name) {
                set_override(c, &["port"], json!(port));
                set_override(c, &["host"], json!("127.0.0.1"));
            }
        }
        // The ADE console would take 3113.
        for c in containers.values_mut() {
            if Manifest::is_package(c, "ade") {
                set_override(c, &["http_port"], json!(extra_port()));
            }
        }
        for db in dbs {
            if let Some(c) = containers.get_mut(&db.container) {
                set_override(c, &["databases", &db.alias, "url"], json!(db.url));
            }
        }
    }
    Ok(out)
}

/// The shapes the rewrite indexes into. A compose file that does not have
/// them fails the run instead of panicking in the spawned task.
pub fn validate_compose(yaml: &Value) -> Result<(), String> {
    let map_or_absent = |v: Option<&Value>, what: &str| match v {
        None | Some(Value::Null) | Some(Value::Object(_)) => Ok(()),
        Some(_) => Err(format!("`{what}` in the compose file is not a map")),
    };
    if !yaml.is_object() {
        return Err("the compose file is not a map".into());
    }
    map_or_absent(yaml.get("engine"), "engine")?;
    map_or_absent(yaml.pointer("/engine/workers"), "engine.workers")?;
    map_or_absent(yaml.get("containers"), "containers")?;
    for (name, c) in yaml
        .get("containers")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        if !c.is_object() {
            return Err(format!("container `{name}` is not a map"));
        }
        map_or_absent(
            c.get("config_override"),
            &format!("containers.{name}.config_override"),
        )?;
        map_or_absent(
            c.pointer("/config_override/databases"),
            &format!("containers.{name}.config_override.databases"),
        )?;
        for (alias, db) in c
            .pointer("/config_override/databases")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            if !(db.is_object() || db.is_null()) {
                return Err(format!("database `{alias}` of `{name}` is not a map"));
            }
        }
    }
    Ok(())
}

fn set_override(container: &mut Value, path: &[&str], value: Value) {
    if !container
        .get("config_override")
        .is_some_and(Value::is_object)
    {
        container["config_override"] = json!({});
    }
    let mut cur = &mut container["config_override"];
    for key in &path[..path.len() - 1] {
        if !cur.get(*key).is_some_and(Value::is_object) {
            cur[*key] = json!({});
        }
        cur = &mut cur[*key];
    }
    cur[path[path.len() - 1]] = value;
}

/// Ports the harness uses: the run never takes them.
pub const FORBIDDEN_PORTS: [u16; 4] = [3111, 3112, 3113, 49134];

pub fn free_port() -> u16 {
    loop {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .unwrap_or(0);
        if !FORBIDDEN_PORTS.contains(&port) {
            return port;
        }
    }
}

/// Why `docker run` of the local Postgres failed: a taken port is its own code,
/// not a missing docker.
pub fn docker_failure(port: u16, stderr: &str) -> (&'static str, String) {
    let why = cut(stderr.trim(), 600);
    if why.contains("already allocated") || why.contains("address already in use") {
        (
            "PORT_IN_USE",
            format!(
                "Port {port} on this machine is taken, so Postgres could not start; check again. {why}"
            ),
        )
    } else {
        (
            "DOCKER_MISSING",
            format!("docker could not start Postgres: {why}"),
        )
    }
}

/// A free port no other run of this worker holds; kept in `ctx.ports` (and in
/// `held`, for the run to give back) until the run's teardown.
fn reserve_port(ctx: &Ctx, held: &mut Vec<u16>) -> u16 {
    loop {
        let port = free_port();
        if ctx.ports.lock().unwrap().insert(port) {
            held.push(port);
            return port;
        }
    }
}

fn on_path(bin: &str) -> bool {
    bin.contains('/')
        || std::env::var_os("PATH")
            .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
}

// ------------------------------------------------------------ the run

struct Prep {
    manifest: Manifest,
    root: PathBuf,
    settings: Value,
    run_env: Vec<(String, String)>,
    bindings: Vec<(String, String)>,
    engine_port: u16,
    http_ports: Vec<(String, u16)>,
    dbs: Vec<Db>,
    copy: PathBuf,
}

struct Exec {
    ctx: Arc<Ctx>,
    check_id: String,
    run: Run,
    dir: PathBuf,
    iii_bin: String,
    manifest_rel: Option<String>,
    workspace: PathBuf,
    env: EnvironmentInfo,
    settings: Value,
    problems: Vec<String>,
    /// Values the run must never show: `.env` values and bindings.
    secrets: Vec<String>,
    prep: Option<Prep>,
    pg: Vec<String>,
    child: Option<Child>,
    /// What `<manifest dir>/config/` held before the run's compose started:
    /// the files it writes there are removed after.
    config_before: Option<(PathBuf, Vec<String>)>,
    stage_started: Instant,
    /// True when a newer commit of the workspace asks this run to stop.
    cancel: watch::Receiver<bool>,
    /// The checks that attached to this run.
    followers: Arc<Mutex<Vec<String>>>,
    /// Ports this run picked, given back to `Ctx::ports` at the end.
    ports: Vec<u16>,
    /// The process group of the stage command now running, kept in `run.json`
    /// so `recover` can stop it after a crash.
    stage_group: Option<u32>,
    /// When that group's leader began (`proc_start`): it names the process itself.
    stage_start: Option<String>,
    /// What the worktree had changed when the run began (`set_aside` compares at its end).
    wrote_before: Option<std::collections::BTreeMap<String, String>>,
}

/// Start a run for a stored check. One at a time per workspace.
pub async fn start(ctx: &Arc<Ctx>, check_id: String) -> Res<Run> {
    let sel = selected(ctx)?;
    let until = Instant::now() + call_wait(ctx);
    start_with(ctx, check_id, sel, None, until).await
}

/// Start a run for a check, on the environment and worktree `sel` names (a
/// hand-off's own, which may not be the page's current selection).
///
/// A workspace has one run. A call whose environment and reuse `key` are those
/// of the running one attaches to it: the run publishes to the call's check
/// too, and the call returns when the run ends, says it waits on a worker, or
/// `until` passes. Anything else for the workspace (another commit, another
/// environment, no key) replaces the running one, after its teardown.
pub async fn start_with(
    ctx: &Arc<Ctx>,
    check_id: String,
    sel: crate::check::Selected,
    key: Option<String>,
    until: Instant,
) -> Res<Run> {
    let slot = sel.workspace.to_string_lossy().into_owned();
    let (id, cancel_rx, guard, followers) = 'slot: loop {
        let step = {
            let mut runs = ctx.runs.lock().unwrap();
            match runs.get(&slot) {
                // A run already told to stop is not attachable: the call waits for the slot.
                Some(a)
                    if key.is_some()
                        && a.key == key
                        && a.env_id == sel.env.id
                        && !*a.cancel.borrow() =>
                {
                    a.followers.lock().unwrap().push(check_id.clone());
                    Err((a.id.clone(), a.done.clone(), false))
                }
                Some(a) => {
                    let _ = a.cancel.send(true);
                    Err((a.id.clone(), a.done.clone(), true))
                }
                None => {
                    let id = new_id();
                    let (cancel_tx, cancel_rx) = watch::channel(false);
                    let (done_tx, done_rx) = watch::channel(false);
                    let followers = Arc::new(Mutex::new(vec![]));
                    runs.insert(
                        slot.clone(),
                        ActiveRun {
                            id: id.clone(),
                            env_id: sel.env.id.clone(),
                            key: key.clone(),
                            followers: followers.clone(),
                            cancel: cancel_tx,
                            done: done_rx,
                        },
                    );
                    Ok((
                        id.clone(),
                        cancel_rx,
                        ActiveGuard(ctx.clone(), slot.clone(), id, done_tx),
                        followers,
                    ))
                }
            }
        };
        match step {
            Ok(own) => break own,
            // Replaced: wait for its teardown, then take its place.
            Err((_, mut done, true)) => {
                let waited = tokio::time::timeout_at(until.into(), done.wait_for(|d| *d)).await;
                if waited.is_err() {
                    // Not this call's run: nothing started, and nothing may be keyed to the old one.
                    return Err(CloudError::new(
                        "RUN_TIMEOUT",
                        "The previous run is still stopping; check again.",
                    ));
                }
            }
            Err((id, done, false)) => {
                // What the run has now; from here its publishes reach this check.
                let current = ctx.store.read(|s| {
                    s.checks
                        .iter()
                        .filter_map(|c| c.run.clone())
                        .find(|r| r.id == id)
                });
                if let Some(run) = current.filter(|r| r.check_id != check_id) {
                    update_check(ctx, &check_id, |c| c.run = Some(run));
                }
                loop {
                    let run = ctx
                        .store
                        .read(|s| s.find_check(&check_id).and_then(|c| c.run.clone()))
                        .filter(|r| r.id == id);
                    let over = *done.borrow();
                    if let Some(run) = run {
                        if over
                            || run.status != RunStatus::Running
                            || run.waiting_on.is_some()
                            || Instant::now() >= until
                        {
                            return Ok(run);
                        }
                    } else if over {
                        continue 'slot;
                    } else if Instant::now() >= until {
                        return Err(CloudError::new(
                            "RUN_TIMEOUT",
                            "The run did not start in time; check again.",
                        ));
                    }
                    tokio::time::sleep(ctx.secs(1).min(Duration::from_millis(100))).await;
                }
            }
        }
    };
    // Read once the slot is ours: a commit made while the old run was torn down counts.
    let commit = workspace_commit(ctx, &sel.workspace).await?;
    // What was dirty before the run: what it changes or adds is the run's, not the agent's.
    // Decided now, not at the end: in the person's own folder nothing of this run is set aside
    // or put back, whatever the selection is by then.
    let before = if is_persons_folder(ctx, &sel.workspace) {
        None
    } else {
        let snapshot = dirty_snapshot(&sel.workspace).await;
        if snapshot.is_none() {
            mark_unrecorded(ctx, &sel.workspace);
        }
        snapshot
    };
    let mut exec = prepare(ctx, &id, &check_id, &sel, &commit).await?;
    exec.cancel = cancel_rx;
    exec.wrote_before = before.clone();
    exec.followers = followers.clone();
    exec.publish();
    let run = exec.run.clone();
    let ctx2 = ctx.clone();
    let wrote_ws = sel.workspace.clone();
    tokio::spawn(async move {
        // The workspace is free again however the run ends, a panic included.
        let _guard = guard;
        let panicked = tokio::spawn(async move { exec.drive().await })
            .await
            .is_err();
        if panicked {
            let updated = update_check(&ctx2, &check_id, |c| {
                if let Some(run) = c.run.as_mut().filter(|r| r.status == RunStatus::Running) {
                    let stage = run
                        .stages
                        .iter()
                        .find(|s| s.status == StageStatus::Running)
                        .map(|s| s.name)
                        .unwrap_or(StageName::Install);
                    run.status = RunStatus::Failed;
                    run.waiting_on = None;
                    run.finished_at = Some(now());
                    run.error = Some(RunError {
                        stage,
                        code: "RUN_PANIC".into(),
                        message: "The run stopped on an internal error.".into(),
                    });
                }
            });
            if let Some(run) = updated.and_then(|c| c.run) {
                put_in_followers(&ctx2, &followers, &check_id, &run);
                emit_run(&ctx2, &run);
            }
        }
        // However it ended (passed, failed, stopped): what it left dirty is set aside.
        // A run that panicked never reached its own `set_aside`.
        if panicked && let Some(before) = &before {
            let _ = set_aside(&ctx2, &wrote_ws, before).await;
        }
    });
    Ok(run)
}

/// The run's record into every check that attached to it.
fn put_in_followers(ctx: &Ctx, followers: &Mutex<Vec<String>>, owner: &str, run: &Run) {
    let mut ids = followers.lock().unwrap().clone();
    ids.sort();
    ids.dedup();
    for id in ids.iter().filter(|id| *id != owner) {
        update_check(ctx, id, |c| c.run = Some(run.clone()));
    }
}

/// Frees the workspace's slot and tells the waiters when dropped.
struct ActiveGuard(Arc<Ctx>, String, String, watch::Sender<bool>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        let mut runs = self.0.runs.lock().unwrap();
        if runs.get(&self.1).is_some_and(|a| a.id == self.2) {
            runs.remove(&self.1);
        }
        drop(runs);
        let _ = self.3.send(true);
    }
}

/// The most paths a run's leftovers are kept for; beyond that none are, and the run says so.
const MAX_WROTE: usize = 500;

/// The most bytes one run's `left/` holds; beyond it the remaining files stay set aside.
const LEFT_BYTES_MAX: u64 = 64 * 1024 * 1024;

/// Over this size a file is compared by size and mtime, not read.
const HASH_LIMIT: u64 = 4 * 1024 * 1024;

/// Every changed path of the worktree (`status --porcelain -z --untracked-files=all`, so an
/// untracked directory is its files) with a fingerprint of its content now: a hash (`-` when
/// it is gone; a symlink by its target; a file over `HASH_LIMIT` by size and mtime). With
/// `Fingerprints::None` they are empty (nothing is read); `Only(record)` only the paths the record
/// holds are read (no other path can match it). None when git can't say.
async fn dirty_entries(
    ws: &Path,
    which: Fingerprints<'_>,
) -> Option<std::collections::BTreeMap<String, String>> {
    let out = tool::git(
        ws,
        &["status", "--porcelain", "-z", "--untracked-files=all"],
    )
    .await
    .ok()?;
    let mut entries = out.split('\0').filter(|e| !e.is_empty());
    let mut paths = vec![];
    while let Some(e) = entries.next() {
        if e.len() < 4 {
            return None;
        }
        // A rename or copy lists the old path next: not a path of its own.
        if e.starts_with(['R', 'C']) || e[1..2].starts_with(['R', 'C']) {
            entries.next();
        }
        paths.push(e[3..].to_string());
    }
    let wanted: Option<std::collections::BTreeSet<String>> = match which {
        Fingerprints::All => None,
        Fingerprints::None => Some(Default::default()),
        Fingerprints::Only(record) => Some(record.keys().cloned().collect()),
    };
    let ws = ws.to_path_buf();
    tokio::task::spawn_blocking(move || {
        paths
            .into_iter()
            .map(|p| {
                let h = if wanted.as_ref().is_none_or(|w| w.contains(&p)) {
                    fingerprint(&ws.join(&p))
                } else {
                    String::new()
                };
                (p, h)
            })
            .collect()
    })
    .await
    .ok()
}

/// Which changed paths `dirty_entries` reads.
enum Fingerprints<'a> {
    All,
    None,
    Only(&'a std::collections::BTreeMap<String, String>),
}

fn fingerprint(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return "-".to_string();
    };
    if meta.file_type().is_symlink() {
        return match std::fs::read_link(path) {
            Ok(t) => format!("link:{}", t.display()),
            Err(_) => "-".to_string(),
        };
    }
    if meta.len() > HASH_LIMIT {
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        return format!("big:{}:{mtime}", meta.len());
    }
    match std::fs::read(path) {
        Ok(bytes) => hex::encode(Sha256::digest(bytes)),
        Err(_) => "-".to_string(),
    }
}

/// The one info line for the tracked files a run rewrote and the worker put back. Never their content.
pub fn restored_problem(restored: &[String], dir: &Path) -> Problem {
    let mut p = crate::local::here(
        "RUN_FILES_RESTORED",
        format!(
            "The local run rewrote {} tracked file{}; they are back as committed",
            restored.len(),
            if restored.len() == 1 { "" } else { "s" }
        ),
        Some(format!(
            "{}{}. The rewritten versions are in {}. If one of them was your own edit made during the run, apply it again and commit it. Compare with `diff {}/<path> <path>`.",
            restored
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            if restored.len() > 5 { " and more" } else { "" },
            dir.display(),
            dir.display()
        )),
        Owner::None,
        None,
    );
    p.severity = Severity::Info;
    p
}

/// What the worktree has changed now, to compare with when a run ends (`set_aside`).
/// None when git can't say: without a snapshot the run's end records nothing new.
pub async fn dirty_snapshot(ws: &Path) -> Option<std::collections::BTreeMap<String, String>> {
    dirty_entries(ws, Fingerprints::All).await
}

/// A run ended: what it changed or added since `before` is its own, not the agent's, as long as the
/// content stays what it left; what an earlier run left and is still there with that content stays
/// the run's (a run that writes nothing, or the same bytes again, drops nothing). A path that was
/// dirty before the run with content that was not the run's stays the agent's, even if the run
/// rewrote it. Nothing is deleted or restored. A file the agent edited WHILE the run went is
/// set aside too (the run can't tell it from its own): the check names every set-aside path
/// (`RUN_LEFT_FILES`) so one that is the agent's own work is seen. More than `MAX_WROTE` paths
/// keep none, and the line to put in the run's record says so (returned).
pub async fn set_aside(
    ctx: &Ctx,
    ws: &Path,
    before: &std::collections::BTreeMap<String, String>,
) -> Option<String> {
    set_aside_run(ctx, ws, before, None).await.note
}

/// What a run's end did to its worktree.
#[derive(Default)]
pub struct SetAside {
    /// A line for the run's record (more than `MAX_WROTE` paths).
    pub note: Option<String>,
    /// The tracked files put back as committed.
    pub restored: Vec<String>,
    /// Where their rewritten versions were kept.
    pub saved_in: Option<PathBuf>,
}

/// `set_aside`, and with the run's folder: each tracked file that was clean when the run began and
/// that it rewrote or deleted is saved under `<run folder>/left/<path>` (0600, folders 0700) and
/// put back with `git checkout -- <path>`, only once the save worked. Never in the person's own
/// folder; a file over `HASH_LIMIT`, or more than `MAX_WROTE` files, stay set aside as they are;
/// a path dirty before the run is never touched; untracked files are not deleted. What a run kept in
/// its `left/` goes with its folder (the prune of old run folders), not before: the agent's next run
/// must not take away a version it has not looked at yet.
pub async fn set_aside_run(
    ctx: &Ctx,
    ws: &Path,
    before: &std::collections::BTreeMap<String, String>,
    run_dir: Option<&Path>,
) -> SetAside {
    // The person's own folder: every change in it is theirs to judge, none is the run's.
    if is_persons_folder(ctx, ws) {
        return SetAside::default();
    }
    let Some(mut after) = dirty_entries(ws, Fingerprints::All).await else {
        mark_unrecorded(ctx, ws);
        return SetAside::default();
    };
    let previous = ctx
        .store
        .read(|s| s.run_wrote.get(&ws.to_string_lossy().into_owned()).cloned())
        .unwrap_or_default();
    // Tracked files the run rewrote or deleted, clean when it began: back as committed.
    let mut restored: Vec<String> = vec![];
    let mut left_dir: Option<PathBuf> = None;
    if let Some(run_dir) = run_dir {
        let tracked: std::collections::BTreeSet<String> =
            tool::git(ws, &["ls-files", "-z", "--modified", "--deleted"])
                .await
                .map(|o| {
                    o.split('\0')
                        .filter(|p| !p.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
        let candidates: Vec<String> = after
            .keys()
            .filter(|p| !before.contains_key(*p) && tracked.contains(*p))
            .cloned()
            .collect();
        if !candidates.is_empty() && candidates.len() <= MAX_WROTE {
            let dir = run_dir.join("left");
            let (ws2, dir2) = (ws.to_path_buf(), dir.clone());
            // Save each file, then check again that it still is what was saved: what is put back
            // is exactly what `left/` holds. (Blocking work, off the runtime.)
            let saved: Vec<String> = tokio::task::spawn_blocking(move || {
                let mut total = 0u64;
                let mut kept: Vec<(String, String)> = vec![];
                for p in candidates {
                    let Some((hash, len)) = save_left(&ws2, &dir2, &p) else {
                        continue;
                    };
                    if total + len > LEFT_BYTES_MAX {
                        // Over the cap: not put back, stays set aside.
                        let _ = std::fs::remove_file(dir2.join(&p));
                        continue;
                    }
                    total += len;
                    kept.push((p, hash));
                }
                recheck(&ws2, &dir2, kept)
            })
            .await
            .unwrap_or_default();
            restored = put_back(ws, &saved).await;
            for p in &restored {
                after.remove(p);
            }
            if !restored.is_empty() {
                left_dir = Some(dir);
            }
        }
    }
    let mut files: std::collections::BTreeMap<String, String> = after
        .into_iter()
        .filter(|(p, _)| {
            before
                .get(p)
                .is_none_or(|b| previous.files.get(p) == Some(b))
        })
        .collect();
    let note = (files.len() > MAX_WROTE).then(|| {
        format!(
            "The run left more than {MAX_WROTE} changed files in the worktree; none are set aside, so all of them count as uncommitted changes."
        )
    });
    if note.is_some() {
        files.clear();
    }
    ctx.store.update(|s| {
        // A record of a worktree that is gone goes with it.
        s.run_wrote.retain(|p, _| Path::new(p).exists());
        s.run_unrecorded.retain(|p| Path::new(p).exists());
        s.run_wrote.insert(
            ws.to_string_lossy().into_owned(),
            crate::store::RunWrote {
                files,
                restored: restored.clone(),
                left_dir: left_dir.as_ref().map(|d| d.to_string_lossy().into_owned()),
            },
        );
    });
    SetAside {
        note,
        restored,
        saved_in: left_dir,
    }
}

/// Right before the checkout: each saved file is hashed again. One whose content changed since its
/// save is saved again, and put back only if it holds still: `left/` always has the bytes that
/// the checkout replaces.
fn recheck(ws: &Path, dir: &Path, kept: Vec<(String, String)>) -> Vec<String> {
    kept.into_iter()
        .filter_map(|(p, hash)| {
            if fingerprint(&ws.join(&p)) == hash {
                return Some(p);
            }
            let (again, _) = save_left(ws, dir, &p)?;
            (fingerprint(&ws.join(&p)) == again).then_some(p)
        })
        .collect()
}

/// Put the saved files back as committed: ONE `git checkout` with literal pathspecs (a name with
/// `[`, `*`, `?` or a leading `:` is a name, not a pattern), the paths on stdin. An old git without
/// `--pathspec-from-file`, or a refusal of the batch, falls back to one literal checkout per file.
/// The paths that are back.
async fn put_back(ws: &Path, paths: &[String]) -> Vec<String> {
    if paths.is_empty() {
        return vec![];
    }
    let mut list = paths.join("\0").into_bytes();
    list.push(0);
    let batch = tool::run(
        "git",
        &[
            "--literal-pathspecs",
            "checkout",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
        ],
        Some(ws),
        Some(&list),
        "TOOL_MISSING",
    )
    .await;
    if batch.is_ok_and(|o| o.ok()) {
        return paths.to_vec();
    }
    let mut back = vec![];
    for p in paths {
        if tool::git(ws, &["--literal-pathspecs", "checkout", "--", p])
            .await
            .is_ok()
        {
            back.push(p.clone());
        }
    }
    back
}

/// Every parent of `rel` that exists in `ws` is a real directory (not a file, not a symlink).
fn parents_are_directories(ws: &Path, rel: &str) -> bool {
    let mut at = ws.to_path_buf();
    let parts: Vec<&str> = rel.split('/').collect();
    for part in &parts[..parts.len().saturating_sub(1)] {
        at.push(part);
        match std::fs::symlink_metadata(&at) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => return false,
            // A parent that is gone too: the whole directory was deleted.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => return false,
        }
    }
    true
}

/// Keep what the run left of one tracked file under `dir` (0600, folders 0700): the hash of what was
/// kept (`-` for a deletion: nothing to keep) and its size. None when it can't be kept: the file
/// then stays as the run left it.
fn save_left(ws: &Path, dir: &Path, rel: &str) -> Option<(String, u64)> {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let from = ws.join(rel);
    let meta = match std::fs::symlink_metadata(&from) {
        Ok(m) => m,
        // Deleted: only when it is NotFound and no parent that exists is a file or a link (a file
        // where a directory was is the run's own data: it is not for a checkout to remove).
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && parents_are_directories(ws, rel) => {
            return Some(("-".to_string(), 0));
        }
        Err(_) => return None,
    };
    if !meta.is_file() || meta.len() > HASH_LIMIT {
        return None;
    }
    let to = dir.join(rel);
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(to.parent()?)
        .ok()?;
    let bytes = std::fs::read(&from).ok()?;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&to)
        .and_then(|mut f| f.write_all(&bytes))
        .ok()?;
    Some((hex::encode(Sha256::digest(&bytes)), bytes.len() as u64))
}

/// The changed paths of the worktree that are the agent's: not what the last local run left
/// there (a path counts again once its content is not what the run left). None when git
/// can't say: that counts as dirty.
pub async fn dirty_paths(ctx: &Ctx, ws: &Path) -> Option<Vec<String>> {
    dirty_split(ctx, ws).await.map(|(agent, _)| agent)
}

/// A run ended in `ws` without its leftovers recorded: until the worktree is clean, what is
/// dirty there may be partly the run's.
fn mark_unrecorded(ctx: &Ctx, ws: &Path) {
    ctx.store
        .update(|s| s.run_unrecorded.insert(ws.to_string_lossy().into_owned()));
}

/// Set-aside paths that are tracked and modified or deleted now (a rebase, merge or checkout
/// would stop on them).
pub async fn tracked_aside_dirty(ctx: &Ctx, ws: &Path) -> bool {
    let aside: Vec<String> = ctx.store.read(|s| {
        s.run_wrote
            .get(&ws.to_string_lossy().into_owned())
            .map(|w| w.files.keys().cloned().collect())
            .unwrap_or_default()
    });
    if aside.is_empty() {
        return false;
    }
    tool::git(ws, &["ls-files", "-z", "--modified", "--deleted"])
        .await
        .is_ok_and(|o| o.split('\0').any(|p| aside.iter().any(|a| a == p)))
}

/// The worktree has a run's unrecorded leftovers (see `mark_unrecorded`).
pub fn unrecorded(ctx: &Ctx, ws: &Path) -> bool {
    ctx.store.read(|s| {
        s.run_unrecorded
            .contains(&ws.to_string_lossy().into_owned())
    })
}

/// The workspace is the person's own folder (checkout mode), not a worktree of the worker's.
fn is_persons_folder(ctx: &Ctx, ws: &Path) -> bool {
    ctx.store
        .read(|s| s.project.as_ref().is_some_and(|p| Path::new(&p.path) == ws))
}

/// `(the agent's changed paths, the paths set aside as the last run's)`; None when git can't say.
pub async fn dirty_split(ctx: &Ctx, ws: &Path) -> Option<(Vec<String>, Vec<String>)> {
    let left = ctx
        .store
        .read(|s| s.run_wrote.get(&ws.to_string_lossy().into_owned()).cloned())
        .filter(|_| !is_persons_folder(ctx, ws));
    // A worktree no run has left anything in (an empty record is none): every changed path is the
    // agent's, and nothing is read. With a record, only its paths are.
    let left = left.filter(|l| !l.files.is_empty());
    let now = match &left {
        Some(l) => dirty_entries(ws, Fingerprints::Only(&l.files)).await?,
        None => dirty_entries(ws, Fingerprints::None).await?,
    };
    let left = left.unwrap_or_default();
    let (aside, agent): (Vec<_>, Vec<_>) = now
        .into_iter()
        .partition(|(p, h)| left.files.get(p) == Some(h));
    // Nothing of the agent's is dirty: whatever a run left unrecorded is gone or set aside.
    if agent.is_empty() && unrecorded(ctx, ws) {
        ctx.store
            .update(|s| s.run_unrecorded.remove(&ws.to_string_lossy().into_owned()));
    }
    Some((
        agent.into_iter().map(|(p, _)| p).collect(),
        aside.into_iter().map(|(p, _)| p).collect(),
    ))
}

/// What the run proves: HEAD, and `+dirty` when the agent has anything uncommitted. Ready
/// needs it to equal the checked commit.
async fn workspace_commit(ctx: &Ctx, workspace: &Path) -> Res<String> {
    let head = crate::tool::git(workspace, &["rev-parse", "HEAD"])
        .await?
        .trim()
        .to_string();
    let dirty = dirty_paths(ctx, workspace)
        .await
        .is_none_or(|p| !p.is_empty());
    Ok(if dirty { format!("{head}+dirty") } else { head })
}

/// The run's event: terminal once it is no longer running.
fn emit_run(ctx: &Ctx, run: &Run) {
    let summary = match (&run.status, &run.error) {
        (RunStatus::Failed, Some(e)) => format!(
            "run {} failed at {:?}: {}",
            short(&run.commit),
            e.stage,
            e.code
        ),
        (RunStatus::Passed, _) => format!("run {} passed", short(&run.commit)),
        _ => format!("run {} is going", short(&run.commit)),
    };
    ctx.emit(
        EventKind::Run,
        &run.id,
        run.status != RunStatus::Running,
        summary,
        run,
    );
}

async fn prepare(
    ctx: &Arc<Ctx>,
    id: &str,
    check_id: &str,
    sel: &crate::check::Selected,
    commit: &str,
) -> Res<Exec> {
    let check = ctx
        .store
        .read(|s| s.find_check(check_id).cloned())
        .ok_or_else(|| CloudError::invalid("no such check"))?;
    let sel = sel.clone();
    let cfg = ctx.config().await;
    let env_body = ctx
        .call(&crate::cloud::Req::get(format!(
            "/teams/{}/environments/{}",
            sel.team_id, sel.env.id
        )))
        // No environment settings, no run: a null here would skip the release,
        // the root directory and the healthcheck without a word.
        .await?;
    let dir = cfg.data_path().join("runs").join(id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| CloudError::new("GIT_FAILED", format!("cannot create the run folder: {e}")))?;
    let engine = crate::check::local_iii_version(&cfg.iii_bin)
        .await
        .unwrap_or_else(|| "unknown".into());
    let run = Run {
        id: id.to_string(),
        check_id: check.id.clone(),
        commit: commit.to_string(),
        engine,
        status: RunStatus::Running,
        started_at: now(),
        finished_at: None,
        stages: StageName::ALL
            .iter()
            .map(|n| Stage {
                name: *n,
                status: StageStatus::Pending,
                detail: None,
                duration_ms: None,
            })
            .collect(),
        workers: WorkerCount { ready: 0, total: 0 },
        routes: RouteCount {
            count: 0,
            prefix: None,
        },
        healthcheck: None,
        not_covered: default_not_covered(),
        log_tail: vec![],
        error: None,
        reused: false,
        waiting_on: None,
        restored: vec![],
    };
    set_run_live(&dir, true);
    Ok(Exec {
        ctx: ctx.clone(),
        check_id: check.id.clone(),
        run,
        dir,
        iii_bin: cfg.iii_bin.clone(),
        // The file the check was of: another one than the environment's runs too.
        // Else the cloud's setting as it is now (a null one is none, as in `check`).
        manifest_rel: check.manifest.clone().or_else(|| {
            env_body
                .get("manifest")
                .and_then(Value::as_str)
                .map(str::to_owned)
        }),
        workspace: sel.workspace,
        env: sel.env,
        settings: env_body.get("settings").cloned().unwrap_or(Value::Null),
        problems: check.problems.iter().map(|p| p.code.clone()).collect(),
        secrets: vec![],
        prep: None,
        pg: vec![],
        child: None,
        config_before: None,
        wrote_before: None,
        stage_started: Instant::now(),
        cancel: watch::channel(false).1,
        followers: Arc::new(Mutex::new(vec![])),
        ports: vec![],
        stage_group: None,
        stage_start: None,
    })
}

impl Drop for Exec {
    /// A panic skips `teardown`: remove what is left, without awaiting.
    fn drop(&mut self) {
        set_run_live(&self.dir, false);
        self.give_back_ports();
        // A graceful stop drops the task: the stage's group was killed with it, so
        // `run.json` must not keep an id that may name another process by the next start.
        if self.stage_group.take().is_some() {
            self.stage_start = None;
            self.persist_run();
        }
        // `Drop` never waits on an external command: a hung docker daemon would hold the
        // workspace's slot for ever. The names stay in `run.json` (the next teardown or
        // `recover` removes them); a thread tries now without anyone waiting for it.
        let names = std::mem::take(&mut self.pg);
        if !names.is_empty() {
            std::thread::spawn(move || {
                for name in names {
                    let _ = std::process::Command::new("docker")
                        .args(["rm", "-f", &name])
                        .output();
                }
            });
        }
        if let Some(p) = &self.prep {
            clean_copy(&p.copy);
        }
    }
}

impl Exec {
    fn give_back_ports(&mut self) {
        let mut held = self.ctx.ports.lock().unwrap();
        for port in self.ports.drain(..) {
            held.remove(&port);
        }
    }

    fn persist_run(&self) {
        let files = RunFiles {
            containers: self.pg.clone(),
            // Only a copy that exists: after `clean_copy` nothing is left to `iii kill`.
            copy: self
                .prep
                .as_ref()
                .map(|p| p.copy.clone())
                .filter(|c| c.exists()),
            iii_bin: self.iii_bin.clone(),
            stage_group: self.stage_group,
            stage_start: self.stage_start.clone(),
        };
        if let Ok(bytes) = serde_json::to_vec(&files) {
            let _ = std::fs::write(self.dir.join("run.json"), bytes);
        }
    }

    fn redact(&self, text: &str) -> String {
        redact(text, &self.secrets)
    }

    fn publish(&mut self) {
        self.run.log_tail = tail_of(&self.dir.join("run.log"))
            .iter()
            .map(|l| cut(&self.redact(l), 400))
            .collect();
        let mut run = self.run.clone();
        for stage in &mut run.stages {
            stage.detail = stage.detail.as_deref().map(|d| self.redact(d));
        }
        if let Some(e) = run.error.as_mut() {
            e.message = self.redact(&e.message);
        }
        update_check(&self.ctx, &self.check_id, |c| c.run = Some(run.clone()));
        put_in_followers(&self.ctx, &self.followers, &self.check_id, &run);
        emit_run(&self.ctx, &run);
    }

    fn set(&mut self, name: StageName, status: StageStatus, detail: Option<String>) {
        let ms = self.stage_started.elapsed().as_millis() as u64;
        if let Some(stage) = self.run.stages.iter_mut().find(|s| s.name == name) {
            stage.status = status;
            stage.detail = detail;
            stage.duration_ms = (status != StageStatus::Running).then_some(ms);
        }
        self.stage_started = Instant::now();
        self.publish();
    }

    fn note(&self, line: &str) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("run.log"))
        {
            let _ = writeln!(f, "{}", self.redact(line));
        }
    }

    /// A child's stdout and stderr into the run folder's `file`, redacted on
    /// the way: no log file of the run holds a value it knows.
    fn pump_output(&self, child: &mut Child, file: &str) -> Vec<tokio::task::JoinHandle<()>> {
        let path = self.dir.join(file);
        let secrets = Arc::new(self.secrets.clone());
        let mut pumps = vec![];
        if let Some(out) = child.stdout.take() {
            pumps.push(tokio::spawn(pump(out, path.clone(), secrets.clone())));
        }
        if let Some(err) = child.stderr.take() {
            pumps.push(tokio::spawn(pump(err, path, secrets)));
        }
        pumps
    }

    async fn drive(&mut self) {
        let budget = Duration::from_secs(self.ctx.config().await.run_timeout_s);
        let mut cancel = self.cancel.clone();
        // None: a newer commit of the workspace replaced this run. Dropping
        // the stages stops their children; teardown does the rest.
        let outcome = tokio::select! {
            r = tokio::time::timeout(budget, self.stages()) => Some(r),
            _ = cancel.wait_for(|c| *c) => None,
        };
        match outcome {
            Some(Ok(Ok(()))) => self.run.status = RunStatus::Passed,
            Some(Ok(Err((stage, f)))) => self.finish_failed(stage, f),
            other => {
                let stage = self
                    .run
                    .stages
                    .iter()
                    .find(|s| s.status == StageStatus::Running)
                    .map(|s| s.name)
                    .unwrap_or(StageName::Install);
                let f = if other.is_none() {
                    fail(
                        "RUN_REPLACED",
                        "A newer commit of this workspace started a run, so this one stopped.",
                    )
                } else {
                    fail(
                        "RUN_TIMEOUT",
                        format!("The run passed {} s.", budget.as_secs()),
                    )
                };
                self.finish_failed(stage, f);
            }
        }
        self.teardown().await;
        // What the run left in the worktree is set aside before its last event goes out.
        if let Some(before) = self.wrote_before.take() {
            let done = set_aside_run(
                &self.ctx,
                &self.workspace,
                &before,
                Some(self.dir.as_path()),
            )
            .await;
            if let Some(note) = done.note {
                // Into the run's own log, so the last publish carries it (it rebuilds `log_tail` from there).
                self.note(&note);
            }
            if !done.restored.is_empty() {
                self.run.restored = done.restored.clone();
                if let Some(dir) = &done.saved_in {
                    let problem = restored_problem(&done.restored, dir);
                    let mut ids = self.followers.lock().unwrap().clone();
                    ids.push(self.check_id.clone());
                    ids.sort();
                    ids.dedup();
                    for id in ids {
                        // This run's notice replaces the one a check carried from the run before.
                        update_check(&self.ctx, &id, |c| {
                            c.problems.retain(|p| p.code != problem.code);
                            c.problems.push(problem.clone());
                        });
                    }
                }
            }
        }
        self.run.finished_at = Some(now());
        self.publish();
    }

    fn finish_failed(&mut self, stage: StageName, f: Fail) {
        self.run.waiting_on = None;
        self.note(&format!("!! {:?} failed: {}: {}", stage, f.code, f.message));
        self.set(stage, StageStatus::Failed, Some(cut(&f.message, 200)));
        self.run.status = RunStatus::Failed;
        self.run.error = Some(RunError {
            stage,
            code: f.code,
            message: cut(&f.message, 600),
        });
    }

    async fn stages(&mut self) -> Result<(), (StageName, Fail)> {
        for name in StageName::ALL {
            self.stage_started = Instant::now();
            self.set(name, StageStatus::Running, None);
            self.stage_started = Instant::now();
            let result = match name {
                StageName::Install => self.install().await,
                StageName::Build => self.build().await,
                StageName::Release => self.release().await,
                StageName::ComposeUp => self.compose_up().await,
                StageName::Workers => self.workers().await,
                StageName::Routes => self.routes().await,
                StageName::Healthcheck => self.healthcheck().await,
            };
            match result {
                Ok(done) => self.set(name, done.status, done.detail),
                Err(f) => return Err((name, f)),
            }
        }
        Ok(())
    }

    /// Everything the later stages share, computed in `install`.
    fn prep(&mut self) -> Result<(), Fail> {
        let manifest =
            Manifest::load(&self.workspace, self.manifest_rel.as_deref()).ok_or_else(|| {
                fail(
                    "INSTALL_FAILED",
                    "The compose file is missing or is not YAML.",
                )
            })?;
        let root = self.workspace.join(
            self.settings
                .get("root_directory")
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        let mut run_env = crate::local::worktree_env(&self.workspace, &manifest);
        // What the run's `.env` lacks, from the person's folder's env files:
        // into the run's environment only, and known to redaction.
        let checkout = self
            .ctx
            .store
            .read(|s| s.project.as_ref().map(|p| PathBuf::from(&p.path)))
            .unwrap_or_else(|| self.workspace.clone());
        // The same plan the check read, saved with it: what iii cloud will resolve.
        let cloud_set =
            std::fs::read_to_string(crate::deploy::plan_file(&self.ctx, &self.check_id))
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .map(|p| crate::values::cloud_set_in(&p))
                .unwrap_or_default();
        let filled = crate::values::here_values(
            &checkout,
            &self.workspace,
            self.manifest_rel.as_deref(),
            &self.env,
            &cloud_set,
        );
        {
            let mut known = self.ctx.known_secrets.lock().unwrap();
            for (_, value, _) in &filled {
                if value.len() >= 3 && !known.contains(value) {
                    known.push(value.clone());
                }
            }
        }
        run_env.extend(filled.into_iter().map(|(name, value, _)| (name, value)));
        // A name skipped for now with no value runs with an empty one (process env only).
        for name in crate::values::skipped_of(&self.ctx, &self.env.id) {
            if !run_env.iter().any(|(n, _)| *n == name) {
                run_env.push((name, String::new()));
            }
        }
        self.secrets.extend(run_env.iter().map(|(_, v)| v.clone()));
        self.secrets
            .extend(self.ctx.known_secrets.lock().unwrap().iter().cloned());
        let http_ports = manifest
            .containers()
            .into_iter()
            .filter(|(n, c)| manifest.is_http_worker(n, c))
            .map(|(n, _)| (n, reserve_port(&self.ctx, &mut self.ports)))
            .collect();
        let engine_port = reserve_port(&self.ctx, &mut self.ports);
        let stem = manifest
            .file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let copy = manifest.dir.join(format!("{stem}.iii-cloud-run.yaml"));
        self.prep = Some(Prep {
            manifest,
            root,
            settings: self.settings.clone(),
            run_env,
            bindings: vec![],
            engine_port,
            http_ports,
            dbs: vec![],
            copy,
        });
        Ok(())
    }

    fn base_env() -> Vec<(String, String)> {
        ["PATH", "HOME", "TMPDIR"]
            .iter()
            .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
            .collect()
    }

    /// `sh -c cmd` with only PATH, HOME, TMPDIR and `extra`; output to `run.log`.
    async fn sh(
        &mut self,
        cmd: &str,
        cwd: &Path,
        extra: &[(String, String)],
        limit: Duration,
    ) -> Result<i32, Fail> {
        self.note(&format!("$ {cmd}"));
        let mut command = Command::new("sh");
        command
            .args(["-c", cmd])
            .current_dir(cwd)
            .env_clear()
            .envs(Self::base_env())
            .envs(extra.iter().cloned())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Its own group, so a cancel or a timeout also stops what `sh` started.
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
            .spawn()
            .map_err(|e| fail("TOOL_MISSING", format!("cannot start sh: {e}")))?;
        let mut group = GroupKill(child.id());
        // On disk while it runs: after a crash `recover` kills the group.
        self.stage_group = child.id();
        self.stage_start = child.id().and_then(proc_start);
        self.persist_run();
        let pumps = self.pump_output(&mut child, "run.log");
        let result = match tokio::time::timeout(limit, child.wait()).await {
            Ok(Ok(status)) => {
                group.0 = None;
                // Its output is in the log before its result counts; a process
                // it left running in the background does not hold the run.
                for p in pumps {
                    let _ = tokio::time::timeout(Duration::from_secs(2), p).await;
                }
                Ok(status.code().unwrap_or(-1))
            }
            Ok(Err(e)) => Err(fail("INSTALL_FAILED", e.to_string())),
            Err(_) => {
                drop(group);
                let _ = child.kill().await;
                Err(fail(
                    "RUN_TIMEOUT",
                    format!("`{}` did not end in {} s.", cut(cmd, 60), limit.as_secs()),
                ))
            }
        };
        self.stage_group = None;
        self.stage_start = None;
        self.persist_run();
        result
    }

    async fn install(&mut self) -> Result<Done, Fail> {
        if self.problems.iter().any(|c| c == "LOCAL_ENGINE_VERSION") {
            return Err(fail(
                "LOCAL_ENGINE_VERSION",
                "The run here needs the cloud's engine version; see the check's problem.",
            ));
        }
        self.prep()?;
        // The plan's runtime of each worker (what the cloud will install), else
        // `cloud.runtimes`, else the worker's own iii.worker.yaml.
        let planned: Option<Vec<(String, Option<String>)>> =
            std::fs::read_to_string(crate::deploy::plan_file(&self.ctx, &self.check_id))
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|plan| {
                    plan.get("workers").and_then(Value::as_array).map(|ws| {
                        ws.iter()
                            .filter_map(|w| {
                                Some((
                                    w.get("name")?.as_str()?.to_string(),
                                    w.get("runtime").and_then(Value::as_str).map(str::to_owned),
                                ))
                            })
                            .collect()
                    })
                });
        let (manifest_dir_workers, unknown, root) = {
            let p = self.prep.as_ref().unwrap();
            let mut runtimes: Vec<String> = vec![];
            let mut unknown: Vec<String> = vec![];
            for (name, c) in p.manifest.containers() {
                let Some(dir) = p.manifest.path_worker_dir(c) else {
                    continue;
                };
                match worker_runtime(&p.manifest, &name, &dir, planned.as_deref()) {
                    Some(r) => {
                        let base = runtime_base(&r).to_string();
                        if !runtimes.contains(&base) {
                            runtimes.push(base);
                        }
                    }
                    None => unknown.push(name),
                }
            }
            (runtimes, unknown, p.root.clone())
        };
        // A worker whose runtime is not known would be skipped without a word:
        // say so instead, since nothing would be installed for it.
        if !unknown.is_empty() {
            return Err(fail(
                "INSTALL_FAILED",
                format!(
                    "The runtime of {} is not known (not in the plan, cloud.runtimes or its iii.worker.yaml), so nothing would be installed for it.",
                    unknown.join(", ")
                ),
            ));
        }
        if manifest_dir_workers.is_empty() {
            return Ok(Done {
                status: StageStatus::Skipped,
                detail: Some("no path:// worker".into()),
            });
        }
        let mut shown = vec![];
        for runtime in manifest_dir_workers {
            let lock = lock_for(&runtime, &root).ok_or_else(|| {
                fail(
                    "INSTALL_FAILED",
                    format!("The repository has no lock for {runtime}."),
                )
            })?;
            let (cmd, extra) = install_command(lock);
            let bin = cmd.split_whitespace().next().unwrap_or("sh");
            if !on_path(bin) {
                return Err(fail(
                    "TOOL_MISSING",
                    format!("`{bin}` is not installed or not on PATH."),
                ));
            }
            let extra: Vec<(String, String)> = extra
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let limit = Duration::from_secs(self.ctx.config().await.run_timeout_s);
            let code = self.sh(cmd, &root, &extra, limit).await?;
            if code != 0 {
                return Err(fail("INSTALL_FAILED", format!("`{cmd}` exited {code}.")));
            }
            shown.push(cmd.to_string());
        }
        passed(shown.join("; "))
    }

    async fn build(&mut self) -> Result<Done, Fail> {
        let p = self.prep.as_ref().unwrap();
        let cmd = p.manifest.cloud_build().or_else(|| {
            p.settings
                .get("build_command")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        let Some(cmd) = cmd else {
            return Ok(Done {
                status: StageStatus::Skipped,
                detail: None,
            });
        };
        let root = p.root.clone();
        let limit = Duration::from_secs(self.ctx.config().await.run_timeout_s);
        // The cloud's build has no variables: only PATH, HOME, TMPDIR.
        let code = self.sh(&cmd, &root, &[], limit).await?;
        if code != 0 {
            return Err(fail("BUILD_FAILED", format!("`{cmd}` exited {code}.")));
        }
        passed(cmd)
    }

    async fn release(&mut self) -> Result<Done, Fail> {
        let (aliases, image) = {
            let p = self.prep.as_ref().unwrap();
            (
                p.manifest.database_aliases(),
                self.ctx.config().await.postgres_image.clone(),
            )
        };
        let mut started = false;
        for (container, names) in aliases {
            for alias in names {
                let url = self.start_postgres(&image).await?;
                self.secrets.push(url.clone());
                started = true;
                let p = self.prep.as_mut().unwrap();
                // The cloud's `DATABASE_<A>_DIRECT_URL` is the same URL as the pooled one:
                // the app's login (member of the owner role, `SET role`), never the owner itself.
                p.bindings.push((database_binding(&alias), url.clone()));
                p.bindings.push((
                    database_binding(&alias).replace("_URL", "_DIRECT_URL"),
                    url.clone(),
                ));
                p.dbs.push(Db {
                    container: container.clone(),
                    alias,
                    url,
                });
            }
        }
        let (release, run_env, container_env, dir) = {
            let p = self.prep.as_ref().unwrap();
            let release = p.manifest.cloud_release().or_else(|| {
                let r = p.settings.get("release")?;
                Some(crate::local::Release {
                    worker: r.get("worker")?.as_str()?.to_string(),
                    command: r.get("command")?.as_str()?.to_string(),
                    timeout: r.get("timeout").and_then(Value::as_str).map(str::to_owned),
                })
            });
            let mut env = p.run_env.clone();
            env.extend(p.bindings.iter().cloned());
            let container = release.as_ref().and_then(|r| {
                p.manifest
                    .containers()
                    .into_iter()
                    .find(|(n, _)| *n == r.worker)
            });
            let cenv: Vec<(String, String)> = container
                .as_ref()
                .and_then(|(_, c)| c.get("environment").and_then(Value::as_object))
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| {
                            (
                                k.clone(),
                                interpolate(v.as_str().unwrap_or(&v.to_string()), &env),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let dir = container
                .as_ref()
                .and_then(|(_, c)| p.manifest.path_worker_dir(c))
                .unwrap_or_else(|| p.manifest.dir.clone());
            (release, env, cenv, dir)
        };
        let Some(release) = release else {
            return Ok(Done {
                status: StageStatus::Skipped,
                detail: started.then(|| "local Postgres".into()),
            });
        };
        // The release worker's own environment, plus the bindings.
        let mut env: Vec<(String, String)> = container_env;
        env.extend(
            run_env
                .into_iter()
                .filter(|(k, _)| k.starts_with("DATABASE_")),
        );
        let limit = release
            .timeout
            .as_deref()
            .and_then(parse_duration)
            .unwrap_or(Duration::from_secs(300));
        let code = self.sh(&release.command, &dir, &env, limit).await?;
        if code != 0 {
            return Err(fail(
                "RELEASE_FAILED",
                format!("`{}` exited {code}.", release.command),
            ));
        }
        passed(cut(&release.command, 80))
    }

    async fn start_postgres(&mut self, image: &str) -> Result<String, Fail> {
        let name = format!("iii-cloud-run-pg-{}-{}", &self.run.id[..8], self.pg.len());
        let password = hex::encode(rand::random::<[u8; 12]>());
        let port = reserve_port(&self.ctx, &mut self.ports);
        let publish = format!("127.0.0.1:{port}:5432");
        let pw = format!("POSTGRES_PASSWORD={password}");
        // Known before it exists: a cancel while `docker run` is in flight still
        // has the teardown remove it (`rm -f` of a name nothing has is harmless).
        self.pg.push(name.clone());
        self.persist_run();
        let out = tool::run(
            "docker",
            &[
                "run", "-d", "--rm", "--name", &name, "-e", &pw, "-p", &publish, image,
            ],
            None,
            None,
            "DOCKER_MISSING",
        )
        .await
        .map_err(|e| fail(&e.code, e.message))?;
        if !out.ok() {
            let (code, message) = docker_failure(port, &out.stderr);
            return Err(fail(code, message));
        }
        self.secrets.push(password.clone());
        self.note(&format!("started {name} on 127.0.0.1:{port}"));
        let ready = async {
            loop {
                let probe = tool::run(
                    "docker",
                    &[
                        "exec",
                        &name,
                        "pg_isready",
                        "-h",
                        "127.0.0.1",
                        "-U",
                        "postgres",
                        "-d",
                        "postgres",
                    ],
                    None,
                    None,
                    "DOCKER_MISSING",
                )
                .await;
                if probe.is_ok_and(|o| o.ok()) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(30), ready)
            .await
            .map_err(|_| {
                fail(
                    "RELEASE_FAILED",
                    "Postgres did not accept connections in 30 s.",
                )
            })?;
        // The cloud's role model, so a migration that needs more than the app's
        // login has fails here as it would there.
        let login_password = hex::encode(rand::random::<[u8; 12]>());
        self.secrets.push(login_password.clone());
        setup_cloud_roles(&name, CLOUD_LOGIN, &login_password)
            .await
            .map_err(|m| fail("RELEASE_FAILED", m))?;
        Ok(format!(
            "postgres://{CLOUD_LOGIN}:{login_password}@127.0.0.1:{port}/app"
        ))
    }

    async fn compose_up(&mut self) -> Result<Done, Fail> {
        let ctx = self.ctx.clone();
        let mut extra = vec![];
        let rewritten = {
            let p = self.prep.as_ref().unwrap();
            rewrite_run_copy(
                &p.manifest.yaml,
                p.engine_port,
                &p.http_ports,
                &p.dbs,
                &mut || reserve_port(&ctx, &mut extra),
            )
        };
        self.ports.extend(extra);
        let rewritten = rewritten.map_err(|m| fail("COMPOSE_FAILED", m))?;
        let (copy, lock_from, dir, env, names) = {
            let p = self.prep.as_ref().unwrap();
            let text = serde_yaml_ng::to_string(&rewritten)
                .map_err(|e| fail("COMPOSE_FAILED", e.to_string()))?;
            std::fs::write(&p.copy, text).map_err(|e| fail("COMPOSE_FAILED", e.to_string()))?;
            let mut env = p.run_env.clone();
            env.extend(p.bindings.iter().cloned());
            let names: Vec<String> = p
                .manifest
                .containers()
                .into_iter()
                .map(|(n, _)| n)
                .collect();
            (
                p.copy.clone(),
                p.manifest.lock_path(),
                p.manifest.dir.clone(),
                env,
                names,
            )
        };
        if lock_from.exists() {
            let _ = std::fs::copy(&lock_from, copy.with_extension("lock"));
        }
        self.run.workers.total = names.len() as u32;
        self.config_before = Some((dir.join("config"), file_names(&dir.join("config"))));
        self.persist_run();
        std::fs::File::create(self.dir.join("compose.log"))
            .map_err(|e| fail("COMPOSE_FAILED", e.to_string()))?;
        let mut command = Command::new(&self.iii_bin);
        command
            .args(["compose", "--up", "-f"])
            .arg(&copy)
            .args(["-n", "iii-cloud-run", "--frozen"])
            .current_dir(&dir)
            .env_clear()
            .envs(Self::base_env())
            .envs(env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                fail(
                    "TOOL_MISSING",
                    format!("`{}` is not installed or not on PATH.", self.iii_bin),
                )
            } else {
                fail("COMPOSE_FAILED", e.to_string())
            }
        })?;
        // They end with the compose process.
        drop(self.pump_output(&mut child, "compose.log"));
        self.child = Some(child);
        passed(names.join(", "))
    }

    /// `<iii_bin> call --engine <run engine> <function> k=v...`, as JSON.
    async fn engine_call(&self, function: &str, args: &[&str]) -> Option<Value> {
        let port = self.prep.as_ref()?.engine_port;
        let url = format!("ws://127.0.0.1:{port}");
        let mut command = Command::new(&self.iii_bin);
        command
            .args(["call", "--engine", &url, function])
            .args(args)
            .env_clear()
            .envs(Self::base_env())
            .stdin(Stdio::null())
            .kill_on_drop(true);
        let out = tokio::time::timeout(Duration::from_secs(10), command.output())
            .await
            .ok()?
            .ok()?;
        out.status
            .success()
            .then(|| serde_json::from_slice(&out.stdout).ok())
            .flatten()
    }

    fn container_names(&self) -> Vec<String> {
        self.prep
            .as_ref()
            .map(|p| {
                p.manifest
                    .containers()
                    .into_iter()
                    .map(|(n, _)| n)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn compose_exited(&mut self) -> Option<i32> {
        self.child
            .as_mut()?
            .try_wait()
            .ok()
            .flatten()
            .map(|s| s.code().unwrap_or(-1))
    }

    async fn workers(&mut self) -> Result<Done, Fail> {
        let startup = self
            .prep
            .as_ref()
            .unwrap()
            .manifest
            .yaml
            .get("startup_timeout")
            .and_then(Value::as_str)
            .and_then(parse_duration)
            .unwrap_or(Duration::from_secs(60));
        // Under a test tick the deadline shrinks like the 60 s window does.
        let startup = if self.ctx.tick.is_some() {
            self.ctx.secs(startup.as_secs())
        } else {
            startup
        };
        let total = self.run.workers.total;
        let deadline = Instant::now() + startup;
        // The last time one more worker connected (or the stage began).
        let mut progress = Instant::now();
        let mut stable_since: Option<Instant> = None;
        let mut connected: Vec<String> = vec![];
        let mut dead_before: Vec<String> = vec![];
        loop {
            if let Some(code) = self.compose_exited() {
                // `iii compose` stops itself when a container dies before it registers.
                let gone = self.exited_in_compose_log();
                if gone.is_empty() {
                    self.append_compose_log();
                    return Err(fail(
                        "COMPOSE_FAILED",
                        format!("`iii compose` exited {code} before the workers connected."),
                    ));
                }
                return Err(self.workers_fail(
                    format!("{} exited before it connected.", gone.join(", ")),
                    &connected,
                ));
            }
            // Dead twice in a row: `iii compose` records `restarting` only after it
            // reaped the child, so one look can catch the gap before it.
            let gone = confirm_exited(&mut dead_before, self.exited_containers());
            if !gone.is_empty() {
                return Err(self.workers_fail(
                    format!("{} exited before all workers connected.", gone.join(", ")),
                    &connected,
                ));
            }
            if Instant::now() >= deadline {
                return Err(self.workers_fail(
                    format!(
                        "{} of {total} workers connected in {} s.",
                        self.run.workers.ready,
                        startup.as_secs()
                    ),
                    &connected,
                ));
            }
            // A container's worker registers under the container's name
            // (verified on iii 0.27.0: `api`, `http`), with status `connected`.
            let names = self.container_names();
            let listed = self
                .engine_call("engine::workers::list", &[])
                .await
                .and_then(|v| v.get("workers").and_then(Value::as_array).cloned());
            // A list that could not be read says nothing: poll again, and leave
            // the count, the clock and `waiting_on` as they are.
            let Some(ws) = listed else {
                tokio::time::sleep(self.ctx.secs(1)).await;
                continue;
            };
            connected = names
                .iter()
                .filter(|n| {
                    ws.iter().any(|w| {
                        w.get("name").and_then(Value::as_str) == Some(n.as_str())
                            && matches!(
                                w.get("status").and_then(Value::as_str),
                                Some("connected" | "available")
                            )
                    })
                })
                .cloned()
                .collect();
            let ready = connected.len() as u32;
            if ready > self.run.workers.ready {
                progress = Instant::now();
                if self.run.waiting_on.take().is_some() {
                    self.run.workers.ready = ready.min(total);
                    self.publish();
                }
            }
            self.run.workers.ready = ready.min(total);
            if ready >= total {
                let since = *stable_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= self.ctx.secs(2) {
                    self.run.waiting_on = None;
                    return passed(format!("{} of {total}", self.run.workers.ready));
                }
            } else {
                stable_since = None;
                // Not a failure (the cloud waits the startup timeout too): say who it waits on
                // and show their logs, so the caller can fix and check again.
                if self.run.waiting_on.is_none()
                    && progress.elapsed() >= self.ctx.secs(NO_PROGRESS_S)
                {
                    let missing = self.missing(&connected);
                    for name in &missing {
                        for line in self.container_log(name) {
                            self.note(&format!("[{name}] {line}"));
                        }
                    }
                    self.run.waiting_on = Some(WaitingOn {
                        containers: missing,
                        since: now(),
                    });
                    self.publish();
                }
            }
            tokio::time::sleep(self.ctx.secs(1)).await;
        }
    }

    /// `WORKERS_NOT_READY` naming the containers that did not connect; the run's
    /// log gets their own last lines, after `compose.log`'s.
    fn missing(&self, connected: &[String]) -> Vec<String> {
        self.container_names()
            .into_iter()
            .filter(|n| !connected.contains(n))
            .collect()
    }

    fn workers_fail(&self, lead: String, connected: &[String]) -> Fail {
        let missing = self.missing(connected);
        self.append_compose_log();
        for name in &missing {
            for line in self.container_log(name) {
                self.note(&format!("[{name}] {line}"));
            }
        }
        fail(
            "WORKERS_NOT_READY",
            format!("{lead} Did not connect: {}.", missing.join(", ")),
        )
    }

    fn state_dir(&self) -> Option<PathBuf> {
        Some(self.prep.as_ref()?.manifest.dir.join(COMPOSE_STATE))
    }

    /// The last lines a container wrote (`iii compose` keeps one file per
    /// container: a header, then `stdout<TAB>line` / `stderr<TAB>line`).
    fn container_log(&self, name: &str) -> Vec<String> {
        let Some(dir) = self.state_dir() else {
            return vec![];
        };
        let text = std::fs::read_to_string(dir.join("logs").join(format!("{name}.log")))
            .unwrap_or_default();
        let lines: Vec<&str> = text
            .lines()
            .filter(|l| !l.starts_with('#'))
            .map(|l| l.split_once('\t').map_or(l, |(_, rest)| rest))
            .collect();
        lines[lines.len().saturating_sub(CONTAINER_TAIL)..]
            .iter()
            .map(|l| l.to_string())
            .collect()
    }

    /// Containers whose process is gone, by the pids `iii compose` keeps in its
    /// state file (it lists a container once it is up).
    fn exited_containers(&self) -> Vec<String> {
        let Some(dir) = self.state_dir() else {
            return vec![];
        };
        let state = std::fs::read_to_string(dir.join("state.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok());
        state
            .as_ref()
            .and_then(|s| s.get("containers").and_then(Value::as_object))
            .into_iter()
            .flatten()
            // `restart: on-failure` keeps a dead pid in the record while it backs off.
            .filter(|(_, c)| c.get("status").and_then(Value::as_str) != Some("restarting"))
            .filter(|(_, c)| c.get("pid").and_then(Value::as_u64).is_some_and(pid_gone))
            .map(|(n, _)| n.clone())
            .collect()
    }

    /// `container 'x' exited with N before it registered`, from `compose.log`.
    fn exited_in_compose_log(&self) -> Vec<String> {
        let text = std::fs::read_to_string(self.dir.join("compose.log")).unwrap_or_default();
        text.lines()
            .filter(|l| l.contains("before it registered"))
            .filter_map(|l| {
                let rest = l.split_once("container '")?.1;
                Some(rest.split_once('\'')?.0.to_string())
            })
            .collect()
    }

    fn append_compose_log(&self) {
        for line in tail_of(&self.dir.join("compose.log")) {
            self.note(&line);
        }
    }

    async fn routes(&mut self) -> Result<Done, Fail> {
        let (has_http, prefix, strip) = {
            let p = self.prep.as_ref().unwrap();
            let (prefix, strip) = p.manifest.web_prefix();
            (!p.http_ports.is_empty(), prefix, strip)
        };
        let routes: Vec<String> = self
            .engine_call("engine::observers::list", &["function_id=http::on-request"])
            .await
            .and_then(|v| v.get("observers").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .iter()
            .filter(|o| o.get("state").and_then(Value::as_str) == Some("active"))
            .filter_map(|o| {
                let args = o.get("args")?;
                Some(format!(
                    "{} {}",
                    args.get("method")?.as_str()?,
                    args.get("path")?.as_str()?
                ))
            })
            .collect();
        self.run.routes = RouteCount {
            count: routes.len() as u32,
            prefix: prefix.clone(),
        };
        // The cloud serves routes only through `package://http`: a route with no
        // such container is served by nothing (`ROUTE_OUTSIDE_HTTP_GROUP`).
        if let (false, Some(first)) = (has_http, routes.first()) {
            return Err(fail(
                "ROUTE_OUTSIDE_HTTP_GROUP",
                format!(
                    "The route {first} is registered by a worker outside the http group: iii cloud routes traffic only to `package://http`, and this manifest has none."
                ),
            ));
        }
        if routes.is_empty() && !has_http {
            return Ok(Done {
                status: StageStatus::Skipped,
                detail: Some("no http group".into()),
            });
        }
        if routes.is_empty() {
            // The cloud does not call it a failure when a web part answers
            // (iii-cloud `control-plane/src/workers.rs:796`).
            // The plan's own `settings.web` / `settings.site`, as the cloud reads
            // them (`workers.rs:718-727`); without a saved plan, the file and the settings.
            let planned =
                std::fs::read_to_string(crate::deploy::plan_file(&self.ctx, &self.check_id))
                    .ok()
                    .and_then(|t| serde_json::from_str::<Value>(&t).ok());
            let web = match planned {
                Some(plan) => ["/settings/web", "/settings/site"]
                    .iter()
                    .any(|p| plan.pointer(p).is_some_and(|v| !v.is_null())),
                None => {
                    let p = self.prep.as_ref().unwrap();
                    p.manifest
                        .cloud()
                        .and_then(|c| c.get("web"))
                        .is_some_and(|w| !w.is_null())
                        || !p.settings.get("web").is_none_or(Value::is_null)
                }
            };
            if web {
                return passed("0; the web part answers".to_string());
            }
            return Err(fail(
                "NO_ROUTE",
                "The http group serves no route: no worker registered one.",
            ));
        }
        let outside = routes_outside_prefix(&routes, prefix.as_deref(), strip);
        if !outside.is_empty() {
            return Err(fail(
                "ROUTE_OUTSIDE_PREFIX",
                format!(
                    "Routes outside {}: {}",
                    prefix.unwrap_or_default(),
                    cut(&outside.join(", "), 300)
                ),
            ));
        }
        passed(match &prefix {
            Some(p) => format!("{} under {p}", routes.len()),
            None => routes.len().to_string(),
        })
    }

    async fn healthcheck(&mut self) -> Result<Done, Fail> {
        let (check, port) = {
            let p = self.prep.as_ref().unwrap();
            let check = p.manifest.cloud_healthcheck().or_else(|| {
                let h = p.settings.get("healthcheck")?;
                Some(crate::local::Healthcheck {
                    path: h.get("path")?.as_str()?.to_string(),
                    timeout: h.get("timeout").and_then(Value::as_str).map(str::to_owned),
                })
            });
            (check, p.http_ports.first().map(|(_, port)| *port))
        };
        let (Some(check), Some(port)) = (check, port) else {
            return Ok(Done {
                status: StageStatus::Skipped,
                detail: None,
            });
        };
        let path = healthcheck_path(&check.path);
        let limit = check
            .timeout
            .as_deref()
            .and_then(parse_duration)
            .unwrap_or(Duration::from_secs(30));
        let url = format!("http://127.0.0.1:{port}{path}");
        let deadline = Instant::now() + limit;
        let mut last = None;
        loop {
            if let Ok(r) = self
                .ctx
                .http
                .get(&url)
                .timeout(Duration::from_secs(5))
                .send()
                .await
            {
                let status = r.status().as_u16();
                last = Some(status);
                if r.status().is_success() {
                    self.run.healthcheck = Some(HealthResult {
                        path: path.clone(),
                        status: Some(status),
                    });
                    return passed(format!("GET {path} {status}"));
                }
            }
            if Instant::now() >= deadline {
                self.run.healthcheck = Some(HealthResult {
                    path: path.clone(),
                    status: last,
                });
                return Err(fail(
                    "HEALTHCHECK_FAILED",
                    format!(
                        "GET {path} did not answer 2xx in {} s (last: {}).",
                        limit.as_secs(),
                        last.map(|s| s.to_string()).unwrap_or("no answer".into())
                    ),
                ));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Always: stop compose, remove Postgres, delete the run copy, prune old runs.
    async fn teardown(&mut self) {
        if let Some(mut child) = self.child.take() {
            #[cfg(unix)]
            if let Some(pid) = child.id() {
                // SAFETY: plain signal to our own child process.
                unsafe { libc::kill(pid as i32, libc::SIGTERM) };
            }
            let pid = child.id();
            if tokio::time::timeout(Duration::from_secs(30), child.wait())
                .await
                .is_err()
            {
                // It did not stop: its whole group goes.
                drop(GroupKill(pid));
                let _ = child.kill().await;
            }
        }
        self.stage_group = None;
        self.stage_start = None;
        // A stuck docker daemon must not hold the workspace's slot for ever, nor make
        // the container forgotten: what could not be removed stays on record.
        let mut left = vec![];
        for name in std::mem::take(&mut self.pg) {
            if !remove_container(&name).await {
                left.push(name);
            }
        }
        self.pg = left;
        self.give_back_ports();
        if let Some(p) = &self.prep {
            clean_copy(&p.copy);
        }
        if let Some((dir, before)) = self.config_before.take() {
            remove_new_files(&dir, &before);
        }
        if self.pg.is_empty() {
            let _ = std::fs::remove_file(self.dir.join("run.json"));
        } else {
            // The next `recover` tries again; `Drop` tries once more too.
            self.persist_run();
        }
        // Not awaited: a hung docker daemon adds its timeout per old container,
        // and the workspace's slot must not wait for it.
        if let Some(runs) = self.dir.parent().map(Path::to_path_buf) {
            tokio::spawn(async move { prune(&runs, MAX_RUNS_KEPT).await });
        }
    }
}

/// What a crash would leave behind, for `recover`.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct RunFiles {
    containers: Vec<String>,
    copy: Option<PathBuf>,
    iii_bin: String,
    /// The process group of a stage command that was running.
    #[serde(default)]
    stage_group: Option<u32>,
    /// `proc_start` of that group's leader: `recover` kills only a process that still shows it.
    #[serde(default)]
    stage_start: Option<String>,
}

/// The names of the files in `dir` (empty when it does not exist).
pub fn file_names(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

/// Remove the YAML files that appeared in `dir` since `before` (the config the
/// run's compose wrote next to the manifest), and the directory itself when the
/// run made it and it is empty. Files that were there before stay.
pub fn remove_new_files(dir: &Path, before: &[String]) {
    for name in file_names(dir) {
        if !before.contains(&name) && name.ends_with(".yaml") {
            let _ = std::fs::remove_file(dir.join(&name));
        }
    }
    if before.is_empty() {
        let _ = std::fs::remove_dir(dir); // only when empty
    }
}

/// The app's login on the local Postgres, as the cloud makes one for a binding.
pub const CLOUD_LOGIN: &str = "app_login";

/// Run one SQL string with `psql` inside the container, as the superuser, on `db`.
async fn psql(container: &str, db: &str, sql: &str) -> Result<(), String> {
    let out = tool::run(
        "docker",
        &[
            "exec",
            container,
            "psql",
            "-h",
            "127.0.0.1",
            "-U",
            "postgres",
            "-d",
            db,
            "-v",
            "ON_ERROR_STOP=1",
            "-qAt",
            "-c",
            sql,
        ],
        None,
        None,
        "DOCKER_MISSING",
    )
    .await
    .map_err(|e| e.message)?;
    if out.ok() {
        Ok(())
    } else {
        Err(format!(
            "postgres setup failed ({sql:.40}): {}",
            cut(out.stderr.trim(), 400)
        ))
    }
}

/// Give the local Postgres iii cloud's database roles (iii-cloud
/// `crates/resources/src/postgres.rs`: `shared_database`, `login_generation`):
/// the database `app` owned by a NOLOGIN role of the same name, `REVOKE ALL ON
/// DATABASE FROM PUBLIC`, `vector` created in template1 only, and the app
/// connecting as a separate login that is a member of the owner and whose
/// sessions `SET role` to it, never as the superuser.
pub async fn setup_cloud_roles(container: &str, login: &str, password: &str) -> Result<(), String> {
    psql(container, "postgres", r#"CREATE ROLE "app" NOLOGIN"#).await?;
    // pgvector lives in template1, so the new database has it (as in the cloud).
    // An image without it fails here: the cloud's Postgres always has it.
    psql(
        container,
        "template1",
        "CREATE EXTENSION IF NOT EXISTS vector",
    )
    .await?;
    psql(
        container,
        "postgres",
        r#"CREATE DATABASE "app" OWNER "app""#,
    )
    .await?;
    psql(
        container,
        "app",
        r#"SET ROLE "app"; REVOKE ALL ON DATABASE "app" FROM PUBLIC; GRANT CONNECT ON DATABASE "app" TO SESSION_USER; REVOKE ALL ON SCHEMA public FROM PUBLIC; RESET ROLE"#,
    )
    .await?;
    psql(
        container,
        "postgres",
        &format!(r#"CREATE ROLE "{login}" LOGIN PASSWORD '{password}'"#),
    )
    .await?;
    psql(
        container,
        "postgres",
        &format!(r#"GRANT "app" TO "{login}" WITH INHERIT TRUE, SET TRUE"#),
    )
    .await?;
    psql(
        container,
        "postgres",
        &format!(r#"ALTER ROLE "{login}" IN DATABASE "app" SET role = 'app'"#),
    )
    .await
}

/// Delete the run copy, its lock and the compose state of the run's namespace.
fn clean_copy(copy: &Path) {
    let _ = std::fs::remove_file(copy);
    let _ = std::fs::remove_file(copy.with_extension("lock"));
    if let Some(dir) = copy.parent() {
        let _ = std::fs::remove_dir_all(dir.join(COMPOSE_STATE));
    }
}

/// On start: fail the runs a crash left `running`, remove their containers,
/// `iii kill --compose` a copy that remains, and delete it.
pub async fn recover_runs(ctx: &Arc<Ctx>) -> Stale {
    let cfg = ctx.config().await;
    let stuck: Vec<String> = ctx.store.read(|s| {
        s.check
            .iter()
            .chain(s.checks.iter())
            .filter(|c| {
                c.run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Running)
            })
            .map(|c| c.id.clone())
            .collect()
    });
    let mut done: Vec<String> = vec![];
    for id in stuck {
        if done.contains(&id) {
            continue;
        }
        done.push(id.clone());
        let updated = update_check(ctx, &id, |c| {
            if let Some(run) = c.run.as_mut().filter(|r| r.status == RunStatus::Running) {
                let stage = run
                    .stages
                    .iter()
                    .find(|s| s.status == StageStatus::Running)
                    .map(|s| s.name)
                    .unwrap_or(StageName::Install);
                run.status = RunStatus::Failed;
                run.waiting_on = None;
                run.finished_at = Some(now());
                run.error = Some(RunError {
                    stage,
                    code: "WORKER_RESTARTED".into(),
                    message: "The worker restarted while the run was going.".into(),
                });
                for s in run
                    .stages
                    .iter_mut()
                    .filter(|s| s.status == StageStatus::Running)
                {
                    s.status = StageStatus::Failed;
                }
            }
        });
        // Its leftovers were never recorded: the worktrees of that environment may hold some.
        if let Some(env) = updated.as_ref().map(|c| c.environment_id.clone()) {
            let paths: Vec<String> = ctx.store.read(|s| {
                s.handoffs
                    .iter()
                    .filter(|h| h.environment_id == env)
                    .map(|h| h.workspace_path.clone())
                    .chain(
                        s.project
                            .as_ref()
                            .filter(|p| p.environment.as_ref().is_some_and(|e| e.id == env))
                            .and_then(|p| p.workspace.as_ref().map(|w| w.path.clone())),
                    )
                    .filter(|p| !p.is_empty())
                    .collect()
            });
            for p in paths {
                mark_unrecorded(ctx, Path::new(&p));
            }
        }
        // Whoever waits for this run's end hears it.
        if let Some(run) = updated.and_then(|c| c.run) {
            emit_run(ctx, &run);
        }
    }
    let mut stale = vec![];
    if let Ok(read) = std::fs::read_dir(cfg.data_path().join("runs")) {
        for entry in read.flatten() {
            let file = entry.path().join("run.json");
            let Some(files) = std::fs::read(&file)
                .ok()
                .and_then(|b| serde_json::from_slice::<RunFiles>(&b).ok())
            else {
                continue;
            };
            // What is keyed by the workspace (a group, `iii compose`'s namespace and its
            // copy) goes now, before any run of this process can use the same ones.
            // A stage command the crash left running (its own group, so the worker's
            // death did not reach it): only when that pid is still the very process
            // that was recorded, since a pid is reused after a reboot.
            if let (Some(pid), Some(start)) = (files.stage_group, files.stage_start.as_deref())
                && proc_start(pid).as_deref() == Some(start)
            {
                drop(GroupKill(Some(pid)));
            }
            if let Some(copy) = files.copy.filter(|c| c.exists()) {
                let copy_arg = copy.to_string_lossy().into_owned();
                let args = ["kill", "--compose", copy_arg.as_str()];
                let kill = tool::run(&files.iii_bin, &args, None, None, "TOOL_MISSING");
                let _ = tokio::time::timeout(cleanup_timeout(), kill).await;
                clean_copy(&copy);
            }
            // What is left is the containers, named by run id: they can wait.
            let rest = RunFiles {
                containers: files.containers,
                iii_bin: files.iii_bin,
                ..Default::default()
            };
            if rest.containers.is_empty() {
                let _ = std::fs::remove_file(&file);
            } else {
                write_run_files(&file, &rest);
                stale.push((file, rest.containers));
            }
        }
    }
    Stale(stale)
}

fn write_run_files(file: &Path, files: &RunFiles) {
    if let Ok(bytes) = serde_json::to_vec(files) {
        let _ = std::fs::write(file, bytes);
    }
}

/// The containers earlier runs left, with the `run.json` that names them.
pub struct Stale(Vec<(PathBuf, Vec<String>)>);

/// `recover_runs` and `clean_stale` one after the other.
pub async fn recover(ctx: &Arc<Ctx>) {
    let stale = recover_runs(ctx).await;
    clean_stale(stale).await;
}

/// Remove the containers earlier runs left (their names carry the run id, so this
/// cannot touch a run of this process). Each removal has a timeout; one that could
/// not be done stays on record for the next start.
pub async fn clean_stale(stale: Stale) {
    for (file, containers) in stale.0 {
        let left = remove_containers(&containers).await;
        if left.is_empty() {
            let _ = std::fs::remove_file(file);
        } else {
            let iii_bin = std::fs::read(&file)
                .ok()
                .and_then(|b| serde_json::from_slice::<RunFiles>(&b).ok())
                .map(|f| f.iii_bin)
                .unwrap_or_default();
            write_run_files(
                &file,
                &RunFiles {
                    containers: left,
                    iii_bin,
                    ..Default::default()
                },
            );
        }
    }
}

/// Try to remove each container; the names that are still there.
async fn remove_containers(names: &[String]) -> Vec<String> {
    let mut left = vec![];
    for name in names {
        if !remove_container(name).await {
            left.push(name.clone());
        }
    }
    left
}

/// Hide the password of every `scheme://user:password@host` in `text`.
pub fn redact_urls(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (head, after) = rest.split_at(at + 3);
        out.push_str(head);
        let end = after
            .find(|c: char| c.is_whitespace() || c == '/' || c == '"' || c == '\'')
            .unwrap_or(after.len());
        let authority = &after[..end];
        match (authority.rfind('@'), authority.find(':')) {
            (Some(a), Some(c)) if c < a => {
                out.push_str(&authority[..=c]);
                out.push_str("[redacted]");
                out.push_str(&authority[a..]);
            }
            _ => out.push_str(authority),
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// Copy `out` into `file` (appended) a line at a time, each line redacted.
// ponytail: a value split across a 64 KiB line cut is not caught; values are short.
async fn pump(out: impl tokio::io::AsyncRead + Unpin, file: PathBuf, secrets: Arc<Vec<String>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let Ok(mut log) = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
        .await
    else {
        return;
    };
    let mut out = out;
    let (mut buf, mut line) = (vec![0u8; 8192], Vec::<u8>::new());
    loop {
        let n = out.read(&mut buf).await.unwrap_or(0);
        line.extend_from_slice(&buf[..n]);
        // Whole lines out; at the end (or past 64 KiB) the rest too.
        let cut_at = match line.iter().rposition(|b| *b == b'\n') {
            Some(i) => i + 1,
            None if n == 0 || line.len() > 64 << 10 => line.len(),
            None => continue,
        };
        let text = redact(&String::from_utf8_lossy(&line[..cut_at]), &secrets);
        if log.write_all(text.as_bytes()).await.is_err() {
            return;
        }
        line.drain(..cut_at);
        if n == 0 {
            return;
        }
    }
}

/// Replace every secret value (3 characters or more) with `[redacted]`.
pub fn redact(text: &str, secrets: &[String]) -> String {
    secrets
        .iter()
        // Every non-empty value, so `s3cret` goes too; only 1-2 characters
        // would turn the log into noise.
        .filter(|s| s.len() >= 3)
        .fold(text.to_string(), |acc, s| {
            acc.replace(s.as_str(), "[redacted]")
        })
}

fn tail_of(file: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    // Not cut here: a line is redacted whole first, then cut.
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    lines[lines.len().saturating_sub(TAIL)..].to_vec()
}

/// The folders of the runs that are going now: `prune` never touches them
/// (a live run's `run.json` already names its Postgres).
static LIVE_RUNS: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

pub fn set_run_live(dir: &Path, live: bool) {
    let mut runs = LIVE_RUNS.lock().unwrap_or_else(|e| e.into_inner());
    runs.retain(|d| d != dir);
    if live {
        runs.push(dir.to_path_buf());
    }
}

/// Keep the newest `keep` run folders.
pub async fn prune(runs: &Path, keep: usize) {
    let Ok(read) = std::fs::read_dir(runs) else {
        return;
    };
    let mut dirs: Vec<_> = read
        .flatten()
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    dirs.sort();
    let extra = dirs.len().saturating_sub(keep);
    for (_, path) in dirs.into_iter().take(extra) {
        if LIVE_RUNS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&path)
        {
            continue;
        }
        // A folder whose record still names containers is the only trace of them:
        // it goes once they are removed, and stays while one is not.
        let file = path.join("run.json");
        if let Some(files) = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice::<RunFiles>(&b).ok())
            .filter(|f| !f.containers.is_empty())
        {
            let left = remove_containers(&files.containers).await;
            if !left.is_empty() {
                write_run_files(
                    &file,
                    &RunFiles {
                        containers: left,
                        ..files
                    },
                );
                continue;
            }
        }
        let _ = std::fs::remove_dir_all(path);
    }
}

#[cfg(test)]
mod tests {
    use super::{recheck, save_left};

    #[test]
    fn a_file_changed_between_its_save_and_the_checkout_is_saved_again() {
        let ws = tempfile::tempdir().unwrap();
        let left = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("a.yaml"), "as the run left it\n").unwrap();
        std::fs::write(ws.path().join("b.yaml"), "unchanged\n").unwrap();
        let kept: Vec<(String, String)> = ["a.yaml", "b.yaml"]
            .iter()
            .map(|p| {
                (
                    p.to_string(),
                    save_left(ws.path(), left.path(), p).unwrap().0,
                )
            })
            .collect();
        // The agent edits one of them after it was saved.
        std::fs::write(ws.path().join("a.yaml"), "the agent's edit\n").unwrap();
        let back = recheck(ws.path(), left.path(), kept);
        assert_eq!(back, vec!["a.yaml", "b.yaml"]);
        // What the checkout will replace is what `left/` holds.
        assert_eq!(
            std::fs::read_to_string(left.path().join("a.yaml")).unwrap(),
            "the agent's edit\n"
        );
    }
}
