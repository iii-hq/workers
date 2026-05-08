use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "iii-skill-check", about = "Validate worker README/skill artifacts against project rules")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Render docs/* + introspection into README.md, skill.md, skills/*.md.
    Render { worker: PathBuf },
    /// Run all configured layers against a worker.
    Verify {
        worker: PathBuf,
        /// Subset of layers to run: structure,vale,ai (comma-separated).
        #[arg(long, default_value = "structure,vale,ai")]
        layers: String,
    },
    /// Re-render and diff against checked-in artifacts; non-zero on drift.
    VerifyRendered { worker: PathBuf },
    /// Run the validator's self-test fixtures.
    Fixtures,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Render { worker } => {
            let out = iii_skill_check::render::render_worker(&worker)?;
            println!(
                "rendered {} (readme {} bytes, skill {} bytes, {} leaves)",
                worker.display(),
                out.readme.len(),
                out.skill.len(),
                out.leaves.len(),
            );
            Ok(())
        }
        Command::Verify { worker, layers } => {
            anyhow::bail!(
                "verify not yet implemented (worker={}, layers={})",
                worker.display(),
                layers
            )
        }
        Command::VerifyRendered { worker } => {
            anyhow::bail!(
                "verify-rendered not yet implemented (worker={})",
                worker.display()
            )
        }
        Command::Fixtures => {
            anyhow::bail!("fixtures not yet implemented")
        }
    }
}
