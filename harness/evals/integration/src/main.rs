use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Args, Parser, Subcommand, ValueEnum};
use harness_integration::expand::{
    compile_scenario, render_authored_yaml, render_compiled, scenario_template,
    ScenarioTemplateKind,
};
use harness_integration::fixtures::{
    ensure_scenario_id_available, scenario_fixtures, ScenarioFixture,
};
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
    /// Compile and validate fixtures without booting a stack.
    Validate(SelectionArgs),
    /// Print one deterministic, fully expanded compiled scenario.
    Render(RenderArgs),
    /// Create one valid single-file scenario without overwriting files.
    Init(InitArgs),
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
}

#[derive(Debug, Clone, Args)]
struct SelectionArgs {
    /// Scenario id, scenario directory name, or `all`.
    #[arg(long, default_value = "all")]
    scenario: String,

    /// Directory containing the checked-in scenarios.
    #[arg(long, default_value = DEFAULT_SCENARIOS_DIR)]
    scenarios_dir: PathBuf,
}

#[derive(Debug, Args)]
struct RenderArgs {
    /// Scenario id or directory name.
    scenario: String,

    #[arg(long, default_value = DEFAULT_SCENARIOS_DIR)]
    scenarios_dir: PathBuf,
}

#[derive(Debug, Args)]
struct InitArgs {
    #[arg(long)]
    id: String,

    /// Directory slug created below the scenarios directory.
    #[arg(long)]
    name: String,

    #[arg(long)]
    description: String,

    #[arg(long, value_enum)]
    kind: TemplateKind,

    #[arg(long, default_value = DEFAULT_SCENARIOS_DIR)]
    scenarios_dir: PathBuf,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TemplateKind {
    Text,
    Function,
    Hook,
    Crash,
}

impl From<TemplateKind> for ScenarioTemplateKind {
    fn from(value: TemplateKind) -> Self {
        match value {
            TemplateKind::Text => Self::Text,
            TemplateKind::Function => Self::Function,
            TemplateKind::Hook => Self::Hook,
            TemplateKind::Crash => Self::Crash,
        }
    }
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
        Command::Render(args) => render(args),
        Command::Init(args) => init(args),
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
        tracing::info!(scenario = %scenario_id, "running");
        let outcome = run_scenario(&bins, fixture, &artifacts_dir, args.retain_success).await;
        let classification = outcome.result.classification;
        let failed: Vec<&str> = outcome
            .result
            .invariants
            .iter()
            .filter(|invariant| !invariant.passed)
            .map(|invariant| invariant.id.as_str())
            .collect();
        println!(
            "{scenario_id}: {}{} — run {} ({} ms), artifacts: {}",
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

fn init(args: InitArgs) -> anyhow::Result<String> {
    validate_slug(&args.name)?;
    anyhow::ensure!(!args.id.trim().is_empty(), "--id must not be empty");
    anyhow::ensure!(
        !args.description.trim().is_empty(),
        "--description must not be empty"
    );

    let target = args.scenarios_dir.join(&args.name);
    anyhow::ensure!(
        !target.exists(),
        "refusing to overwrite existing scenario directory {}",
        target.display()
    );
    let authored = scenario_template(&args.id, &args.description, args.kind.into());
    let prompt_path = args.scenarios_dir.join("system-prompt.txt");
    let prompt = std::fs::read_to_string(&prompt_path)
        .with_context(|| format!("reading shared prompt {}", prompt_path.display()))?;
    compile_scenario(&authored, &prompt).context("generated scenario is invalid")?;
    ensure_scenario_id_available(&args.scenarios_dir, &args.id)?;
    let yaml = render_authored_yaml(&authored)?;

    let scenario_path = write_scenario_atomically(&target, &yaml)?;
    Ok(format!("created {}", scenario_path.display()))
}

fn write_scenario_atomically(target: &Path, yaml: &str) -> anyhow::Result<PathBuf> {
    let parent = target
        .parent()
        .context("scenario target has no parent directory")?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .context("scenario target name is not UTF-8")?;
    let temporary = parent.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir(&temporary)
        .with_context(|| format!("creating temporary scenario {}", temporary.display()))?;
    let temporary_file = temporary.join("scenario.yaml");
    let outcome = std::fs::write(&temporary_file, yaml)
        .with_context(|| format!("writing {}", temporary_file.display()))
        .and_then(|()| {
            std::fs::rename(&temporary, target).with_context(|| {
                format!(
                    "publishing temporary scenario {} as {}",
                    temporary.display(),
                    target.display()
                )
            })
        });
    if let Err(error) = outcome {
        let _ = std::fs::remove_dir_all(&temporary);
        return Err(error);
    }
    Ok(target.join("scenario.yaml"))
}

fn validate_slug(slug: &str) -> anyhow::Result<()> {
    let valid = !slug.is_empty()
        && slug != "."
        && slug != ".."
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    anyhow::ensure!(
        valid,
        "--name must contain only ASCII letters, digits, '-' or '_'"
    );
    Ok(())
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
    fn scenario_names_cannot_escape_the_root() {
        for invalid in ["", ".", "..", "../escape", "nested/name", "with space"] {
            assert!(validate_slug(invalid).is_err(), "{invalid:?}");
        }
        for valid in ["streamed-text", "case_507", "C505"] {
            validate_slug(valid).unwrap();
        }
    }
}
