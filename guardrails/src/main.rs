use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, RegisterFunctionMessage, RegisterTriggerInput};
use std::sync::Arc;

mod checks;
mod config;
mod functions;
mod manifest;
mod state;

#[derive(Parser, Debug)]
#[command(name = "iii-guardrails", about = "III engine guardrails safety layer")]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,

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
        println!("{}", serde_json::to_string_pretty(&manifest).unwrap());
        return Ok(());
    }

    let guardrails_config = match config::load_config(&cli.config) {
        Ok(c) => {
            tracing::info!(
                pii_patterns = c.pii_patterns.len(),
                injection_keywords = c.injection_keywords.len(),
                max_input_length = c.max_input_length,
                max_output_length = c.max_output_length,
                "loaded config from {}",
                cli.config
            );
            c
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::GuardrailsConfig::default()
        }
    };

    let compiled_patterns = Arc::new(guardrails_config.compile_pii_patterns());
    let compiled_secrets = Arc::new(crate::checks::compile_secret_patterns());
    tracing::info!(
        pii = compiled_patterns.len(),
        secrets = compiled_secrets.len(),
        "compiled regex patterns"
    );

    let cfg = Arc::new(guardrails_config);

    tracing::info!(url = %cli.url, "connecting to III engine");

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );

    let iii_arc = Arc::new(iii.clone());

    {
        let iii_c = iii_arc.clone();
        let cfg_c = cfg.clone();
        let patterns_c = compiled_patterns.clone();
        iii.register_function((
            RegisterFunctionMessage {
                id: "guardrails::check_input".to_string(),
                description: Some(
                    "Check input text for PII, injection attacks, and length violations".to_string(),
                ),
                request_format: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "text": { "type": "string", "description": "Input text to check" },
                        "context": {
                            "type": "object",
                            "description": "Optional context metadata",
                            "properties": {
                                "function_id": { "type": "string" },
                                "user_id": { "type": "string" }
                            }
                        }
                    },
                    "required": ["text"]
                })),
                response_format: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "passed": { "type": "boolean" },
                        "risk": { "type": "string", "enum": ["none", "low", "medium", "high"] },
                        "pii": { "type": "array" },
                        "injections": { "type": "array" },
                        "over_length": { "type": "boolean" },
                        "check_id": { "type": "string" }
                    }
                })),
                metadata: None,
                invocation: None,
            },
            move |payload: serde_json::Value| {
                let iii_c = iii_c.clone();
                let cfg_c = cfg_c.clone();
                let patterns_c = patterns_c.clone();
                Box::pin(async move {
                    functions::check_input::handle(iii_c, cfg_c, patterns_c, payload).await
                })
                    as std::pin::Pin<
                        Box<
                            dyn std::future::Future<
                                    Output = Result<serde_json::Value, iii_sdk::IIIError>,
                                > + Send,
                        >,
                    >
            },
        ));
    }

    {
        let iii_c = iii_arc.clone();
        let cfg_c = cfg.clone();
        let patterns_c = compiled_patterns.clone();
        let secrets_c = compiled_secrets.clone();
        iii.register_function((
            RegisterFunctionMessage {
                id: "guardrails::check_output".to_string(),
                description: Some(
                    "Check output text for PII, leaked secrets, and length violations".to_string(),
                ),
                request_format: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "text": { "type": "string", "description": "Output text to check" },
                        "context": {
                            "type": "object",
                            "description": "Optional context metadata",
                            "properties": {
                                "function_id": { "type": "string" },
                                "user_id": { "type": "string" }
                            }
                        }
                    },
                    "required": ["text"]
                })),
                response_format: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "passed": { "type": "boolean" },
                        "risk": { "type": "string", "enum": ["none", "low", "medium", "high"] },
                        "pii": { "type": "array" },
                        "secrets": { "type": "array" },
                        "over_length": { "type": "boolean" },
                        "check_id": { "type": "string" }
                    }
                })),
                metadata: None,
                invocation: None,
            },
            move |payload: serde_json::Value| {
                let iii_c = iii_c.clone();
                let cfg_c = cfg_c.clone();
                let patterns_c = patterns_c.clone();
                let secrets_c = secrets_c.clone();
                Box::pin(async move {
                    functions::check_output::handle(iii_c, cfg_c, patterns_c, secrets_c, payload).await
                })
                    as std::pin::Pin<
                        Box<
                            dyn std::future::Future<
                                    Output = Result<serde_json::Value, iii_sdk::IIIError>,
                                > + Send,
                        >,
                    >
            },
        ));
    }

    {
        let cfg_c = cfg.clone();
        let patterns_c = compiled_patterns.clone();
        let secrets_c = compiled_secrets.clone();
        iii.register_function((
            RegisterFunctionMessage {
                id: "guardrails::classify".to_string(),
                description: Some(
                    "Lightweight risk classification without blocking or audit trail".to_string(),
                ),
                request_format: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "text": { "type": "string", "description": "Text to classify" }
                    },
                    "required": ["text"]
                })),
                response_format: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "risk": { "type": "string", "enum": ["none", "low", "medium", "high"] },
                        "categories": { "type": "array", "items": { "type": "string" } },
                        "pii_types": { "type": "array", "items": { "type": "string" } },
                        "details": { "type": "object" }
                    }
                })),
                metadata: None,
                invocation: None,
            },
            move |payload: serde_json::Value| {
                let cfg_c = cfg_c.clone();
                let patterns_c = patterns_c.clone();
                let secrets_c = secrets_c.clone();
                Box::pin(async move {
                    functions::classify::handle(cfg_c, patterns_c, secrets_c, payload).await
                })
                    as std::pin::Pin<
                        Box<
                            dyn std::future::Future<
                                    Output = Result<serde_json::Value, iii_sdk::IIIError>,
                                > + Send,
                        >,
                    >
            },
        ));
    }

    let _http_check_input = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "guardrails::check_input".to_string(),
        config: serde_json::json!({
            "api_path": "guardrails/check_input",
            "http_method": "POST"
        }),
    });

    let _http_check_output = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "guardrails::check_output".to_string(),
        config: serde_json::json!({
            "api_path": "guardrails/check_output",
            "http_method": "POST"
        }),
    });

    let _http_classify = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "guardrails::classify".to_string(),
        config: serde_json::json!({
            "api_path": "guardrails/classify",
            "http_method": "POST"
        }),
    });

    let _queue_check = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "subscribe".to_string(),
        function_id: "guardrails::check_input".to_string(),
        config: serde_json::json!({
            "topic": "guardrails.check"
        }),
    });

    tracing::info!("iii-guardrails registered 3 functions and 4 triggers, waiting for invocations");

    tokio::signal::ctrl_c().await?;

    tracing::info!("iii-guardrails shutting down");
    iii.shutdown_async().await;

    Ok(())
}
