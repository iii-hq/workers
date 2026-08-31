use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, RegisterFunction};
use std::sync::Arc;

mod config;
mod handler;
mod processing;

#[derive(Parser, Debug)]
#[command(name = "image-resize", about = "III engine image resize module")]
struct Cli {
    /// Path to config.yaml file
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    /// WebSocket URL of the III engine (port 49134 = engine main WS, not StreamModule 3112)
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,

    /// Namespace to register under. Absent uses the engine's default namespace.
    /// Falls back to the III_NAMESPACE env var.
    #[arg(long, env = "III_NAMESPACE")]
    namespace: Option<String>,
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

    let resize_config = match config::load_config(&cli.config) {
        Ok(c) => {
            tracing::info!(
                width = c.width,
                height = c.height,
                strategy = ?c.strategy,
                "loaded config from {}",
                cli.config
            );
            c
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::ResizeConfig::default()
        }
    };

    let config = Arc::new(resize_config);

    tracing::info!(url = %cli.url, namespace = ?cli.namespace, "connecting to III engine");

    let iii = register_worker(
        &cli.url,
        InitOptions {
            namespace: cli.namespace.clone(),
            ..Default::default()
        },
    );

    let resize_handler = handler::build_handler(cli.url.clone(), config);

    let _fn_ref = iii.register_function(
        "image_resize::resize",
        RegisterFunction::new_async(resize_handler)
            .description("Resize an image via channel I/O")
            .request_format(serde_json::json!({
                "type": "object",
                "properties": {
                    "input_channel": {
                        "type": "object",
                        "description": "StreamChannelRef (read) carrying metadata text + image binary"
                    },
                    "output_channel": {
                        "type": "object",
                        "description": "StreamChannelRef (write) for thumbnail output"
                    },
                    "metadata": {
                        "type": "object",
                        "description": "Optional inline ImageMetadata override",
                        "properties": {
                            "format": { "type": "string", "enum": ["jpeg", "png", "webp"], "description": "Source image format" },
                            "output_format": { "type": "string", "enum": ["jpeg", "png", "webp"], "description": "Desired output format (defaults to source format)" },
                            "width": { "type": "integer" },
                            "height": { "type": "integer" },
                            "quality": { "type": "integer" },
                            "strategy": { "type": "string", "enum": ["scale-to-fit", "crop-to-fit"] },
                            "target_width": { "type": "integer" },
                            "target_height": { "type": "integer" }
                        }
                    }
                },
                "required": ["input_channel", "output_channel"]
            }))
            .response_format(serde_json::json!({
                "type": "object",
                "properties": {
                    "format": { "type": "string" },
                    "width": { "type": "integer" },
                    "height": { "type": "integer" },
                    "strategy": { "type": "string" }
                }
            })),
    );

    tracing::info!("image_resize::resize function registered, waiting for invocations");

    tokio::signal::ctrl_c().await?;

    tracing::info!("image-resize shutting down");
    iii.shutdown_async().await;

    Ok(())
}
