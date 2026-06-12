use iii_sdk::{register_worker, InitOptions};
use provider_anthropic::register::register_provider;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Registry publish pipeline: print the manifest JSON and exit.
    if std::env::args().nth(1).as_deref() == Some("--manifest") {
        println!(
            "{}",
            serde_json::to_string_pretty(&provider_anthropic::manifest::build_manifest())?
        );
        return Ok(());
    }

    let url = std::env::var("III_WS_URL").unwrap_or_else(|_| "ws://localhost:49134".to_string());
    let iii = register_worker(&url, InitOptions::default());

    register_provider(iii.clone()).await?;
    println!("[provider-anthropic] registered against {url}");

    tokio::signal::ctrl_c().await?;
    iii.shutdown();
    Ok(())
}
