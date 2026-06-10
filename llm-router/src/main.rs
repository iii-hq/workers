use std::sync::Arc;

use iii_sdk::{register_worker, InitOptions};
use llm_router::bus_sdk::SdkBus;
use llm_router::channels::SdkChannels;
use llm_router::register::register_router;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Registry publish pipeline: print the manifest JSON and exit.
    if std::env::args().nth(1).as_deref() == Some("--manifest") {
        println!(
            "{}",
            serde_json::to_string_pretty(&llm_router::manifest::build_manifest())?
        );
        return Ok(());
    }

    let url = std::env::var("III_WS_URL").unwrap_or_else(|_| "ws://localhost:49134".to_string());
    let iii = register_worker(&url, InitOptions::default());

    let bus = Arc::new(SdkBus { iii: iii.clone() });
    let channels = Arc::new(SdkChannels { iii: iii.clone() });
    register_router(bus, channels).await?;
    println!("[llm-router] registered against {url}");

    tokio::signal::ctrl_c().await?;
    iii.shutdown();
    Ok(())
}
