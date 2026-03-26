use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, RegisterFunctionMessage};
use std::sync::Arc;

mod config;
mod handler;
mod manifest;
mod processing;

#[derive(Parser, Debug)]
#[command(name = "image-resize", about = "III engine image resize module")]
struct Cli {
    /// Path to config.yaml file
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    /// WebSocket URL of the III engine (port 49134 = engine main WS, not StreamModule 3112)
    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,

    /// Output worker manifest as YAML and exit
    #[arg(long)]
    manifest: bool,
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

    if cli.manifest {
        let manifest = manifest::build_manifest();
        let yaml = serde_yaml::to_string(&manifest).expect("failed to serialize manifest");
        print!("{}", yaml);
        return Ok(());
    }

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

    tracing::info!(url = %cli.url, "connecting to III engine");

    // TODO: Once iii-sdk publishes manifest support (WorkerManifestCompact + InitOptions.manifest),
    // load the embedded manifest from the OCI well-known path and pass it during registration:
    //
    //   let manifest = iii_sdk::WorkerManifestCompact::from_file("/iii/worker.yaml").ok();
    //
    // Then set `manifest` in InitOptions below. Until then, we rely on Default (manifest: None).
    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );

    let resize_handler = handler::build_handler(cli.url.clone(), config);

    let _fn_ref = iii.register_function(
        RegisterFunctionMessage {
            id: "image_resize::resize".to_string(),
            description: Some("Resize an image via channel I/O".to_string()),
            request_format: Some(serde_json::json!({
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
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "format": { "type": "string" },
                    "width": { "type": "integer" },
                    "height": { "type": "integer" },
                    "strategy": { "type": "string" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        resize_handler,
    );

    tracing::info!("image_resize::resize function registered, waiting for invocations");

    tokio::signal::ctrl_c().await?;

    tracing::info!("image-resize shutting down");
    iii.shutdown_async().await;

    Ok(())
}
