mod color;
mod commands;
mod config;
mod discover;
mod git;
mod graph;
mod logs;
mod orchestrator;
mod runtime;
mod status;
mod tui;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::orchestrator::Orchestrator;

#[derive(Parser, Debug)]
#[command(
    name = "workers-dev",
    about = "Local dev orchestrator for iii workers in this repo",
    version
)]
struct Cli {
    /// Workers repo root (auto-detected when omitted)
    #[arg(long, global = true)]
    repo: Option<PathBuf>,

    /// Engine WebSocket URL
    #[arg(long, global = true)]
    url: Option<String>,

    /// Engine WebSocket port (overrides port in --url)
    #[arg(long, global = true)]
    port: Option<u16>,

    /// Run workers with cargo run --release
    #[arg(long, global = true)]
    release: bool,

    /// Optional workers-dev.yaml config file
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Stop all managed workers when the process exits
    #[arg(long, global = true)]
    stop_on_exit: bool,

    /// Color output: auto, always, or never (respects NO_COLOR)
    #[arg(long, global = true, default_value = "auto")]
    color: String,

    /// Start injectable-UI workers in watcher mode: run `pnpm watch` in
    /// <worker>/ui and set III_<WORKER>_UI_WATCH=1 so open console tabs
    /// hot-reload the worker's UI on rebuild (toggle per worker with `w`)
    #[arg(long, global = true)]
    ui_watch: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Start the harness stack and open the TUI dashboard
    Up,
    /// Start workers (default: harness stack; use --all for every worker)
    Start {
        /// Worker name(s) to start; default = the harness stack
        #[arg(value_name = "WORKER")]
        workers: Vec<String>,
        /// Start every discovered worker instead of the harness stack
        #[arg(long)]
        all: bool,
    },
    /// Stop workers (default: all managed)
    Stop {
        /// Worker name(s) to stop; default = all managed workers
        #[arg(value_name = "WORKER")]
        workers: Vec<String>,
    },
    /// Rebuild and restart a worker, then restart its dependents
    Restart {
        /// Worker to rebuild and restart (its dependents restart too)
        worker: String,
    },
    /// Print worker logs from the local ring buffer
    Logs {
        /// Worker whose logs to print
        worker: String,
        /// Follow log output
        #[arg(short = 'f', long)]
        follow: bool,
        /// Number of lines to show
        #[arg(short = 'n', long, default_value_t = crate::config::DEFAULT_LOG_TAIL)]
        lines: usize,
    },
    /// Print one-shot status table (no TUI)
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load(
        cli.repo,
        cli.url,
        cli.port,
        cli.release,
        cli.config,
        cli.stop_on_exit,
        Some(cli.color),
        cli.ui_watch,
    )?;

    let progress = matches!(
        &cli.command,
        Some(Command::Start { .. }) | Some(Command::Restart { .. })
    );
    let orchestrator = Arc::new(Orchestrator::new(config, progress)?);

    let result = match cli.command {
        None => crate::tui::run(orchestrator.clone()).await,
        Some(Command::Up) => commands::run_up(orchestrator.clone()).await,
        Some(Command::Start { workers, all }) => {
            commands::run_start(&orchestrator, workers, all).await
        }
        Some(Command::Stop { workers }) => commands::run_stop(&orchestrator, workers).await,
        Some(Command::Restart { worker }) => commands::run_restart(&orchestrator, &worker).await,
        Some(Command::Logs {
            worker,
            follow,
            lines,
        }) => commands::run_logs(orchestrator.clone(), worker, follow, lines).await,
        Some(Command::Status) => commands::run_status(&orchestrator).await,
    };

    if orchestrator.config.stop_on_exit {
        if let Err(err) = orchestrator.stop_workers(&[]).await {
            eprintln!("warning: {err:#}");
        }
    }

    orchestrator.shutdown().await;

    result
}
