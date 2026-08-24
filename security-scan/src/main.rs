use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use iii_helpers::observability::OtelConfig;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use security_scan::{
    configuration, functions, manifest, IiiRuntime, RunStatusV1, SecurityScanExecutor,
    SecurityScanService,
};

#[derive(Debug, Parser)]
#[command(
    name = "security-scan",
    about = "Durable, report-only security reviews over exact Git commits"
)]
struct Cli {
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    #[arg(long)]
    manifest: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest::build_manifest())
                .expect("manifest must serialize")
        );
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "security-scan".into(),
                os: std::env::consts::OS.into(),
                description: Some(manifest::DESCRIPTION.into()),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            otel: Some(OtelConfig::default()),
            ..InitOptions::default()
        },
    ));

    let config = configuration::register_and_fetch(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading security-scan configuration")?;
    let runtime = Arc::new(IiiRuntime::new(iii.clone()));
    let executor = Arc::new(SecurityScanExecutor::new(runtime.clone(), config.clone()));
    let deps = Arc::new(functions::Deps {
        service: Arc::new(SecurityScanService::new(runtime.clone(), config.clone())),
        executor: executor.clone(),
    });
    functions::register_all(&iii, &deps);
    security_scan::ui::register(&iii);

    let _completion_trigger = match iii.register_trigger(RegisterTriggerInput {
        trigger_type: "harness::turn-completed".into(),
        function_id: functions::TURN_COMPLETED_ID.into(),
        config: serde_json::json!({}),
        metadata: None,
        namespace: None,
        trigger_namespace: None,
    }) {
        Ok(trigger) => Some(trigger),
        Err(error) => {
            tracing::warn!(%error, "Harness completion doorbell binding failed; polling remains active");
            None
        }
    };
    let schedule_handles =
        security_scan::schedule::register(&iii, deps.service.clone(), Arc::new(config.clone()))
            .await;
    let initial_schedule_count = schedule_handles.bound_schedule_count();

    runtime
        .claim_private_state()
        .await
        .map_err(anyhow::Error::msg)
        .context("claiming private security-scan state")?;
    match runtime.backfill_run_index().await {
        Ok(0) => {}
        Ok(count) => tracing::info!(count, "backfilled security scan run history"),
        Err(error) => {
            tracing::warn!(%error, "security scan run history backfill deferred")
        }
    }
    runtime
        .ensure_queue()
        .await
        .map_err(anyhow::Error::msg)
        .context("defining security-scan FIFO queue")?;

    reconcile_runs(&runtime, &executor).await;

    // Harness completion events are an optimization, not the source of
    // truth. Periodic State/Queue/Harness reconciliation covers sibling boot
    // order, lost asynchronous trigger registration, and lost queue wakes.
    let recovery_runtime = runtime.clone();
    let recovery_executor = executor.clone();
    let mut recovery_schedule_handles = schedule_handles;
    let recovery = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            recovery_schedule_handles.recover_bindings().await;
            reconcile_runs(&recovery_runtime, &recovery_executor).await;
        }
    });

    tracing::info!(
        repositories = deps.service.configured_repository_count(),
        schedules = initial_schedule_count,
        "security-scan ready"
    );
    tokio::signal::ctrl_c().await?;
    recovery.abort();
    let _ = recovery.await;
    iii.shutdown_async().await;
    Ok(())
}

async fn reconcile_runs(
    runtime: &Arc<IiiRuntime>,
    executor: &Arc<SecurityScanExecutor<IiiRuntime>>,
) {
    match runtime.retry_run_index_backfill().await {
        Ok(None | Some(0)) => {}
        Ok(Some(count)) => tracing::info!(count, "backfilled security scan run history"),
        Err(error) => tracing::warn!(%error, "security scan run history backfill deferred"),
    }
    let repaired = runtime.repair_pending_run_index().await;
    if repaired > 0 {
        tracing::info!(repaired, "repaired security scan run history projections");
    }
    match runtime.list_reconciliation_runs().await {
        Ok(runs) => {
            for run in runs {
                if run.status == RunStatusV1::Analyzing {
                    if let Err(error) = executor.reconcile_analysis(&run).await {
                        tracing::warn!(run_id = %run.run_id, %error, "analysis recovery failed");
                    }
                } else if run.materialized.is_some()
                    && matches!(
                        run.status,
                        RunStatusV1::Completed | RunStatusV1::Failed | RunStatusV1::Cancelled
                    )
                {
                    if let Err(error) = executor.cleanup_terminal(&run).await {
                        tracing::warn!(run_id = %run.run_id, %error, "checkout cleanup recovery failed");
                    }
                }
            }
        }
        Err(error) => tracing::warn!(%error, "could not list analyses for recovery"),
    }
    match runtime.recover_queueable_runs().await {
        Ok(0) => {}
        Ok(count) => tracing::info!(count, "re-enqueued recoverable security scan runs"),
        Err(error) => tracing::warn!(%error, "security scan queue recovery failed"),
    }
}
