use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "iii-http", about = "HTTP server worker for iii.")]
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
    let cli = Cli::parse();
    if cli.manifest {
        println!("{}", serde_json::to_string_pretty(&iii_http::manifest::build_manifest()).unwrap());
        return Ok(());
    }
    println!("boot wired in later phases");
    Ok(())
}
