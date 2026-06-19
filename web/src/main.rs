//! `iii-web` binary entry. Full boot sequence is completed in Task 11;
//! this version supports `--manifest` so the crate is exercisable early.

use clap::Parser;

use web::manifest;

#[derive(Parser, Debug)]
#[command(name = "iii-web", about = "Outbound HTTP client on the iii bus (web::fetch).")]
struct Cli {
    #[arg(long)]
    config: Option<String>,
    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,
    #[arg(long)]
    manifest: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    if cli.manifest {
        let m = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m).unwrap());
        return Ok(());
    }

    // Full boot wiring is implemented in Task 11.
    let _ = (cli.config, cli.url);
    Ok(())
}
