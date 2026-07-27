use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Parser;
use harness_e2e::scenarios::{self, ScenarioId};
use harness_e2e::{run_suite, subject, LimitsArgs, SuiteRunConfig};

#[derive(Debug, Parser)]
#[command(name = "harness-e2e", about = "Run real-model harness E2E scenarios")]
struct Cli {
    #[arg(long)]
    subject: PathBuf,

    #[arg(long)]
    output: PathBuf,

    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,

    #[command(flatten)]
    limits: LimitsArgs,

    /// Run only the selected scenario. Repeat to select more than one.
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
    let subject = subject::load(&cli.subject).map_err(anyhow::Error::msg)?;
    let limits = cli.limits.resolve().map_err(anyhow::Error::msg)?;
    let outcome = run_suite(SuiteRunConfig {
        url: cli.url,
        subject,
        output: cli.output,
        scenarios: scenarios::selected(&cli.scenario),
        limits,
    })
    .await
    .map_err(anyhow::Error::msg)
    .context("run e2e suite")?;

    if !outcome.report.passed {
        bail!(
            "e2e scenario failed; see {}/results.json",
            outcome.run_dir.display()
        );
    }
    tracing::info!(path = %outcome.run_dir.display(), "e2e run passed");
    Ok(())
}
