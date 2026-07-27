use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Parser;
use harness_e2e::scenarios::{self, ScenarioId};
use harness_e2e::{
    compare_runs, comparison_dimension, run_suite, subject, E2eLimitsV1, E2eRunReportV1,
    LimitsArgs, ResolvedE2eSubjectV1, SuiteRunConfig,
};

#[derive(Debug, Parser)]
#[command(
    name = "harness-prompt-eval",
    about = "Compare two harness subjects while changing only the model or system prompt"
)]
struct Cli {
    #[arg(long)]
    control: PathBuf,
    #[arg(long)]
    treatment: PathBuf,
    #[arg(long, default_value_t = 3)]
    runs: u32,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    #[command(flatten)]
    limits: LimitsArgs,
    #[arg(long, value_enum)]
    scenario: Vec<ScenarioId>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let cli = Cli::parse();
    if cli.runs == 0 {
        bail!("--runs must be greater than zero");
    }
    let limits = cli.limits.resolve().map_err(anyhow::Error::msg)?;

    let control = subject::load(&cli.control).map_err(anyhow::Error::msg)?;
    let treatment = subject::load(&cli.treatment).map_err(anyhow::Error::msg)?;
    let dimension = comparison_dimension(&control, &treatment).map_err(anyhow::Error::msg)?;

    let scenarios = scenarios::selected(&cli.scenario);
    let mut control_runs = Vec::with_capacity(cli.runs as usize);
    let mut treatment_runs = Vec::with_capacity(cli.runs as usize);

    for repetition in 0..cli.runs {
        let order = if repetition % 2 == 0 {
            [true, false]
        } else {
            [false, true]
        };
        for is_control in order {
            let (variant, subject) = if is_control {
                ("control", control.clone())
            } else {
                ("treatment", treatment.clone())
            };
            let report =
                run_variant(&cli, variant, repetition, subject, &scenarios, limits).await?;
            if is_control {
                control_runs.push(report);
            } else {
                treatment_runs.push(report);
            }
        }
    }

    let comparison = compare_runs(
        dimension,
        control.artifact(),
        treatment.artifact(),
        &scenarios,
        &control_runs,
        &treatment_runs,
    )
    .map_err(anyhow::Error::msg)?;
    let path = comparison
        .write_to(&cli.output)
        .map_err(anyhow::Error::msg)
        .context("write prompt comparison")?;

    if !comparison.eligible {
        bail!(
            "treatment failed the correctness gate; see {}",
            path.display()
        );
    }
    tracing::info!(path = %path.display(), "prompt comparison passed");
    Ok(())
}

async fn run_variant(
    cli: &Cli,
    variant: &str,
    repetition: u32,
    subject: ResolvedE2eSubjectV1,
    scenarios: &[ScenarioId],
    limits: E2eLimitsV1,
) -> Result<E2eRunReportV1> {
    tracing::info!(
        variant,
        run = repetition + 1,
        total_runs = cli.runs,
        "running prompt evaluation variant"
    );
    let outcome = run_suite(SuiteRunConfig {
        url: cli.url.clone(),
        subject,
        output: cli.output.join("runs").join(variant),
        scenarios: scenarios.to_vec(),
        limits,
    })
    .await
    .map_err(anyhow::Error::msg)
    .with_context(|| format!("run {variant} repetition {}", repetition + 1))?;
    Ok(outcome.report)
}
