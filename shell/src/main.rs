use anyhow::{Context, Result};
use clap::Parser;
use iii_helpers::observability::OtelConfig;
use iii_sdk::errors::Error;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions, RegisterFunction};
use serde_json::Value;

mod code;
mod config;
mod configuration;
mod exec;
mod exec_dispatch;
mod fs;
mod functions;
mod jobs;
mod path;
mod scode;
mod target;
mod telemetry;
mod triggers;

use configuration::AppState;
use functions::types::{KillRequest, StatusRequest};

#[derive(Parser, Debug)]
#[command(
    name = "shell",
    version,
    about = "Unix shell execution worker for iii agents"
)]
struct Cli {
    /// Seed config registered as `initial_value` with the `configuration` worker
    /// on first registration. Defaults to ./config.yaml. The live value from the
    /// configuration worker takes precedence once an entry exists.
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    /// WebSocket URL of the iii engine. Also read from the III_URL env var.
    /// The worker retries the connection forever (2s backoff); when the engine
    /// is unreachable at boot, a single loud error from the pre-connect probe
    /// says so.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
}

/// Host/port of a ws(s):// engine URL, for the pre-connect probe. `None` when
/// the URL does not parse or has no host; `port_or_known_default` maps
/// ws→80 / wss→443 when no explicit port is given.
fn ws_host_port(url_str: &str) -> Option<(String, u16)> {
    let u = url::Url::parse(url_str).ok()?;
    // host_str keeps IPv6 brackets ("[::1]"), which ToSocketAddrs rejects.
    let host = u.host_str()?.trim_matches(['[', ']']).to_string();
    let port = u.port_or_known_default()?;
    Some((host, port))
}

/// One loud, actionable ERROR when the engine is unreachable, BEFORE handing
/// off to the SDK's silent infinite 2s-backoff reconnect loop (which only
/// WARNs). Never fails fast — supervised deployments rely on the SDK retry —
/// and never blocks boot for more than ~4s (2s connect timeout, at most two
/// resolved addresses tried). Parse/resolve failures just skip the probe: the
/// SDK is the authority on what URLs it accepts.
fn probe_engine_reachable(url_str: &str) {
    use std::net::{TcpStream, ToSocketAddrs};
    let Some((host, port)) = ws_host_port(url_str) else {
        tracing::warn!(url = %url_str, "could not parse engine URL; skipping reachability probe");
        return;
    };
    let addrs = match (host.as_str(), port).to_socket_addrs() {
        Ok(a) => a.collect::<Vec<_>>(),
        Err(e) => {
            tracing::warn!(url = %url_str, error = %e, "engine host did not resolve");
            return;
        }
    };
    let reachable = addrs
        .iter()
        .take(2)
        .any(|a| TcpStream::connect_timeout(a, std::time::Duration::from_secs(2)).is_ok());
    if !reachable {
        tracing::error!(
            url = %url_str,
            "engine unreachable at {url_str} — is the iii engine running? Set --url or the \
             III_URL env var if it listens elsewhere. Continuing to retry in the background \
             every 2s."
        );
    }
}

/// Identify this worker to the engine as `shell` (name, runtime, version, pid)
/// so it appears as `shell` in `engine::workers::list` and the `worker`
/// lifecycle stream — not the default `Host:<pid>` identity. Console surfaces
/// gate the working-directory picker on this name, mirroring `approval-gate`.
fn worker_metadata() -> WorkerMetadata {
    WorkerMetadata {
        runtime: "rust".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: "shell".to_string(),
        os: std::env::consts::OS.to_string(),
        pid: Some(std::process::id()),
        telemetry: None,
        ..WorkerMetadata::default()
    }
}

/// JSON Schema for a typed request/response struct, attached to a `Value`
/// handler via `request_format`/`response_format` so the engine publishes the
/// full contract while the handler keeps its legacy `S210` deserialization.
fn schema_value<T: schemars::JsonSchema>() -> Value {
    let root = schemars::gen::SchemaGenerator::default().into_root_schema_for::<T>();
    serde_json::to_value(root).expect("schema serializes")
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    tracing::info!(url = %cli.url, seed_config = %cli.config, "connecting to IIIClient engine");
    probe_engine_reachable(&cli.url);

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(worker_metadata()),
            ..Default::default()
        },
    );

    // Build the per-call metric instruments once (and register the
    // shell.jobs.running observable gauge) now that the OTel meter provider is
    // installed. Idempotent and a silent no-op when no collector is attached.
    telemetry::init();

    let seed = match config::ShellConfig::from_file(&cli.config) {
        Ok(cfg) => {
            tracing::info!(path = %cli.config, "loaded seed config for initial registration");
            Some(cfg)
        }
        Err(e) => {
            tracing::warn!(path = %cli.config, error = %e, "could not load --config seed; using the stored configuration value if present, else the built-in zero-config default");
            None
        }
    };

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering shell configuration schema")?;

    // One-shot, best-effort fold of a legacy `coder` config entry into the
    // `shell` value (never-widen; idempotent). Runs after schema registration
    // and before the fetch below so the merged value is what we boot from.
    configuration::migrate_legacy_coder(&iii).await;

    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading shell configuration")?;

    let runtime = configuration::build_runtime(&cfg, &iii)
        .map_err(anyhow::Error::msg)
        .context("building initial shell runtime")?;

    // ONE advisory reminder at boot (not per-reload): an operator might mistake
    // the exec denylist for a hard boundary. It is advisory regex over
    // argv.join(" "); the real security boundary is the sandbox backend.
    if !runtime.config.denylist_patterns.is_empty() {
        tracing::warn!(
            target: "sandbox",
            count = runtime.config.denylist_patterns.len(),
            "exec denylist is ADVISORY: regex matched against argv.join(\" \"), trivially \
             evadable — the security boundary is the sandbox backend, not this denylist"
        );
    }

    let state = AppState {
        runtime: std::sync::Arc::new(tokio::sync::RwLock::new(runtime)),
        iii: iii.clone(),
        reload_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        reload_status: std::sync::Arc::new(tokio::sync::RwLock::new(
            configuration::ReloadStatus::default(),
        )),
    };

    // Register the config-change trigger and reconcile BEFORE exposing public
    // functions. The trigger registration + this reconcile sit ahead of every
    // shell/fs function, so a failure here aborts startup before anything is
    // exposed (fail-closed). The reconcile closes the boot race: an update that
    // landed between the initial fetch (above) and trigger registration has no
    // listener and no later event to repair it, so we MUST confirm the
    // authoritative value before serving. fetch_config already retries
    // internally; the initial fetch already proved the engine reachable, so a
    // failure here means the engine just went away — refuse to serve a possibly
    // stale security policy and exit so the supervisor restarts us.
    configuration::register_config_trigger(&iii, state.clone())
        .context("registering configuration change trigger")?;
    configuration::reconcile(&state)
        .await
        .map_err(anyhow::Error::msg)
        .context(
            "boot reconcile of configuration failed (refusing to serve a possibly stale policy)",
        )?;

    // Code surface (folded coder::*): build the path-jail resolver from the
    // UNIFIED roots (fs.host_roots) + the `code` block's protected globs, then
    // register the 9 code functions. The resolver IS the jail. A bad glob /
    // unreachable root makes PathResolver::new fail and aborts startup — the
    // whole worker fails closed (no half-booted surface). The code surface
    // requires a jail: unjailed shells don't expose coder::*.
    if cfg.fs.is_jailed() {
        let code_cfg = cfg.code_resolver_config();
        let resolver = code::path::PathResolver::new(&code_cfg)
            .map_err(|e| anyhow::anyhow!("failed to build code PathResolver (coder::*): {e}"))?;
        let cell: code::ConfigCell =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::sync::Arc::new(code_cfg)));
        code::register_all(&iii, std::sync::Arc::new(resolver), cell);
        tracing::info!("code surface (coder::*) registered over the unified fs jail");
    } else {
        tracing::warn!(
            "fs is unjailed (no fs.host_root/fs.host_roots) — code surface (coder::*) NOT \
             registered; coder file functions require a jail root"
        );
    }

    // exec / exec_bg / list read the live config snapshot from AppState.
    {
        let st = state.clone();
        iii.register_function(
            "shell::exec",
            RegisterFunction::new_async(move |req: functions::types::ExecRequest| {
                let st = st.clone();
                telemetry::record_call("shell::exec", async move {
                    let cfg = { st.runtime.read().await.config.clone() };
                    // handle already returns Result<_, Error> (S-codes lifted
                    // to Remote inside); no map_err needed.
                    let res = functions::exec::handle(cfg, st.iii.clone(), req).await;
                    // Truncation is only visible on the typed Ok response (the
                    // generic record_call wrapper sees an opaque T), so emit the
                    // truncation counter here without altering the result.
                    if let Ok(ref out) = res {
                        telemetry::record_output_truncated(
                            "shell::exec",
                            out.stdout_truncated,
                            out.stderr_truncated,
                        );
                    }
                    res
                })
            })
            .description(
                "Run an allowlisted command in the foreground and return its full output. \
                 Put the program name in `command` (string) and arguments in `args` (string[]) — \
                 do NOT pass argv as an array in `command`. `timeout_ms` is capped at the \
                 configured max; negative/fractional values fall back to the default. `target` \
                 defaults to the host; pass { kind: \"sandbox\", sandbox_id } to run in a microVM. \
                 Optional host-only `cwd` scopes this call to a directory (jail-confined exactly \
                 like shell::fs::* paths; escaping it is S215), optional `env` (object) sets \
                 per-call values — but only for keys already in the operator's env.allow list and never for \
                 PATH/IFS/HOME/LD_*/DYLD_* or other loader/lookup and interpreter-startup keys \
                 (those reject S210) — and optional host-only `stdin` (string) is written to the \
                 program's standard input (use it for `tee`, `patch`, or any stdin filter instead \
                 of a shell heredoc). cwd/env/stdin on a sandbox target reject S210. \
                 Backend errors return { code, message }; common: S216 host exec error, S300 VM \
                 boot failed, S200 in-VM failure. argv-parse and allowlist/denylist rejections are \
                 plain-string messages naming the violation.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "shell::exec_bg",
            RegisterFunction::new_async(move |req: functions::types::ExecBgRequest| {
                let st = st.clone();
                telemetry::record_call("shell::exec_bg", async move {
                    let cfg = { st.runtime.read().await.config.clone() };
                    functions::exec_bg::handle(cfg, st.iii.clone(), req)
                        .await
                        .map_err(Error::from)
                })
            })
            .description(
                "Spawn an allowlisted command as a background job; returns { job_id, argv } \
                 immediately. Same payload as shell::exec (command + args, do NOT pass argv as an \
                 array), including the optional host-only `cwd` (jail-confined; escape is S215), \
                 `env` (only keys in the operator's env.allow list, never PATH/IFS/HOME/LD_*/DYLD_* or other loader/lookup \
                 and interpreter-startup keys), and `stdin` (string written to the job's stdin); \
                 violations and cwd/env/stdin on a sandbox target reject with an S210 message. Poll \
                 with shell::status, terminate with shell::kill, list with shell::list. \
                 Host background jobs ignore the per-call timeout_ms and run until they exit or \
                 shell::kill terminates them; set the operator config `max_bg_timeout_ms` (0 = \
                 unbounded, the default) to force-kill a runaway job after that long. \
                 Spawn-time failures (argv-parse, allowlist/denylist, cwd/env gating, spawn, \
                 concurrency cap) are plain-string messages naming the violation; once spawned, \
                 per-job failures surface through shell::status (the job record's status/stderr), \
                 not this call's return.",
            ),
        );
    }

    iii.register_function(
        "shell::kill",
        RegisterFunction::new_async(|req: KillRequest| {
            telemetry::record_call("shell::kill", async move {
                functions::kill::handle(req).await.map_err(Error::from)
            })
        })
        .description(
            "Terminate a running background job by job_id (the UUID from shell::exec_bg). \
             Errors return { code, message }; common: S211 no such job, S216 kill/signal delivery \
             failure.",
        ),
    );

    iii.register_function(
        "shell::status",
        RegisterFunction::new_async(|req: StatusRequest| {
            telemetry::record_call("shell::status", async move {
                functions::status::handle(req).await.map_err(Error::from)
            })
        })
        .description(
            "Fetch the full record (status, exit_code, timing) of a background job by job_id. \
             Errors return { code, message }; common: S211 no such job.",
        ),
    );

    {
        let st = state.clone();
        iii.register_function(
            "shell::list",
            // Ignore the payload: shell::list takes no args, and the engine
            // injects a `_caller_worker_id` field into every call — a typed
            // request param would reject that injected field. (ListRequest is
            // schema-only; see its doc comment.)
            RegisterFunction::new_async(move |_req: Value| {
                let st = st.clone();
                telemetry::record_call("shell::list", async move {
                    let cfg = { st.runtime.read().await.config.clone() };
                    functions::list::handle(cfg).await.map_err(Error::from)
                })
            })
            .request_format(schema_value::<functions::types::ListRequest>())
            .response_format(schema_value::<functions::types::ListResponse>())
            .description(
                "List background jobs (running + recently completed). Takes no arguments.",
            ),
        );
    }

    {
        let st = state.clone();
        iii.register_function(
            "shell::config-status",
            // Ignore the payload (see shell::list): no-arg call, and the
            // engine-injected `_caller_worker_id` would break a typed param.
            RegisterFunction::new_async(move |_req: Value| {
                let st = st.clone();
                telemetry::record_call("shell::config-status", async move {
                    let status = { st.reload_status.read().await.clone() };
                    Ok::<Value, Error>(serde_json::to_value(status)?)
                })
            })
            .request_format(schema_value::<functions::types::ConfigStatusRequest>())
            .response_format(schema_value::<configuration::ReloadStatus>())
            .description(
                "Report the last configuration hot-reload outcome: last_outcome \
                 (applied|rejected), last_error, and rejected_reloads (count since \
                 boot). A rejected outcome or non-zero count means a stored config \
                 was refused and shell is enforcing an older policy than the central \
                 store. Takes no arguments.",
            ),
        );
    }

    register_workspace(&iii, &state);

    // fs::* keep Value handlers (preserving S210) and read the live host backend
    // + sandbox toggle from AppState; the typed schema is attached separately.
    register_fs(&iii, &state);

    // Background reaper: time-based eviction of finished JobRecords. Without
    // it, an agent that uses exec_bg + status-polling (and never calls
    // shell::list) leaks every finished record — each holding up to
    // max_output_bytes of stdout + stderr — for the worker's lifetime. The
    // prune-on-list path remains as a harmless secondary trigger. The reaper
    // reads retention from the LIVE config snapshot so a hot-reload of
    // job_retention_secs is honored. It is detached and does not block
    // shutdown: the process exits on signal regardless of where this loop is.
    spawn_job_reaper(state.clone());

    tracing::info!("shell registered all functions, ready");
    wait_for_shutdown_signal().await?;
    tracing::info!("shell shutting down");

    // Deterministic cleanup: terminate in-flight host jobs so they do not
    // outlive the worker as orphans. A running host bg job's Child is owned by
    // its drain task (not the handle), so the sweep does NOT signal a pid
    // directly (that risked killing a reused pid); it notifies each job's
    // kill-signal channel and the drain task kills the child's process group,
    // then the sweep polls up to ~3s for the jobs to finalize so shutdown is
    // deterministic. kill_on_drop(true) on the spawned commands is the backstop
    // if a drain task does not finish within that window.
    let killed = jobs::kill_running_host_jobs().await;
    if killed > 0 {
        tracing::info!(count = killed, "killed in-flight host jobs on shutdown");
    }

    iii.shutdown_async().await;
    Ok(())
}

/// Register operator control-plane functions used by the console working-dir
/// picker. These are intentionally separate from shell::fs::* and coder::*:
/// they browse existing host directories for UI selection, while the harness
/// injects the chosen path as trusted per-turn metadata.
fn register_workspace(iii: &iii_sdk::IIIClient, state: &AppState) {
    {
        let st = state.clone();
        iii.register_function(
            "shell::workspace::roots",
            RegisterFunction::new_async(
                move |_req: functions::workspace::WorkspaceRootsRequest| {
                    let st = st.clone();
                    telemetry::record_call("shell::workspace::roots", async move {
                        let cfg = { st.runtime.read().await.config.clone() };
                        Ok::<_, Error>(functions::workspace::workspace_roots(&cfg))
                    })
                },
            )
            .description(
                "Console-only workspace picker control plane: return canonical host directory \
                 anchors that an operator can browse before choosing a per-session working \
                 directory.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "shell::workspace::validate",
            RegisterFunction::new_async(
                move |req: functions::workspace::WorkspaceValidateRequest| {
                    let st = st.clone();
                    telemetry::record_call("shell::workspace::validate", async move {
                        let cfg = { st.runtime.read().await.config.clone() };
                        functions::workspace::validate_workspace_path(req, &cfg)
                            .map_err(Error::from)
                    })
                },
            )
            .description(
                "Console-only workspace picker control plane: validate that `path` is an \
                 existing host directory and return its canonical path.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "shell::workspace::list",
            RegisterFunction::new_async(move |req: functions::workspace::WorkspaceListRequest| {
                let st = st.clone();
                telemetry::record_call("shell::workspace::list", async move {
                    let cfg = { st.runtime.read().await.config.clone() };
                    functions::workspace::list_workspace_dirs(req, &cfg).map_err(Error::from)
                })
            })
            .description(
                "Console-only workspace picker control plane: list child directories under an \
                 existing host directory. Returns canonical paths and never returns files.",
            ),
        );
    }
}

/// Detached task that prunes finished `JobRecord`s on a fixed cadence using the
/// live retention from the config snapshot. Interval is `min(30s, retention/2)`
/// so a short retention still gets timely eviction, but we never poll more
/// often than every second.
///
/// Intentionally fire-and-forget: the returned `JoinHandle` is dropped, so the
/// task has no shutdown hook and is torn down when the tokio runtime stops on
/// signal. That is acceptable because the reaper only evicts already-finished
/// records (pure cleanup) — losing it mid-tick on shutdown drops nothing the
/// process needs, and the explicit kill sweep in `main()` handles live jobs.
fn spawn_job_reaper(state: AppState) {
    const MAX_INTERVAL_SECS: u64 = 30;
    tokio::spawn(async move {
        loop {
            let retention = { state.runtime.read().await.config.job_retention_secs };
            // retention/2 keeps eviction timely for short windows; clamp to
            // [1s, 30s] so we neither hot-spin nor lag far behind retention.
            let tick_secs = (retention / 2).clamp(1, MAX_INTERVAL_SECS);
            tokio::time::sleep(std::time::Duration::from_secs(tick_secs)).await;
            jobs::remove_old(retention).await;
        }
    });
}

/// Register the 10 shell::fs::* functions. Each keeps a `Value` handler (so the
/// inline S210 mapping survives), reads the live host backend + sandbox toggle
/// from AppState, and publishes its typed schema via request/response_format.
fn register_fs(iii: &iii_sdk::IIIClient, state: &AppState) {
    macro_rules! fs_fn {
        ($id:literal, $module:ident, $req:ty, $resp:ty, $desc:expr) => {{
            let st = state.clone();
            iii.register_function(
                $id,
                RegisterFunction::new_async(move |req: Value| {
                    let st = st.clone();
                    // telemetry::record_call times the call, classifies the
                    // Result into outcome/code, and emits shell.calls +
                    // shell.call.duration_ms — returning the Result untouched.
                    telemetry::record_call($id, async move {
                        let (host, sb_enabled) = {
                            let rt = st.runtime.read().await;
                            (rt.host_backend.clone(), rt.config.sandbox.enabled)
                        };
                        // handle already returns Result<_, Error> (FsError
                        // S-codes lifted to Remote inside); no map_err needed.
                        functions::$module::handle(host, st.iii.clone(), sb_enabled, req).await
                    })
                })
                .request_format(schema_value::<$req>())
                .response_format(schema_value::<$resp>())
                .description($desc),
            );
        }};
    }

    fs_fn!("shell::fs::ls", fs_ls, fs::LsRequest, fs::LsResponse,
        "List directory contents. `path` is relative to the configured fs jail root (fs.host_root) \
         when set, otherwise absolute. `target` defaults to host; pass { kind: \"sandbox\", sandbox_id } \
         to run in a microVM. Errors return { code, message }; common: S210 bad path, S211 not found, \
         S212 not a directory, S215 jail/denylist.");
    fs_fn!("shell::fs::stat", fs_stat, fs::StatRequest, fs::StatResponse,
        "Stat a single path (jail-relative when fs.host_root is set). Returns the entry's type, size, \
         mode, and mtime. Errors return { code, message }; common: S211 not found, S215 jail/denylist.");
    fs_fn!("shell::fs::mkdir", fs_mkdir, fs::MkdirRequest, fs::MkdirResponse,
        "Create a directory. `mode` is an octal string like \"0755\". `parents: true` creates missing \
         parents and is idempotent. Returns { created, path, already_existed }. Errors return \
         { code, message }; common: S210 bad mode, S213 exists, S215 jail/denylist.");
    fs_fn!(
        "shell::fs::rm",
        fs_rm,
        fs::RmRequest,
        fs::RmResponse,
        "Remove a path. `recursive: true` is required to delete a non-empty directory. Returns \
         { removed, path, was_present }. Errors return { code, message }; common: S211 not found, \
         S214 dir not empty (pass recursive), S215 jail/denylist."
    );
    fs_fn!("shell::fs::chmod", fs_chmod, fs::ChmodRequest, fs::ChmodResponse,
        "Change permissions. `mode` is an octal string like \"0644\". `uid`/`gid` optionally chown. \
         `recursive: true` walks the tree (symlinks skipped). Returns { entries_changed, path, recursive }. \
         Errors return { code, message }; common: S210 bad mode, S211 not found, S215 jail/denylist.");
    fs_fn!(
        "shell::fs::mv",
        fs_mv,
        fs::MvRequest,
        fs::MvResponse,
        "Move/rename a path. `overwrite: true` allows replacing an existing dst. Returns \
         { moved, src, dst, overwrote }. Errors return { code, message }; common: S211 src not found, \
         S213 dst exists, S215 jail/denylist."
    );
    fs_fn!(
        "shell::fs::grep",
        fs_grep,
        fs::GrepRequest,
        fs::GrepResponse,
        "Search file contents. `pattern` is a Rust regex (RE2-like). `recursive` defaults true. \
         `include_glob`/`exclude_glob` filter paths. Returns { matches, truncated }. Errors return \
         { code, message }; common: S217 bad regex, S215 jail/denylist."
    );
    fs_fn!("shell::fs::sed", fs_sed, fs::SedRequest, fs::SedResponse,
        "Find-and-replace across files. `pattern` is a Rust regex by default (set regex:false for a \
         literal). Provide either `files` (explicit list) or `path` (+ recursive). Returns \
         { results, total_replacements }. Errors return { code, message }; common: S217 bad regex, \
         S211 not found, S215 jail/denylist.");
    fs_fn!(
        "shell::fs::write",
        fs_write,
        fs::WriteRequest,
        fs::WriteResponse,
        "Write a file. Simplest form: { path, content: \"text\" } — `content` as a plain STRING is \
         written inline (host target only), no streaming channel needed. For large/streamed payloads \
         or a sandbox target, pass `content` as a ContentRef { channel_id, access_key, direction } \
         from a write stream channel you opened through the engine's streaming layer (inline strings \
         reject S210 on a sandbox target). To write several files at once, pass `files: [{ path, \
         content, mode?, parents? }, ...]` instead of the single-file fields (host target, inline \
         content) — the response then carries per-file results in `files`. `mode` is octal \
         (default \"0644\"); `parents: true` creates missing parents. Errors return { code, message }; \
         common: S210 bad mode/payload or inline-on-sandbox, S215 jail/denylist, S218 payload exceeds \
         max_write_bytes, S216 channel/IO error."
    );
    fs_fn!("shell::fs::read", fs_read, fs::ReadRequest, fs::ReadResponseWire,
        "Stream a file from a path. Returns a ContentRef the caller reads from, plus size/mode/mtime. \
         Errors return { code, message }; common: S211 not found, S212 path is a directory, S215 \
         jail/denylist, S218 file exceeds max_read_bytes, S216 channel/IO error.");
}

/// Wait for SIGINT or, on Unix, SIGTERM so `docker stop` / `kubectl delete`
/// (SIGTERM) still trigger a clean `iii.shutdown_async()`.
async fn wait_for_shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate())?;
        tokio::select! {
            r = tokio::signal::ctrl_c() => r,
            _ = sigterm.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

#[cfg(test)]
mod tests {
    use super::{ws_host_port, Cli};
    use clap::Parser;

    #[test]
    fn config_defaults_to_local_config_yaml() {
        let cli = Cli::parse_from(["shell"]);
        assert_eq!(cli.config, "./config.yaml");
    }

    /// `--version` must exist and report the crate version — operators use it
    /// to check what a deployed binary actually is.
    #[test]
    fn version_flag_reports_crate_version() {
        let err = Cli::try_parse_from(["shell", "--version"])
            .expect_err("--version short-circuits parsing");
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
        assert!(
            err.to_string().contains(env!("CARGO_PKG_VERSION")),
            "renders the crate version: {err}"
        );
    }

    /// The long help must surface the III_URL env var and the default engine
    /// URL — this is the only self-documenting place for the binary's env vars.
    #[test]
    fn help_documents_url_env_and_default() {
        use clap::CommandFactory;
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("III_URL"), "help names III_URL: {help}");
        assert!(
            help.contains("ws://127.0.0.1:49134"),
            "help shows the default URL: {help}"
        );
        assert!(
            help.contains("iii engine"),
            "help describes what the URL points at: {help}"
        );
    }

    #[test]
    fn ws_host_port_parses_explicit_port() {
        assert_eq!(
            ws_host_port("ws://127.0.0.1:49134"),
            Some(("127.0.0.1".to_string(), 49134))
        );
    }

    #[test]
    fn ws_host_port_parses_ipv6_without_brackets() {
        assert_eq!(
            ws_host_port("ws://[::1]:1234"),
            Some(("::1".to_string(), 1234))
        );
    }

    #[test]
    fn ws_host_port_uses_known_default_ports() {
        assert_eq!(ws_host_port("ws://localhost"), Some(("localhost".to_string(), 80)));
        assert_eq!(
            ws_host_port("wss://engine.example"),
            Some(("engine.example".to_string(), 443))
        );
    }

    #[test]
    fn ws_host_port_rejects_garbage() {
        assert_eq!(ws_host_port("not a url"), None);
    }
}
