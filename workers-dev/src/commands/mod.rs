use std::io::{self, Write};
use std::sync::Arc;

use anyhow::{bail, Result};

use crate::logs;
use crate::orchestrator::Orchestrator;
use crate::status;

pub async fn run_status(orchestrator: &Orchestrator) -> Result<()> {
    let default_stack = &orchestrator.config.default_stack;
    let members = orchestrator.stack_members(default_stack)?;
    let (views, engine_error) = orchestrator.dashboard_snapshot(&members).await;
    if let Some(err) = engine_error {
        eprintln!("warning: engine unreachable: {err}");
    }
    status::print_status_table(&views, default_stack);
    Ok(())
}

pub async fn run_start(orchestrator: &Orchestrator, workers: Vec<String>, all: bool) -> Result<()> {
    if workers.is_empty() {
        if all {
            orchestrator.start_all_managed(true).await?;
            println!("started all managed workers");
        } else {
            let stack = orchestrator.config.default_stack.clone();
            orchestrator.start_stack(&stack, true).await?;
            println!("started stack {stack}");
        }
    } else {
        orchestrator.start_workers(&workers, true).await?;
        for worker in &workers {
            println!("started {worker}");
        }
    }
    Ok(())
}

pub async fn run_stop(orchestrator: &Orchestrator, workers: Vec<String>) -> Result<()> {
    orchestrator.stop_workers(&workers).await?;
    Ok(())
}

pub async fn run_restart(orchestrator: &Orchestrator, worker: &str) -> Result<()> {
    println!("restarting {worker} and dependents…");
    orchestrator.restart_worker(worker).await?;
    println!("done");
    Ok(())
}

pub async fn run_logs(
    orchestrator: Arc<Orchestrator>,
    worker: String,
    follow: bool,
    lines: usize,
) -> Result<()> {
    let tail = orchestrator.logs_tail(&worker, lines).await?;
    let color_enabled = orchestrator.config.color_mode.enabled_for_stdout();
    for line in tail {
        logs::print_colored_line(&line, color_enabled, &mut io::stdout())?;
    }

    if !follow {
        return Ok(());
    }
    io::stdout().flush()?;

    let mut rx = orchestrator.subscribe_logs(&worker).await?;
    loop {
        match rx.recv().await {
            Ok(line) => {
                logs::print_colored_line(&line, color_enabled, &mut io::stdout())?;
                io::stdout().flush()?;
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                let proc = orchestrator.worker_views().await?;
                match proc.iter().find(|v| v.name == worker) {
                    Some(v) if v.process_status == "running" || v.process_status == "compiling" => {
                    }
                    _ => break,
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                rx = orchestrator.subscribe_logs(&worker).await?;
            }
        }
    }
    Ok(())
}

/// Roots of `stack`, refusing an empty set. Same message `start_roots` bails
/// with for the same condition — this is strictly an earlier checkpoint on
/// the same rule, so `run_up` can refuse before `ensure_engine()` rather than
/// spawning the engine (an external side effect) for a stack that cannot
/// start anything.
fn startable_stack_roots(orchestrator: &Orchestrator, stack: &str) -> Result<Vec<String>> {
    let roots = orchestrator.stack_roots(stack)?;
    if roots.is_empty() {
        bail!("stack {stack} has no startable workers");
    }
    Ok(roots)
}

pub async fn run_up(orchestrator: Arc<Orchestrator>) -> Result<()> {
    let stack = orchestrator.config.default_stack.clone();
    startable_stack_roots(&orchestrator, &stack)?;
    orchestrator.ensure_engine().await?;
    orchestrator.start_stack(&stack, false).await?;
    crate::tui::run(orchestrator).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// Minimal `Orchestrator` with no discovered workers — `WorkerGraph::load`
    /// never touches the filesystem when `workers` is empty, so this needs no
    /// repo fixture, and (more importantly) no reachable engine: the guard
    /// under test must fire without either.
    fn orchestrator_with_stacks(
        default_stack: &str,
        stacks: Vec<(String, Vec<String>)>,
    ) -> Orchestrator {
        let config = Config {
            repo_root: std::path::PathBuf::new(),
            config_path: std::path::PathBuf::new(),
            engine_url: crate::config::DEFAULT_ENGINE_URL.to_string(),
            release: false,
            poll_interval_ms: crate::config::DEFAULT_POLL_INTERVAL_MS,
            connect_timeout_ms: crate::config::DEFAULT_CONNECT_TIMEOUT_MS,
            workers: Vec::new(),
            stacks,
            default_stack: default_stack.to_string(),
            worker_specs: Vec::new(),
            stop_on_exit: false,
            color_mode: Default::default(),
            ui_watch: false,
        };
        Orchestrator::new(config, false).unwrap()
    }

    /// The regression `run_up` must never reopen: an empty default stack has
    /// to be rejected here, synchronously and with no engine contact, so the
    /// caller (`run_up`) never reaches `ensure_engine()` for a stack that
    /// cannot start anything. `start_roots` carries the same guard for the
    /// actual start; this pins the earlier checkpoint independently since
    /// `run_up` itself always ends by handing off to the TUI and so can't be
    /// exercised end-to-end in a unit test.
    #[test]
    fn startable_stack_roots_rejects_an_empty_default_stack() {
        let orch = orchestrator_with_stacks("ghost", vec![("ghost".to_string(), Vec::new())]);
        let err = startable_stack_roots(&orch, "ghost").unwrap_err();
        assert!(err.to_string().contains("ghost"), "{err:#}");
    }

    #[test]
    fn startable_stack_roots_allows_a_nonempty_stack() {
        let orch = orchestrator_with_stacks(
            "harness",
            vec![("harness".to_string(), vec!["harness".to_string()])],
        );
        assert_eq!(
            startable_stack_roots(&orch, "harness").unwrap(),
            vec!["harness".to_string()]
        );
    }
}
