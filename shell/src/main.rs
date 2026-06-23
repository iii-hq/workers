use anyhow::{Context, Result};
use clap::Parser;
use iii_observability::OtelConfig;
use iii_sdk::{register_worker, IIIError, InitOptions, RegisterFunction};
use serde_json::Value;

mod config;
mod configuration;
mod exec;
mod exec_dispatch;
mod fs;
mod functions;
mod jobs;
mod scode;
mod target;
mod telemetry;
mod triggers;

use configuration::AppState;
use functions::types::{KillRequest, StatusRequest};

#[derive(Parser, Debug)]
#[command(name = "shell", about = "Unix shell execution worker for iii agents")]
struct Cli {
    /// Seed config registered as `initial_value` with the `configuration` worker
    /// on first registration. Defaults to ./config.yaml. The live value from the
    /// configuration worker takes precedence once an entry exists.
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
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
    tracing::info!(url = %cli.url, seed_config = %cli.config, "connecting to III engine");

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
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

    // exec / exec_bg / list read the live config snapshot from AppState.
    {
        let st = state.clone();
        iii.register_function(
            "shell::exec",
            RegisterFunction::new_async(move |req: functions::types::ExecRequest| {
                let st = st.clone();
                telemetry::record_call("shell::exec", async move {
                    let cfg = { st.runtime.read().await.config.clone() };
                    // handle already returns Result<_, IIIError> (S-codes lifted
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
                 per-call values — but only for keys already in allowed_env and never for \
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
                        .map_err(IIIError::from)
                })
            })
            .description(
                "Spawn an allowlisted command as a background job; returns { job_id, argv } \
                 immediately. Same payload as shell::exec (command + args, do NOT pass argv as an \
                 array), including the optional host-only `cwd` (jail-confined; escape is S215), \
                 `env` (only allowed_env keys, never PATH/IFS/HOME/LD_*/DYLD_* or other loader/lookup \
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
                functions::kill::handle(req).await.map_err(IIIError::from)
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
                functions::status::handle(req).await.map_err(IIIError::from)
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
                    functions::list::handle(cfg).await.map_err(IIIError::from)
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
                    Ok::<Value, IIIError>(serde_json::to_value(status)?)
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
fn register_fs(iii: &iii_sdk::III, state: &AppState) {
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
                        // handle already returns Result<_, IIIError> (FsError
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
    use super::Cli;
    use clap::Parser;

    #[test]
    fn config_defaults_to_local_config_yaml() {
        let cli = Cli::parse_from(["shell"]);
        assert_eq!(cli.config, "./config.yaml");
    }
}
