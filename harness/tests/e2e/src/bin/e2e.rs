use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use harness_e2e::history;
use harness_e2e::scenarios::{self, ScenarioId};
use harness_e2e::{run_suite, JudgeConfig, SubjectConfig, SuiteRunConfig};

#[derive(Debug, Parser)]
#[command(name = "harness-e2e", about = "Run real-stack quality scenarios")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print the code-defined scenario ids as a JSON array.
    List,
    /// Execute one or more quality scenarios against a running stack.
    Run(RunArgs),
    /// Compare scores only when two reports describe the same experiment.
    Compare(CompareArgs),
}

#[derive(Debug, Args)]
struct RunArgs {
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,

    #[arg(long, env = "HARNESS_E2E_MODEL")]
    model: String,

    #[arg(long, env = "HARNESS_E2E_PROVIDER")]
    provider: String,

    #[arg(long, env = "HARNESS_E2E_JUDGE_MODEL")]
    judge_model: Option<String>,

    #[arg(long, env = "HARNESS_E2E_JUDGE_PROVIDER")]
    judge_provider: Option<String>,

    #[arg(long)]
    output: PathBuf,

    #[arg(long, default_value_t = 1)]
    runs: u32,

    /// Run only the selected scenario. Repeat to select more than one.
    #[arg(long, value_enum)]
    scenario: Vec<ScenarioId>,
}

#[derive(Debug, Args)]
struct CompareArgs {
    #[arg(long)]
    baseline: PathBuf,

    #[arg(long)]
    candidate: PathBuf,

    #[arg(long, default_value_t = 0.0)]
    max_score_drop: f64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    match Cli::parse().command {
        Command::List => {
            let ids: Vec<_> = ScenarioId::ALL
                .iter()
                .map(|scenario| scenario.as_str())
                .collect();
            println!("{}", serde_json::to_string(&ids)?);
            Ok(())
        }
        Command::Run(args) => run(args).await,
        Command::Compare(args) => compare(args),
    }
}

fn compare(args: CompareArgs) -> Result<()> {
    let outcome = history::compare_files(&args.baseline, &args.candidate, args.max_score_drop)
        .context("compare E2E quality history")?;
    println!("{}", serde_json::to_string_pretty(&outcome)?);
    if !outcome.passed {
        bail!("historical E2E comparison failed");
    }
    Ok(())
}

async fn run(args: RunArgs) -> Result<()> {
    let judge = match (args.judge_model, args.judge_provider) {
        (Some(model), Some(provider)) => Some(JudgeConfig { model, provider }),
        (None, None) => None,
        _ => bail!("--judge-model and --judge-provider must be supplied together"),
    };
    let outcome = run_suite(SuiteRunConfig {
        url: args.url,
        subject: SubjectConfig {
            model: args.model,
            provider: args.provider,
        },
        judge,
        output: args.output,
        scenarios: scenarios::selected(&args.scenario),
        runs: args.runs,
    })
    .await
    .context("run E2E quality suite")?;

    if !outcome.report.passed {
        bail!(
            "E2E quality threshold failed; see {}",
            outcome.report_path.display()
        );
    }
    tracing::info!(path = %outcome.report_path.display(), "E2E quality suite passed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_subcommand_needs_no_model_configuration() {
        assert!(matches!(
            Cli::try_parse_from(["harness-e2e", "list"])
                .unwrap()
                .command,
            Command::List
        ));
    }

    #[test]
    fn run_accepts_code_defined_scenario() {
        let cli = Cli::try_parse_from([
            "harness-e2e",
            "run",
            "--model",
            "model",
            "--provider",
            "provider",
            "--output",
            "target/e2e",
            "--scenario",
            "persistent_state",
        ])
        .unwrap();
        let Command::Run(args) = cli.command else {
            panic!("expected run command");
        };
        assert_eq!(args.scenario, [ScenarioId::PersistentState]);
    }

    #[test]
    fn compare_accepts_explicit_reports_and_tolerance() {
        let cli = Cli::try_parse_from([
            "harness-e2e",
            "compare",
            "--baseline",
            "old.json",
            "--candidate",
            "new.json",
            "--max-score-drop",
            "3",
        ])
        .unwrap();
        let Command::Compare(args) = cli.command else {
            panic!("expected compare command");
        };
        assert_eq!(args.max_score_drop, 3.0);
    }
}
