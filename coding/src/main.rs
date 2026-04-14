use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, RegisterFunctionMessage, RegisterTriggerInput};
use std::sync::Arc;

mod config;
mod functions;
mod manifest;
mod state;
mod templates;

#[derive(Parser, Debug)]
#[command(name = "iii-coding", about = "III engine coding worker — scaffold workers, generate functions and triggers, execute code, test, and deploy")]
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

    let coding_config = match config::load_config(&cli.config) {
        Ok(c) => {
            tracing::info!(
                workspace = %c.workspace_dir,
                languages = ?c.supported_languages,
                timeout_ms = c.execute_timeout_ms,
                "loaded config from {}",
                cli.config
            );
            c
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::CodingConfig::default()
        }
    };

    let config = Arc::new(coding_config);

    tracing::info!(url = %cli.url, "connecting to III engine");

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );

    let iii_arc = Arc::new(iii.clone());

    let _fn_scaffold = iii.register_function_with(
        RegisterFunctionMessage {
            id: "coding::scaffold".to_string(),
            description: Some("Scaffold a complete iii worker project from a definition".to_string()),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Worker name (e.g. my-worker)" },
                    "language": { "type": "string", "enum": ["rust", "typescript", "python"] },
                    "functions": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "description": { "type": "string" },
                                "request_format": { "type": "object" },
                                "response_format": { "type": "object" }
                            },
                            "required": ["id", "description"]
                        }
                    },
                    "triggers": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "trigger_type": { "type": "string", "enum": ["http", "cron", "durable::subscriber"] },
                                "function_id": { "type": "string" },
                                "config": { "type": "object" }
                            },
                            "required": ["trigger_type", "function_id", "config"]
                        }
                    }
                },
                "required": ["name", "language"]
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "worker_id": { "type": "string" },
                    "files": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": { "type": "string" },
                                "content": { "type": "string" },
                                "language": { "type": "string" }
                            }
                        }
                    },
                    "function_count": { "type": "integer" },
                    "trigger_count": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::scaffold::build_handler(iii_arc.clone(), config.clone()),
    );

    let _fn_generate_function = iii.register_function_with(
        RegisterFunctionMessage {
            id: "coding::generate_function".to_string(),
            description: Some("Generate a single function handler file".to_string()),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "worker_id": { "type": "string", "description": "Optional: add to existing scaffolded worker" },
                    "language": { "type": "string", "enum": ["rust", "typescript", "python"] },
                    "id": { "type": "string", "description": "Function ID (e.g. myworker::greet)" },
                    "description": { "type": "string" },
                    "request_format": { "type": "object" },
                    "response_format": { "type": "object" }
                },
                "required": ["language", "id"]
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "function_id": { "type": "string" },
                    "file_path": { "type": "string" },
                    "content": { "type": "string" },
                    "language": { "type": "string" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::generate_function::build_handler(iii_arc.clone()),
    );

    let _fn_generate_trigger = iii.register_function_with(
        RegisterFunctionMessage {
            id: "coding::generate_trigger".to_string(),
            description: Some("Generate trigger registration code for a function".to_string()),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "function_id": { "type": "string" },
                    "trigger_type": { "type": "string", "enum": ["http", "cron", "durable::subscriber"] },
                    "config": { "type": "object" },
                    "language": { "type": "string", "enum": ["rust", "typescript", "python"], "default": "rust" }
                },
                "required": ["function_id", "trigger_type", "config"]
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "trigger_type": { "type": "string" },
                    "function_id": { "type": "string" },
                    "registration_code": { "type": "string" },
                    "config": { "type": "object" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::generate_trigger::build_handler(iii_arc.clone()),
    );

    let _fn_execute = iii.register_function_with(
        RegisterFunctionMessage {
            id: "coding::execute".to_string(),
            description: Some("Execute code in a sandboxed subprocess".to_string()),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "code": { "type": "string" },
                    "language": { "type": "string", "enum": ["rust", "typescript", "python"] },
                    "input": { "type": "object" },
                    "timeout_ms": { "type": "integer" }
                },
                "required": ["code", "language"]
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "success": { "type": "boolean" },
                    "stdout": { "type": "string" },
                    "stderr": { "type": "string" },
                    "exit_code": { "type": "integer" },
                    "duration_ms": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::execute::build_handler(iii_arc.clone(), config.clone()),
    );

    let _fn_test = iii.register_function_with(
        RegisterFunctionMessage {
            id: "coding::test".to_string(),
            description: Some("Run tests for a scaffolded worker or inline code".to_string()),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "worker_id": { "type": "string", "description": "Test a scaffolded worker" },
                    "code": { "type": "string", "description": "Inline code to test" },
                    "language": { "type": "string", "enum": ["rust", "typescript", "python"] },
                    "test_code": { "type": "string", "description": "Test code to run against inline code" }
                }
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "passed": { "type": "boolean" },
                    "total": { "type": "integer" },
                    "passed_count": { "type": "integer" },
                    "failed_count": { "type": "integer" },
                    "output": { "type": "string" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::test::build_handler(iii_arc.clone(), config.clone()),
    );

    let _fn_deploy = iii.register_function_with(
        RegisterFunctionMessage {
            id: "coding::deploy".to_string(),
            description: Some("Deploy a scaffolded worker (returns files and instructions)".to_string()),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "worker_id": { "type": "string" }
                },
                "required": ["worker_id"]
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "deployed": { "type": "boolean" },
                    "worker_id": { "type": "string" },
                    "deployment_id": { "type": "string" },
                    "files": { "type": "array" },
                    "instructions": { "type": "string" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::deploy::build_handler(iii_arc.clone()),
    );

    let _http_scaffold = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "coding::scaffold".to_string(),
        config: serde_json::json!({
            "api_path": "coding/scaffold",
            "http_method": "POST"
        }),
    });

    let _http_generate_function = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "coding::generate_function".to_string(),
        config: serde_json::json!({
            "api_path": "coding/generate-function",
            "http_method": "POST"
        }),
    });

    let _http_generate_trigger = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "coding::generate_trigger".to_string(),
        config: serde_json::json!({
            "api_path": "coding/generate-trigger",
            "http_method": "POST"
        }),
    });

    let _http_execute = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "coding::execute".to_string(),
        config: serde_json::json!({
            "api_path": "coding/execute",
            "http_method": "POST"
        }),
    });

    let _http_test = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "coding::test".to_string(),
        config: serde_json::json!({
            "api_path": "coding/test",
            "http_method": "POST"
        }),
    });

    let _http_deploy = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "coding::deploy".to_string(),
        config: serde_json::json!({
            "api_path": "coding/deploy",
            "http_method": "POST"
        }),
    });

    tracing::info!("iii-coding registered 6 functions and 6 triggers, waiting for invocations");

    tokio::signal::ctrl_c().await?;

    tracing::info!("iii-coding shutting down");
    iii.shutdown_async().await;

    Ok(())
}
