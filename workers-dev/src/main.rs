//! `workers-dev` — a dashboard and remote control for the repo's `iii compose`
//! project. It starts a compose daemon when none is serving, and everything
//! after that is a `compose::*` call.

mod color;
mod compose;
mod config;
mod git;
mod logs;
mod tui;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use crate::compose::Compose;
use crate::config::Config;

#[derive(Parser, Debug)]
#[command(
    name = "workers-dev",
    about = "Dashboard and remote control for the repo's iii compose project",
    version
)]
struct Cli {
    /// Workers repo root (auto-detected from harness/worker-compose.yaml)
    #[arg(long, global = true)]
    repo: Option<PathBuf>,

    /// Offer the workers in this directory too, alongside the repo's own. The
    /// directory is a worker when it carries an `iii.worker.yaml`, otherwise
    /// its children are. Repeatable; `WORKERS_DEV_WORKER_DIRS` is the
    /// colon-separated equivalent, for a shell profile.
    #[arg(long = "worker-dir", value_name = "DIR", global = true)]
    worker_dirs: Vec<PathBuf>,

    /// Compose namespace. Defaults to the compose file's own `namespace:`;
    /// a second worktree needs its own, plus III_ENGINE_PORT.
    #[arg(short = 'n', long, global = true)]
    namespace: Option<String>,

    /// Color output: auto, always, or never (respects NO_COLOR)
    #[arg(long, global = true, value_parser = ["auto", "always", "never"])]
    color: Option<String>,

    /// Start every watchable container's injectable-UI watcher
    #[arg(long, global = true)]
    ui_watch: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Start the project and open the dashboard (same as no subcommand)
    Up,
    /// Start the whole project, or the named containers and their dependencies
    Start {
        #[arg(value_name = "CONTAINER")]
        containers: Vec<String>,
    },
    /// Stop the named containers (and their dependents), or the whole daemon
    Stop {
        #[arg(value_name = "CONTAINER")]
        containers: Vec<String>,
    },
    /// Restart exactly one container, leaving its graph alone
    Restart {
        #[arg(value_name = "CONTAINER")]
        container: String,
    },
    /// Print one compose::status table
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load(
        cli.repo,
        cli.worker_dirs,
        cli.namespace,
        cli.color,
        cli.ui_watch,
    )?;
    let color = config.color_mode.enabled_for_stdout();
    let compose = Arc::new(Compose::new(config)?);

    watch_for_termination(Arc::clone(&compose));

    let result = run(&cli.command, &compose, color).await;
    compose.shutdown().await;
    result
}

/// A closed terminal sends SIGHUP, which kills the process outright — no
/// unwinding, so `kill_on_drop` never fires and the `pnpm watch` sidecars keep
/// rebuilding forever. Stop them, then leave: the compose daemon is not ours
/// to kill, and that is the point of quitting with the stack still up.
fn watch_for_termination(compose: Arc<Compose>) {
    #[cfg(unix)]
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let (Ok(mut term), Ok(mut hup)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::hangup()),
        ) else {
            return;
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = hup.recv() => {}
        }
        compose.stop_ui_children().await;
        std::process::exit(130);
    });
    #[cfg(not(unix))]
    let _ = compose;
}

async fn run(command: &Option<Command>, compose: &Arc<Compose>, color: bool) -> Result<()> {
    match command {
        None | Some(Command::Up) => {
            compose.ensure_daemon().await?;
            tui::run(Arc::clone(compose)).await
        }
        Some(Command::Start { containers }) if containers.is_empty() => {
            // `ensure_daemon` spawns with `--up`, which already brings the
            // project up — and during that window compose has not registered
            // its mutations yet, so a second up would be both redundant and
            // unservable.
            compose.ensure_daemon().await
        }
        Some(Command::Start { containers }) => {
            compose.ensure_daemon().await?;
            for container in containers {
                match compose.project_of(container).await {
                    // An on-demand worker is re-declared every time: the repo
                    // may have moved under it, and the daemon only re-reads a
                    // project it is made to.
                    Some(file) if file.starts_with(&compose.config.local_dir) => {
                        let bin = on_demand_bin(compose, container)?;
                        compose.add_local(container, &bin).await?
                    }
                    Some(file) => compose.up(&file, Some(container)).await?,
                    // Not declared anywhere yet: a repo worker started on
                    // demand, which declares it in the local project first.
                    None => match compose.repo_worker(container) {
                        Some(_) => {
                            let bin = on_demand_bin(compose, container)?;
                            compose.add_local(container, &bin).await?
                        }
                        None => bail!("no container or repo worker named {container:?}"),
                    },
                }
            }
            Ok(())
        }
        Some(Command::Stop { containers }) if containers.is_empty() => compose.stop().await,
        Some(Command::Stop { containers }) => {
            compose.ensure_daemon().await?;
            for container in containers {
                let Some(file) = compose.project_of(container).await else {
                    bail!("{container:?} is not declared by any running project");
                };
                compose.down(&file, container).await?;
            }
            Ok(())
        }
        Some(Command::Restart { container }) => {
            compose.ensure_daemon().await?;
            let Some(file) = compose.project_of(container).await else {
                bail!("{container:?} is not declared by any running project");
            };
            compose.restart(&file, container).await
        }
        Some(Command::Status) => {
            compose.ensure_daemon().await?;
            print_status(compose, color).await
        }
    }
}

/// What `cargo run --bin` would name, or why this worker is not ours to start.
fn on_demand_bin(compose: &Compose, container: &str) -> Result<String> {
    compose
        .repo_worker(container)
        .and_then(|worker| worker.bin.clone())
        .with_context(|| {
            format!(
                "{container} is not a Rust binary — install it from the registry with \
                 `iii trigger compose::add worker={container}`"
            )
        })
}

async fn print_status(compose: &Compose, color: bool) -> Result<()> {
    let status = compose.status().await?;
    println!(
        "namespace {}  ·  daemon {}  ·  {}",
        status.namespace,
        status.daemon_pid,
        status.file.display()
    );
    println!("{:<24} {:<9} {:<7} LAST ERROR", "CONTAINER", "STATE", "PID");
    print_containers(&status.containers, color);

    // Anything started on demand is its own project; leaving those out would
    // report a stack that is not what is actually running.
    let (_, projects) = compose.projects().await.unwrap_or_default();
    for project in projects
        .iter()
        .filter(|project| project.file != status.file && !project.containers.is_empty())
    {
        println!("\non demand  ·  {}", project.file.display());
        print_containers(&project.containers, color);
    }
    Ok(())
}

fn print_containers(containers: &[compose::ContainerStatus], color: bool) {
    for container in containers {
        let pid = container
            .pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "—".into());
        let state = if color {
            paint(&container.state)
        } else {
            container.state.clone()
        };
        println!(
            "{:<24} {state:<9} {pid:<7} {}",
            container.container,
            container.last_error.as_deref().unwrap_or("")
        );
    }
}

fn paint(state: &str) -> String {
    let code = match state {
        "ready" => "32",
        "failed" => "31",
        _ => "90",
    };
    format!("\x1b[{code}m{state}\x1b[0m")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every one of these described a supervisor workers-dev no longer is.
    /// A clap error names the flag; a silently ignored flag would not.
    #[test]
    fn supervisor_era_flags_are_rejected() {
        for flag in ["--release", "--stop-on-exit", "--config", "--url", "--port"] {
            assert!(
                Cli::try_parse_from(["workers-dev", flag, "x"]).is_err(),
                "{flag} should no longer parse"
            );
        }
        // `logs` moved to `iii compose logs`, which has cursors and archives.
        assert!(Cli::try_parse_from(["workers-dev", "logs", "harness"]).is_err());
    }

    #[test]
    fn start_with_no_names_means_the_whole_project() {
        let cli = Cli::try_parse_from(["workers-dev", "start"]).unwrap();
        assert!(
            matches!(cli.command, Some(Command::Start { containers }) if containers.is_empty())
        );
        let cli = Cli::try_parse_from(["workers-dev", "-n", "wt2", "restart", "harness"]).unwrap();
        assert_eq!(cli.namespace.as_deref(), Some("wt2"));
    }
}
