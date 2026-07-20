use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use harness_integration::expand::render_compiled;
use harness_integration::fixtures::{scenario_fixtures, ScenarioFixture};
use harness_integration::scenario::run_scenario;
use harness_integration::stack::StackBins;
use harness_integration::types::scenario::Classification;

const DEFAULT_SCENARIOS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scenarios");

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
    /// Run one scenario or every non-quarantined scenario.
    Run(RunArgs),
    /// Compile and validate every registered scenario without booting a
    /// stack.
    Validate(SelectionArgs),
    /// Print one deterministic, fully expanded compiled scenario.
    Render(RenderArgs),
}

#[derive(Debug, Args)]
struct RunArgs {
    #[command(flatten)]
    selection: SelectionArgs,

    /// Path to the pinned iii engine binary. Falls back to $III_BIN.
    #[arg(long, env = "III_BIN")]
    engine_bin: Option<PathBuf>,

    /// Path to the harness binary under test.
    #[arg(long)]
    harness_bin: Option<PathBuf>,

    /// Real worker binaries as name=path.
    #[arg(long = "worker-bin", value_parser = parse_worker_bin)]
    worker_bins: Vec<(String, PathBuf)>,

    /// Root directory for run artifacts.
    #[arg(long, default_value = "target/integration")]
    artifacts_dir: PathBuf,

    /// Keep heavyweight artifacts for passing scenarios.
    #[arg(long)]
    retain_success: bool,

    /// Run every selected scenario this many times and require an identical
    /// stable result from each repetition.
    #[arg(long, default_value_t = 1, value_parser = parse_repeat)]
    repeat: u16,
}

#[derive(Debug, Clone, Args)]
struct SelectionArgs {
    /// Scenario id, scenario slug, or `all`.
    #[arg(long, default_value = "all")]
    scenario: String,

    /// Directory containing the shared system prompt template.
    #[arg(long, default_value = DEFAULT_SCENARIOS_DIR)]
    scenarios_dir: PathBuf,
}

#[derive(Debug, Args)]
struct RenderArgs {
    /// Scenario id or slug.
    scenario: String,

    #[arg(long, default_value = DEFAULT_SCENARIOS_DIR)]
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

fn parse_repeat(raw: &str) -> Result<u16, String> {
    let repeat = raw
        .parse::<u16>()
        .map_err(|error| format!("invalid repeat count {raw:?}: {error}"))?;
    if repeat == 0 {
        return Err("repeat count must be at least 1".to_string());
    }
    Ok(repeat)
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
        Command::Render(args) => render(args),
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
    let fixtures = match load_fixtures(&args.selection, false) {
        Ok(fixtures) => fixtures,
        Err(error) => {
            eprintln!("runner_error: {error:#}");
            return 3;
        }
    };
    let bins = match resolve_bins(&args) {
        Ok(bins) => bins,
        Err(error) => {
            eprintln!("runner_error: {error:#}");
            return 3;
        }
    };
    if let Err(error) = std::fs::create_dir_all(&args.artifacts_dir) {
        eprintln!(
            "runner_error: creating {}: {error}",
            args.artifacts_dir.display()
        );
        return 3;
    }
    let artifacts_dir = match args.artifacts_dir.canonicalize() {
        Ok(dir) => dir,
        Err(error) => {
            eprintln!(
                "runner_error: resolving {}: {error}",
                args.artifacts_dir.display()
            );
            return 3;
        }
    };

    let mut exit_code = 0;
    for fixture in &fixtures {
        let scenario_id = fixture.scenario.id.clone();
        let mut stable_result = None;
        for repetition in 1..=args.repeat {
            tracing::info!(
                scenario = %scenario_id,
                repetition,
                repeat = args.repeat,
                "running"
            );
            let outcome = run_scenario(&bins, fixture, &artifacts_dir, args.retain_success).await;
            let classification = outcome.result.classification;
            let failed: Vec<&str> = outcome
                .result
                .invariants
                .iter()
                .filter(|invariant| !invariant.passed)
                .map(|invariant| invariant.id.as_str())
                .collect();
            let repetition_label = if args.repeat > 1 {
                format!(" [{repetition}/{}]", args.repeat)
            } else {
                String::new()
            };
            println!(
                "{scenario_id}{repetition_label}: {}{} — run {} ({} ms), artifacts: {}",
                classification_str(classification),
                if failed.is_empty() {
                    String::new()
                } else {
                    format!(" — failed invariants: {}", failed.join(", "))
                },
                outcome.run_id,
                outcome.duration_ms,
                outcome.run_root.display(),
            );
            exit_code = exit_code.max(classification.exit_code());

            match &stable_result {
                None => stable_result = Some(outcome.result),
                Some(expected) if expected == &outcome.result => {}
                Some(_) => {
                    eprintln!(
                        "runner_error: {scenario_id} produced a different stable result on repetition {repetition}"
                    );
                    exit_code = exit_code.max(3);
                }
            }
        }
    }
    exit_code
}

fn validate(args: SelectionArgs) -> anyhow::Result<String> {
    let fixtures = load_fixtures(&args, true)?;
    Ok(format!("{} scenario fixture(s) valid", fixtures.len()))
}

fn render(args: RenderArgs) -> anyhow::Result<String> {
    anyhow::ensure!(
        args.scenario != "all",
        "render requires one scenario id or slug"
    );
    let selection = SelectionArgs {
        scenario: args.scenario,
        scenarios_dir: args.scenarios_dir,
    };
    let fixtures = load_fixtures(&selection, true)?;
    let fixture = fixtures
        .into_iter()
        .next()
        .context("render selector returned no scenario")?;
    render_compiled(&fixture.compiled())
}

fn load_fixtures(
    selection: &SelectionArgs,
    include_quarantined: bool,
) -> anyhow::Result<Vec<ScenarioFixture>> {
    scenario_fixtures(
        &selection.scenarios_dir,
        &selection.scenario,
        include_quarantined,
    )
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

fn resolve_bins(args: &RunArgs) -> anyhow::Result<StackBins> {
    fn absolute(name: &str, path: &Path) -> anyhow::Result<PathBuf> {
        if !path.is_file() {
            anyhow::bail!("{name} binary not found at {}", path.display());
        }
        path.canonicalize()
            .with_context(|| format!("resolving {name} binary {}", path.display()))
    }

    let engine = args
        .engine_bin
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--engine-bin (or III_BIN) is required; see engine.lock"))?;
    let harness = args
        .harness_bin
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--harness-bin is required"))?;
    let bins = StackBins {
        engine: absolute("engine", engine)?,
        harness: absolute("harness", harness)?,
        workers: args
            .worker_bins
            .iter()
            .map(|(name, path)| Ok((name.clone(), absolute(name, path)?)))
            .collect::<anyhow::Result<BTreeMap<String, PathBuf>>>()?,
    };
    let missing = bins.missing_workers();
    if !missing.is_empty() {
        anyhow::bail!("missing --worker-bin for: {}", missing.join(", "));
    }
    Ok(bins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_count_must_be_positive() {
        assert_eq!(parse_repeat("1").unwrap(), 1);
        assert_eq!(parse_repeat("2").unwrap(), 2);
        assert!(parse_repeat("0").is_err());
        assert!(parse_repeat("many").is_err());
    }
}
