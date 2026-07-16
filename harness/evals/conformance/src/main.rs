use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::Parser;
use harness_conformance::fixtures::{scenario_dirs, ScenarioFixture};
use harness_conformance::scenario::run_scenario;
use harness_conformance::stack::StackBins;
use harness_conformance::types::scenario::Classification;

/// Harness conformance E2E runner (spec § Supervisor contract).
#[derive(Debug, Parser)]
#[command(name = "harness-conformance")]
struct Cli {
    /// Path to the pinned iii engine binary. Falls back to $III_BIN.
    #[arg(long, env = "III_BIN")]
    engine_bin: Option<PathBuf>,

    /// Path to the harness binary under test.
    #[arg(long)]
    harness_bin: Option<PathBuf>,

    /// Real worker binaries as name=path (queue, session-manager,
    /// context-manager, iii-directory).
    #[arg(long = "worker-bin", value_parser = parse_worker_bin)]
    worker_bins: Vec<(String, PathBuf)>,

    /// Scenario id, scenario directory name, or `all`.
    #[arg(long, default_value = "all")]
    scenario: String,

    /// Root directory for run artifacts.
    #[arg(long, default_value = "target/conformance")]
    artifacts_dir: PathBuf,

    /// Keep artifacts for passing scenarios too.
    #[arg(long)]
    retain_success: bool,

    /// Load and validate the selected fixtures, then exit without booting a
    /// stack. Exit code 3 (runner_error) on any invalid fixture.
    #[arg(long)]
    validate_only: bool,

    /// Directory containing the checked-in scenarios. Defaults to the
    /// `scenarios/` directory next to this crate's manifest.
    #[arg(long, default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/scenarios"))]
    scenarios_dir: PathBuf,
}

fn parse_worker_bin(raw: &str) -> Result<(String, PathBuf), String> {
    let (name, path) = raw
        .split_once('=')
        .ok_or_else(|| format!("expected name=path, got {raw:?}"))?;
    if name.is_empty() || path.is_empty() {
        return Err(format!("expected name=path, got {raw:?}"));
    }
    Ok((name.to_string(), PathBuf::from(path)))
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let cli = Cli::parse();
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let code = runtime.block_on(run(cli));
    std::process::exit(code);
}

async fn run(cli: Cli) -> i32 {
    let dirs = match scenario_dirs(&cli.scenarios_dir, &cli.scenario) {
        Ok(dirs) => dirs,
        Err(e) => {
            eprintln!("runner_error: {e:#}");
            return 3;
        }
    };

    let mut fixtures = Vec::new();
    for dir in &dirs {
        match ScenarioFixture::load(dir) {
            Ok(fixture) => {
                tracing::info!(scenario = %fixture.scenario.id, dir = %dir.display(), "fixture valid");
                fixtures.push(fixture);
            }
            Err(e) => {
                eprintln!("runner_error: invalid fixture {}: {e:#}", dir.display());
                return 3;
            }
        }
    }

    if cli.validate_only {
        println!("{} scenario fixture(s) valid", fixtures.len());
        return 0;
    }

    let bins = match resolve_bins(&cli) {
        Ok(bins) => bins,
        Err(e) => {
            eprintln!("runner_error: {e:#}");
            return 3;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&cli.artifacts_dir) {
        eprintln!(
            "runner_error: creating {}: {e}",
            cli.artifacts_dir.display()
        );
        return 3;
    }

    // Scenarios run strictly serially, each on a fresh stack (spec § Decisions).
    let mut exit_code = 0;
    for fixture in &fixtures {
        let scenario_id = fixture.scenario.id.clone();
        tracing::info!(scenario = %scenario_id, "running");
        let outcome = run_scenario(&bins, fixture, &cli.artifacts_dir, cli.retain_success).await;
        let classification = outcome.result.classification;
        let failed: Vec<&str> = outcome
            .result
            .invariants
            .iter()
            .filter(|i| !i.passed)
            .map(|i| i.id.as_str())
            .collect();
        println!(
            "{scenario_id}: {} ({} ms, run {}){}",
            classification_str(classification),
            outcome.result.duration_ms,
            outcome.result.run_id,
            if failed.is_empty() {
                String::new()
            } else {
                format!(" — failed invariants: {}", failed.join(", "))
            }
        );
        exit_code = exit_code.max(classification.exit_code());
    }
    exit_code
}

fn classification_str(c: Classification) -> &'static str {
    match c {
        Classification::Pass => "pass",
        Classification::SetupError => "setup_error",
        Classification::ContractFailure => "contract_failure",
        Classification::Timeout => "timeout",
        Classification::ProcessCrash => "process_crash",
        Classification::RunnerError => "runner_error",
    }
}

fn resolve_bins(cli: &Cli) -> anyhow::Result<StackBins> {
    let engine = cli
        .engine_bin
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--engine-bin (or III_BIN) is required; see engine.lock"))?;
    let harness = cli
        .harness_bin
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--harness-bin is required"))?;
    let workers: BTreeMap<String, PathBuf> = cli.worker_bins.iter().cloned().collect();
    let bins = StackBins {
        engine,
        harness,
        workers,
    };
    for (name, path) in std::iter::once(("engine", &bins.engine))
        .chain(std::iter::once(("harness", &bins.harness)))
        .chain(bins.workers.iter().map(|(n, p)| (n.as_str(), p)))
    {
        if !path.is_file() {
            anyhow::bail!("{name} binary not found at {}", path.display());
        }
    }
    let missing = bins.missing_workers();
    if !missing.is_empty() {
        anyhow::bail!("missing --worker-bin for: {}", missing.join(", "));
    }
    Ok(bins)
}
