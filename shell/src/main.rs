use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, RegisterFunction};
use serde_json::Value;
use std::sync::Arc;

mod arity;
mod config;
mod exec;
mod exec_dispatch;
mod fs;
mod functions;
mod jobs;
mod manifest;
mod target;
mod triggers;

use functions::types::{ClassifyArgvRequest, KillRequest, StatusRequest};

#[derive(Parser, Debug)]
#[command(name = "shell", about = "Unix shell execution worker for iii agents")]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,

    #[arg(long)]
    manifest: bool,
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

    if cli.manifest {
        let m = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m).unwrap());
        return Ok(());
    }

    let shell_config = match config::load_config(&cli.config) {
        Ok(c) => {
            tracing::info!(
                allowlist_size = c.allowlist.len(),
                denylist_size = c.denylist_patterns.len(),
                max_timeout_ms = c.max_timeout_ms,
                max_concurrent = c.max_concurrent_jobs,
                "loaded config from {}",
                cli.config
            );
            c
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            let mut c = config::ShellConfig::default();
            c.compile_denylist()?;
            // Defaults have host_root=None and allow_unjailed=false, so this
            // path refuses to start. Otherwise a missing config file would
            // silently bypass the S-H2 jail requirement.
            c.validate_fs_jail()?;
            c
        }
    };
    let shared = Arc::new(shell_config);

    tracing::info!(url = %cli.url, "connecting to III engine");
    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );

    // shell::exec, shell::exec_bg, and the 10 shell::fs::* handlers take
    // Value at the registration boundary so they can preserve legacy wire
    // contracts (S210 for fs malformed payloads, "missing 'command'" /
    // "must be a string" for exec, silent timeout_ms fallback) that the
    // SDK's typed deserialization can't reproduce.

    {
        let cfg = shared.clone();
        let iii_for_exec = iii.clone();
        iii.register_function(
            RegisterFunction::new_async(
                "shell::exec",
                move |req: functions::types::ExecRequest| {
                    let cfg = cfg.clone();
                    let iii_clone = iii_for_exec.clone();
                    async move { functions::exec::handle(cfg, iii_clone, req).await }
                },
            )
            .description(
                "Run an allowlisted command in the foreground and return its \
                 full output. Payload: { command: string (program name), \
                 args?: string[], timeout_ms?: number, target?: { kind: \
                 'host'|'sandbox', sandbox_id?: string } }. Returns { stdout, \
                 stderr, exit_code, duration_ms, timed_out, stdout_truncated, \
                 stderr_truncated }. Do NOT pass argv as an array in 'command' \
                 — split program and arguments across the two fields.",
            ),
        );
    }

    {
        let cfg = shared.clone();
        let iii_for_bg = iii.clone();
        iii.register_function(
            RegisterFunction::new_async(
                "shell::exec_bg",
                move |req: functions::types::ExecBgRequest| {
                    let cfg = cfg.clone();
                    let iii_clone = iii_for_bg.clone();
                    async move { functions::exec_bg::handle(cfg, iii_clone, req).await }
                },
            )
            .description(
                "Spawn an allowlisted command as a background job. Same \
                 payload shape as shell::exec; returns { job_id, argv } \
                 immediately. Poll with shell::status, terminate with \
                 shell::kill, list with shell::list. Do NOT pass argv as an \
                 array in 'command' — use 'command' (string) + 'args' \
                 (string[]).",
            ),
        );
    }

    {
        let cfg = shared.clone();
        iii.register_function(
            RegisterFunction::new_async(
                "shell::classify_argv",
                move |req: ClassifyArgvRequest| {
                    let cfg = cfg.clone();
                    async move { functions::classify::handle(cfg, req).await }
                },
            )
            .description(
                "Classify a shell argv for the approval gate (agent path). Returns \
                 { decision: 'auto' | 'deny' | 'ask', ... } per shell policy — \
                 not for direct agent use.",
            ),
        );
    }

    iii.register_function(
        RegisterFunction::new_async("shell::kill", |req: KillRequest| async move {
            functions::kill::handle(req).await
        })
        .description("Kill a running background job"),
    );

    iii.register_function(
        RegisterFunction::new_async("shell::status", |req: StatusRequest| async move {
            functions::status::handle(req).await
        })
        .description("Get status of a background job"),
    );

    {
        let cfg = shared.clone();
        iii.register_function(
            RegisterFunction::new_async("shell::list", move |_req: Value| {
                let cfg = cfg.clone();
                async move { functions::list::handle(cfg).await }
            })
            .description("List all background jobs"),
        );
    }

    let host_fs_cfg = std::sync::Arc::new(crate::fs::host::HostFsConfig {
        host_root: shared.fs.host_root.clone(),
        max_read_bytes: shared.fs.max_read_bytes,
        max_write_bytes: shared.fs.max_write_bytes,
        denylist_paths: shared.fs.denylist_paths.clone(),
    });
    let chan_maker: std::sync::Arc<dyn crate::fs::host::ChannelMaker> =
        std::sync::Arc::new(crate::fs::host::IiiChannelMaker::new(iii.clone()));
    let host_backend: std::sync::Arc<dyn crate::fs::FsBackend> =
        std::sync::Arc::new(crate::fs::host::HostFsBackend::new(host_fs_cfg, chan_maker));
    let sb_enabled = shared.sandbox.enabled;

    if shared.fs.host_root.is_none() {
        tracing::warn!(
            "fs.host_root is unset — host backend is unjailed; \
             every absolute path is reachable by shell::fs::* (denylist still applies)",
        );
    }

    {
        let h = host_backend.clone();
        let i = iii.clone();
        iii.register_function(
            RegisterFunction::new_async("shell::fs::ls", move |req: Value| {
                let h = h.clone();
                let i = i.clone();
                async move { functions::fs_ls::handle(h, i, sb_enabled, req).await }
            })
            .description("List directory contents on host or sandbox"),
        );
    }

    {
        let h = host_backend.clone();
        let i = iii.clone();
        iii.register_function(
            RegisterFunction::new_async("shell::fs::stat", move |req: Value| {
                let h = h.clone();
                let i = i.clone();
                async move { functions::fs_stat::handle(h, i, sb_enabled, req).await }
            })
            .description("Stat a path on host or sandbox"),
        );
    }

    {
        let h = host_backend.clone();
        let i = iii.clone();
        iii.register_function(
            RegisterFunction::new_async("shell::fs::mkdir", move |req: Value| {
                let h = h.clone();
                let i = i.clone();
                async move { functions::fs_mkdir::handle(h, i, sb_enabled, req).await }
            })
            .description("Create a directory on host or sandbox"),
        );
    }

    {
        let h = host_backend.clone();
        let i = iii.clone();
        iii.register_function(
            RegisterFunction::new_async("shell::fs::rm", move |req: Value| {
                let h = h.clone();
                let i = i.clone();
                async move { functions::fs_rm::handle(h, i, sb_enabled, req).await }
            })
            .description("Remove a path on host or sandbox"),
        );
    }

    {
        let h = host_backend.clone();
        let i = iii.clone();
        iii.register_function(
            RegisterFunction::new_async("shell::fs::chmod", move |req: Value| {
                let h = h.clone();
                let i = i.clone();
                async move { functions::fs_chmod::handle(h, i, sb_enabled, req).await }
            })
            .description("Change permissions on host or sandbox"),
        );
    }

    {
        let h = host_backend.clone();
        let i = iii.clone();
        iii.register_function(
            RegisterFunction::new_async("shell::fs::mv", move |req: Value| {
                let h = h.clone();
                let i = i.clone();
                async move { functions::fs_mv::handle(h, i, sb_enabled, req).await }
            })
            .description("Move/rename a path on host or sandbox"),
        );
    }

    {
        let h = host_backend.clone();
        let i = iii.clone();
        iii.register_function(
            RegisterFunction::new_async("shell::fs::grep", move |req: Value| {
                let h = h.clone();
                let i = i.clone();
                async move { functions::fs_grep::handle(h, i, sb_enabled, req).await }
            })
            .description("Recursive regex search on host or sandbox"),
        );
    }

    {
        let h = host_backend.clone();
        let i = iii.clone();
        iii.register_function(
            RegisterFunction::new_async("shell::fs::sed", move |req: Value| {
                let h = h.clone();
                let i = i.clone();
                async move { functions::fs_sed::handle(h, i, sb_enabled, req).await }
            })
            .description("Find-and-replace on host or sandbox"),
        );
    }

    {
        let h = host_backend.clone();
        let i = iii.clone();
        iii.register_function(
            RegisterFunction::new_async("shell::fs::write", move |req: Value| {
                let h = h.clone();
                let i = i.clone();
                async move { functions::fs_write::handle(h, i, sb_enabled, req).await }
            })
            .description("Stream a file to a host path or sandbox via StreamChannelRef"),
        );
    }

    {
        let h = host_backend.clone();
        let i = iii.clone();
        iii.register_function(
            RegisterFunction::new_async("shell::fs::read", move |req: Value| {
                let h = h.clone();
                let i = i.clone();
                async move { functions::fs_read::handle(h, i, sb_enabled, req).await }
            })
            .description("Stream a file from a host path or sandbox via StreamChannelRef"),
        );
    }

    tracing::info!("shell registered 16 functions, ready");

    tokio::signal::ctrl_c().await?;
    tracing::info!("shell shutting down");
    iii.shutdown_async().await;
    Ok(())
}
