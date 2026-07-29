use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};

use crate::judge::JudgeConfig;
use crate::report::E2eReport;
use crate::scenarios::ScenarioId;
use crate::suite::{run_suite, SubjectConfig, SuiteRunConfig};

mod catalog;
mod context;
mod judge;
mod report;
mod scenarios;
mod suite;

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
    /// List models registered in the running stack.
    Models(ModelsArgs),
    /// Execute one or more quality scenarios against a running stack.
    Run(RunArgs),
    /// Print a human-readable summary from a saved results.json.
    #[command(alias = "inspect")]
    Report(ReportArgs),
}

#[derive(Debug, Args)]
struct ModelsArgs {
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,

    /// Show models registered by only this provider.
    #[arg(long)]
    provider: Option<String>,
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

    #[arg(long, env = "HARNESS_E2E_OUTPUT", default_value = "target/e2e")]
    output: PathBuf,

    #[arg(long, default_value_t = 1)]
    runs: u32,

    /// Retry transient provider and transport failures, never quality or resource failures.
    #[arg(long, env = "HARNESS_E2E_TECHNICAL_RETRIES", default_value_t = 1)]
    technical_retries: u8,

    /// Emit a progress heartbeat while a scenario runs. Set to 0 to disable.
    #[arg(
        long,
        env = "HARNESS_E2E_PROGRESS_INTERVAL_SECONDS",
        default_value_t = 15
    )]
    progress_interval_seconds: u64,

    /// Keep score-only regressions advisory while still failing hard gates and errors.
    #[arg(long, env = "HARNESS_E2E_QUALITY_ADVISORY", default_value_t = false)]
    quality_advisory: bool,

    /// Run only the selected scenario. Repeat to select more than one.
    #[arg(long, value_enum)]
    scenario: Vec<ScenarioId>,
}

#[derive(Debug, Args)]
struct ReportArgs {
    /// Path to results.json or to the directory containing it.
    input: PathBuf,

    /// Include passing gates and fully awarded criteria.
    #[arg(long)]
    verbose: bool,
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
        Command::Models(args) => models(args).await,
        Command::Run(args) => run(args).await,
        Command::Report(args) => report(args),
    }
}

async fn models(args: ModelsArgs) -> Result<()> {
    let context = context::E2eContext::connect(&args.url)
        .await
        .context("connect to the iii stack")?;
    let result = catalog::list(&context, args.provider.as_deref()).await;
    context.shutdown().await;
    let models = result?;
    print!("{}", catalog::summary(&models));
    Ok(())
}

fn report(args: ReportArgs) -> Result<()> {
    let (report, path) = E2eReport::read_from(&args.input)?;
    print!("{}", report.summary(args.verbose));
    println!("report: {}", path.display());
    Ok(())
}

async fn run(args: RunArgs) -> Result<()> {
    let quality_advisory = args.quality_advisory;
    let selected_scenarios = scenarios::selected(&args.scenario);
    let subject = SubjectConfig {
        model: args.model,
        provider: args.provider,
    };
    let judge = judge_config(
        &subject,
        args.judge_model,
        args.judge_provider,
        selected_scenarios
            .iter()
            .any(|scenario| scenario.spec("judge-check").needs_judge()),
    );
    let outcome = run_suite(SuiteRunConfig {
        url: args.url,
        subject,
        judge,
        output: args.output,
        scenarios: selected_scenarios,
        runs: args.runs,
        technical_retries: args.technical_retries,
        progress_interval: (args.progress_interval_seconds > 0)
            .then(|| std::time::Duration::from_secs(args.progress_interval_seconds)),
    })
    .await
    .context("run E2E quality suite")?;

    print!("{}", outcome.report.summary(false));
    println!("report: {}", outcome.report_path.display());
    if !outcome.report.passed && !(quality_advisory && !outcome.report.has_non_quality_failure()) {
        bail!("E2E suite failed");
    }
    if !outcome.report.passed {
        tracing::warn!(
            path = %outcome.report_path.display(),
            "E2E score is below threshold; quality gate is advisory"
        );
        return Ok(());
    }
    tracing::info!(path = %outcome.report_path.display(), "E2E quality suite passed");
    Ok(())
}

fn judge_config(
    subject: &SubjectConfig,
    model: Option<String>,
    provider: Option<String>,
    required: bool,
) -> Option<JudgeConfig> {
    if !required && model.is_none() && provider.is_none() {
        return None;
    }
    Some(JudgeConfig {
        model: model.unwrap_or_else(|| subject.model.clone()),
        provider: provider.unwrap_or_else(|| subject.provider.clone()),
    })
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
    fn models_subcommand_accepts_an_optional_provider() {
        let cli =
            Cli::try_parse_from(["harness-e2e", "models", "--provider", "openai-codex"]).unwrap();
        let Command::Models(args) = cli.command else {
            panic!("expected models command");
        };
        assert_eq!(args.provider.as_deref(), Some("openai-codex"));
        assert_eq!(args.url, "ws://127.0.0.1:49134");
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
            "--scenario",
            "persistent_state",
        ])
        .unwrap();
        let Command::Run(args) = cli.command else {
            panic!("expected run command");
        };
        assert_eq!(args.scenario, [ScenarioId::PersistentState]);
        assert_eq!(args.output, PathBuf::from("target/e2e"));
        assert_eq!(args.technical_retries, 1);
        assert_eq!(args.progress_interval_seconds, 15);
    }

    #[test]
    fn judge_defaults_to_the_subject_when_required() {
        let subject = SubjectConfig {
            model: "model".into(),
            provider: "provider".into(),
        };
        let judge = judge_config(&subject, None, None, true).unwrap();
        assert_eq!(judge.model, subject.model);
        assert_eq!(judge.provider, subject.provider);

        assert!(judge_config(&subject, None, None, false).is_none());
    }

    #[test]
    fn report_subcommand_accepts_a_results_directory() {
        let cli = Cli::try_parse_from(["harness-e2e", "report", "target/e2e"]).unwrap();
        let Command::Report(args) = cli.command else {
            panic!("expected report command");
        };
        assert_eq!(args.input, PathBuf::from("target/e2e"));
    }
}
