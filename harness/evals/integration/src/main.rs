use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Parser;
use harness_integration::fixtures::{scenario_dirs, ScenarioFixture};
use harness_integration::scenario::run_scenario;
use harness_integration::stack::StackBins;
use harness_integration::types::scenario::Classification;

/// Harness integration E2E runner (spec § Supervisor contract).
#[derive(Debug, Parser)]
#[command(name = "harness-integration")]
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

    /// Optional console worker binary: spawned per-run with the stack so a
    /// browser can watch (and record) the scenario's chat live.
    #[arg(long)]
    console_bin: Option<PathBuf>,

    /// Record the console chat view to scenarios/<id>/console-recording.webm
    /// via headless system Chrome (diagnostics only, never an oracle).
    /// Requires --console-bin, `node` on PATH, and an installed
    /// tools/console-recorder/node_modules.
    #[arg(long, requires = "console_bin")]
    record_console: bool,

    /// Scenario id, scenario directory name, or `all`.
    #[arg(long, default_value = "all")]
    scenario: String,

    /// Root directory for run artifacts.
    #[arg(long, default_value = "target/integration")]
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
    // Children run with per-run working directories (stack.rs), so a relative
    // artifacts dir would make every generated config/seed path resolve
    // against the wrong cwd inside the spawned processes.
    let artifacts_dir = match cli.artifacts_dir.canonicalize() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!(
                "runner_error: resolving {}: {e}",
                cli.artifacts_dir.display()
            );
            return 3;
        }
    };

    // Scenarios run strictly serially, each on a fresh stack (spec § Decisions).
    let mut exit_code = 0;
    for fixture in &fixtures {
        let scenario_id = fixture.scenario.id.clone();
        tracing::info!(scenario = %scenario_id, "running");
        let outcome = run_scenario(
            &bins,
            fixture,
            &artifacts_dir,
            cli.retain_success,
            cli.record_console,
        )
        .await;
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
    // Children are spawned with per-run working directories, so a relative
    // binary path from the CLI would fail to exec inside the child.
    fn absolute(name: &str, path: &Path) -> anyhow::Result<PathBuf> {
        if !path.is_file() {
            anyhow::bail!("{name} binary not found at {}", path.display());
        }
        path.canonicalize()
            .with_context(|| format!("resolving {name} binary {}", path.display()))
    }
    let engine = cli
        .engine_bin
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--engine-bin (or III_BIN) is required; see engine.lock"))?;
    let harness = cli
        .harness_bin
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--harness-bin is required"))?;
    let bins = StackBins {
        engine: absolute("engine", engine)?,
        harness: absolute("harness", harness)?,
        workers: cli
            .worker_bins
            .iter()
            .map(|(name, path)| Ok((name.clone(), absolute(name, path)?)))
            .collect::<anyhow::Result<BTreeMap<String, PathBuf>>>()?,
        console: cli
            .console_bin
            .as_deref()
            .map(|p| absolute("console", p))
            .transpose()?,
    };
    let missing = bins.missing_workers();
    if !missing.is_empty() {
        anyhow::bail!("missing --worker-bin for: {}", missing.join(", "));
    }
    Ok(bins)
}
