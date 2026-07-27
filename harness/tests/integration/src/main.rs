use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use harness_integration::fixtures::{scenario_fixtures, ScenarioFixture};
use harness_integration::scenario::{playground_scenario, run_scenario};
use harness_integration::scenarios::ScenarioDriver;
use harness_integration::stack::StackBins;
use harness_integration::types::scenario::Classification;

#[derive(Debug, Parser)]
#[command(
    name = "harness-integration",
    about = "Deterministic harness integration scenarios"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run one direct scenario or every direct scenario.
    Run(RunArgs),
    /// Validate registered fixtures without booting a stack.
    Validate(SelectionArgs),
    /// Boot one stack and Console for manual or Playwright stimulus.
    Playground(PlaygroundArgs),
}

#[derive(Debug, Args)]
struct StackBinArgs {
    /// Path to the pinned iii engine binary. Falls back to $III_BIN.
    #[arg(long, env = "III_BIN")]
    engine_bin: Option<PathBuf>,

    /// Path to the harness binary under test.
    #[arg(long)]
    harness_bin: Option<PathBuf>,

    /// Real worker binaries as name=path.
    #[arg(long = "worker-bin", value_parser = parse_worker_bin)]
    worker_bins: Vec<(String, PathBuf)>,
}

#[derive(Debug, Args)]
struct RunArgs {
    #[command(flatten)]
    selection: SelectionArgs,

    #[command(flatten)]
    bins: StackBinArgs,

    /// Root directory for run artifacts.
    #[arg(long, default_value = "target/integration")]
    artifacts_dir: PathBuf,

    /// Keep heavyweight artifacts for passing scenarios.
    #[arg(long)]
    retain_success: bool,
}

#[derive(Debug, Args)]
struct PlaygroundArgs {
    #[command(flatten)]
    selection: SelectionArgs,

    #[command(flatten)]
    bins: StackBinArgs,

    /// Production Console binary to run against the isolated stack.
    #[arg(long)]
    console_bin: PathBuf,

    /// Optional file atomically published after the Console is ready.
    #[arg(long)]
    ready_file: Option<PathBuf>,

    /// Root directory for run artifacts.
    #[arg(long, default_value = "target/console-e2e")]
    artifacts_dir: PathBuf,
}

#[derive(Debug, Clone, Args)]
struct SelectionArgs {
    /// Scenario id, scenario slug, or `all`.
    #[arg(long, default_value = "all")]
    scenario: String,
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
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let cli = Cli::parse();
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    std::process::exit(runtime.block_on(dispatch(cli)));
}

async fn dispatch(cli: Cli) -> i32 {
    let result = match cli.command {
        Command::Run(args) => return run(args).await,
        Command::Validate(args) => validate(args),
        Command::Playground(args) => return playground(args).await,
    };
    match result {
        Ok(message) => {
            if !message.is_empty() {
                println!("{message}");
            }
            0
        }
        Err(error) => {
            eprintln!("runner_error: {error:#}");
            3
        }
    }
}

async fn run(args: RunArgs) -> i32 {
    let mut fixtures = match load_fixtures(&args.selection) {
        Ok(fixtures) => fixtures,
        Err(error) => {
            eprintln!("runner_error: {error:#}");
            return 3;
        }
    };
    if args.selection.scenario == "all" {
        fixtures.retain(|fixture| fixture.driver == ScenarioDriver::Direct);
    } else if fixtures
        .iter()
        .any(|fixture| fixture.driver != ScenarioDriver::Direct)
    {
        eprintln!(
            "runner_error: scenario {:?} is driven by Playground; use `playground`",
            args.selection.scenario
        );
        return 3;
    }
    let bins = match resolve_stack_bins(&args.bins) {
        Ok(bins) => bins,
        Err(error) => {
            eprintln!("runner_error: {error:#}");
            return 3;
        }
    };
    let artifacts_dir = match prepare_artifacts_dir(&args.artifacts_dir) {
        Ok(dir) => dir,
        Err(error) => {
            eprintln!("runner_error: {error:#}");
            return 3;
        }
    };

    let mut exit_code = 0;
    for fixture in &fixtures {
        let scenario_id = fixture.scenario.id.clone();
        tracing::info!(scenario = %scenario_id, "running");
        let outcome = run_scenario(&bins, fixture, &artifacts_dir, args.retain_success).await;
        let classification = outcome.result.classification;
        println!(
            "{scenario_id}: {}{} — run {} ({} ms), artifacts: {}",
            classification_str(classification),
            match &outcome.result.failure {
                Some(failure) => format!(" — {failure}"),
                None => String::new(),
            },
            outcome.run_id,
            outcome.duration_ms,
            outcome.run_root.display(),
        );
        exit_code = exit_code.max(classification.exit_code());
    }
    exit_code
}

async fn playground(args: PlaygroundArgs) -> i32 {
    if args.selection.scenario == "all" {
        eprintln!("runner_error: playground requires one scenario id or slug");
        return 3;
    }
    let fixture = match load_fixtures(&args.selection).and_then(|fixtures| {
        fixtures
            .into_iter()
            .next()
            .context("playground selector returned no scenario")
    }) {
        Ok(fixture) => fixture,
        Err(error) => {
            eprintln!("runner_error: {error:#}");
            return 3;
        }
    };
    let mut bins = match resolve_stack_bins(&args.bins) {
        Ok(bins) => bins,
        Err(error) => {
            eprintln!("runner_error: {error:#}");
            return 3;
        }
    };
    let console_bin = match absolute_binary("console", &args.console_bin) {
        Ok(bin) => bin,
        Err(error) => {
            eprintln!("runner_error: {error:#}");
            return 3;
        }
    };
    bins.console = Some(console_bin.clone());
    let artifacts_dir = match prepare_artifacts_dir(&args.artifacts_dir) {
        Ok(dir) => dir,
        Err(error) => {
            eprintln!("runner_error: {error:#}");
            return 3;
        }
    };

    let outcome = playground_scenario(
        &bins,
        &console_bin,
        &fixture,
        &artifacts_dir,
        args.ready_file.as_deref(),
    )
    .await;
    println!(
        "{}: {} — run {} ({} ms), artifacts: {}",
        fixture.scenario.id,
        classification_str(outcome.result.classification),
        outcome.run_id,
        outcome.duration_ms,
        outcome.run_root.display(),
    );
    outcome.result.classification.exit_code()
}

fn validate(args: SelectionArgs) -> anyhow::Result<String> {
    let fixtures = load_fixtures(&args)?;
    Ok(format!("{} scenario fixture(s) valid", fixtures.len()))
}

fn load_fixtures(selection: &SelectionArgs) -> anyhow::Result<Vec<ScenarioFixture>> {
    scenario_fixtures(&selection.scenario)
}

fn classification_str(classification: Classification) -> &'static str {
    match classification {
        Classification::Pass => "pass",
        Classification::SetupError => "setup_error",
        Classification::ContractFailure => "contract_failure",
        Classification::Timeout => "timeout",
        Classification::ProcessCrash => "process_crash",
        Classification::RunnerError => "runner_error",
    }
}

fn prepare_artifacts_dir(dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    dir.canonicalize()
        .with_context(|| format!("resolving {}", dir.display()))
}

fn resolve_stack_bins(args: &StackBinArgs) -> anyhow::Result<StackBins> {
    let engine = args
        .engine_bin
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--engine-bin (or III_BIN) is required; see engine.lock"))?;
    let harness = args
        .harness_bin
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--harness-bin is required"))?;
    let bins = StackBins {
        engine: absolute_binary("engine", engine)?,
        harness: absolute_binary("harness", harness)?,
        console: None,
        workers: args
            .worker_bins
            .iter()
            .map(|(name, path)| Ok((name.clone(), absolute_binary(name, path)?)))
            .collect::<anyhow::Result<BTreeMap<String, PathBuf>>>()?,
    };
    let missing = bins.missing_workers();
    if !missing.is_empty() {
        anyhow::bail!("missing --worker-bin for: {}", missing.join(", "));
    }
    Ok(bins)
}

fn absolute_binary(name: &str, path: &Path) -> anyhow::Result<PathBuf> {
    if !path.is_file() {
        anyhow::bail!("{name} binary not found at {}", path.display());
    }
    path.canonicalize()
        .with_context(|| format!("resolving {name} binary {}", path.display()))
}
