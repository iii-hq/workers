use std::io::{self, Write};
use std::sync::Arc;

use anyhow::Result;

use crate::logs;
use crate::orchestrator::Orchestrator;
use crate::status;

pub async fn run_status(orchestrator: &Orchestrator) -> Result<()> {
    let views = orchestrator.worker_views().await?;
    status::print_status_table(&views);
    Ok(())
}

pub async fn run_start(orchestrator: &Orchestrator, workers: Vec<String>, all: bool) -> Result<()> {
    if workers.is_empty() {
        if all {
            orchestrator.start_all_managed(true).await?;
            println!("started all managed workers");
        } else {
            orchestrator.start_harness_stack(true).await?;
            println!("started harness stack");
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
    if !follow {
        let tail = orchestrator.logs_tail(&worker, lines).await?;
        let color_enabled = orchestrator.config.color_mode.enabled_for_stdout();
        for line in tail {
            logs::print_colored_line(&line, color_enabled, &mut io::stdout())?;
        }
        return Ok(());
    }

    let tail = orchestrator.logs_tail(&worker, lines).await?;
    let color_enabled = orchestrator.config.color_mode.enabled_for_stdout();
    for line in tail {
        logs::print_colored_line(&line, color_enabled, &mut io::stdout())?;
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
                if proc.iter().any(|v| v.name == worker && v.process_status == "stopped") {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                rx = orchestrator.subscribe_logs(&worker).await?;
            }
        }
    }
    Ok(())
}

pub async fn run_up(orchestrator: Arc<Orchestrator>) -> Result<()> {
    orchestrator.start_harness_stack(false).await?;
    crate::tui::run(orchestrator).await
}
