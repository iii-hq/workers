use anyhow::Result;
use clap::Parser;
use iii_sdk::{
    register_worker, InitOptions, OtelConfig, RegisterFunctionMessage, RegisterTriggerInput,
};
use std::sync::Arc;

mod config;
mod functions;
mod manifest;

#[derive(Parser, Debug)]
#[command(
    name = "iii-introspect",
    about = "III engine introspection worker — registry discovery, topology maps, and health checks"
)]
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

    let introspect_config = match config::load_config(&cli.config) {
        Ok(c) => {
            tracing::info!(
                cron = %c.cron_topology_refresh,
                cache_ttl = c.cache_ttl_seconds,
                "loaded config from {}",
                cli.config
            );
            c
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::IntrospectConfig::default()
        }
    };

    let config = Arc::new(introspect_config);

    tracing::info!(url = %cli.url, "connecting to III engine");

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );

    let iii_arc = Arc::new(iii.clone());

    let _fn_functions = iii.register_function_with(
        RegisterFunctionMessage {
            id: "introspect::functions".to_string(),
            description: Some("List all registered functions in the engine".to_string()),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {}
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "functions": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "description": { "type": "string" },
                                "request_format": { "type": "object" },
                                "response_format": { "type": "object" },
                                "metadata": { "type": "object" }
                            }
                        }
                    },
                    "count": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::functions::build_handler(iii_arc.clone()),
    );

    let _fn_workers = iii.register_function_with(
        RegisterFunctionMessage {
            id: "introspect::workers".to_string(),
            description: Some("List all connected workers".to_string()),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {}
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "workers": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "name": { "type": "string" },
                                "function_count": { "type": "integer" },
                                "functions": { "type": "array", "items": { "type": "string" } },
                                "status": { "type": "string" },
                                "runtime": { "type": "string" },
                                "version": { "type": "string" },
                                "connected_at_ms": { "type": "integer" },
                                "active_invocations": { "type": "integer" }
                            }
                        }
                    },
                    "count": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::workers::build_handler(iii_arc.clone()),
    );

    let _fn_triggers = iii.register_function_with(
        RegisterFunctionMessage {
            id: "introspect::triggers".to_string(),
            description: Some("List all registered triggers".to_string()),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {}
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "triggers": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "trigger_type": { "type": "string" },
                                "function_id": { "type": "string" },
                                "config": { "type": "object" },
                                "metadata": { "type": "object" }
                            }
                        }
                    },
                    "count": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::triggers::build_handler(iii_arc.clone()),
    );

    let _fn_topology = iii.register_function_with(
        RegisterFunctionMessage {
            id: "introspect::topology".to_string(),
            description: Some(
                "Full system topology combining functions, workers, and triggers".to_string(),
            ),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {}
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "functions": { "type": "array" },
                    "workers": { "type": "array" },
                    "triggers": { "type": "array" },
                    "stats": {
                        "type": "object",
                        "properties": {
                            "total_functions": { "type": "integer" },
                            "total_workers": { "type": "integer" },
                            "total_triggers": { "type": "integer" },
                            "functions_per_worker": { "type": "array" }
                        }
                    },
                    "cached_at": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::topology::build_handler(iii_arc.clone(), config.clone()),
    );

    let _fn_diagram = iii.register_function_with(
        RegisterFunctionMessage {
            id: "introspect::diagram".to_string(),
            description: Some("Generate mermaid diagram of system topology".to_string()),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {}
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "format": { "type": "string" },
                    "content": { "type": "string" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::diagram::build_handler(iii_arc.clone()),
    );

    let _fn_health = iii.register_function_with(
        RegisterFunctionMessage {
            id: "introspect::health".to_string(),
            description: Some(
                "System health check — orphaned functions, empty workers, duplicate IDs"
                    .to_string(),
            ),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {}
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "healthy": { "type": "boolean" },
                    "checks": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "status": { "type": "string" },
                                "detail": { "type": "string" }
                            }
                        }
                    },
                    "timestamp": { "type": "string" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::health::build_handler(iii_arc.clone()),
    );

    let _fn_trace = iii.register_function_with(
        RegisterFunctionMessage {
            id: "introspect::trace_workflow".to_string(),
            description: Some("Trace a specific function or trigger through its dependency chain".to_string()),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "function_id": { "type": "string", "description": "Function ID to trace" },
                    "trigger_id": { "type": "string", "description": "Trigger ID to trace (alternative to function_id)" }
                }
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "function_id": { "type": "string" },
                    "chain": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "step": { "type": "integer" },
                                "function_id": { "type": "string" },
                                "worker": { "type": "string" },
                                "description": { "type": "string" },
                                "triggers": { "type": "array" },
                                "inputs": { "type": "object" },
                                "outputs": { "type": "object" }
                            }
                        }
                    },
                    "diagram": { "type": "string" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::trace::build_handler(iii_arc.clone()),
    );

    let _fn_explain = iii.register_function_with(
        RegisterFunctionMessage {
            id: "introspect::explain".to_string(),
            description: Some("Explain what a function or worker does in business terms".to_string()),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "function_id": { "type": "string", "description": "Function ID to explain" },
                    "worker_name": { "type": "string", "description": "Worker name to explain (alternative to function_id)" }
                }
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "explanation": { "type": "string" },
                    "function_id": { "type": "string" },
                    "worker": { "type": "string" },
                    "triggers": { "type": "array" },
                    "inputs": { "type": "object" },
                    "outputs": { "type": "object" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::explain::build_handler(iii_arc.clone()),
    );

    let _http_trace = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "introspect::trace_workflow".to_string(),
        config: serde_json::json!({
            "api_path": "introspect/trace",
            "http_method": "POST"
        }),
        metadata: None,
    });

    let _http_explain = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "introspect::explain".to_string(),
        config: serde_json::json!({
            "api_path": "introspect/explain",
            "http_method": "POST"
        }),
        metadata: None,
    });

    let _fn_topology_refresh = iii.register_function_with(
        RegisterFunctionMessage {
            id: "introspect::topology_refresh".to_string(),
            description: Some("Refresh topology cache (called by cron trigger)".to_string()),
            request_format: None,
            response_format: None,
            metadata: None,
            invocation: None,
        },
        functions::topology::build_refresh_handler(iii_arc.clone()),
    );

    let _cron_trigger = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "cron".to_string(),
        function_id: "introspect::topology_refresh".to_string(),
        config: serde_json::json!({
            "cron": config.cron_topology_refresh,
        }),
        metadata: None,
    });

    let _http_functions = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "introspect::functions".to_string(),
        config: serde_json::json!({
            "api_path": "introspect/functions",
            "http_method": "GET"
        }),
        metadata: None,
    });

    let _http_workers = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "introspect::workers".to_string(),
        config: serde_json::json!({
            "api_path": "introspect/workers",
            "http_method": "GET"
        }),
        metadata: None,
    });

    let _http_triggers = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "introspect::triggers".to_string(),
        config: serde_json::json!({
            "api_path": "introspect/triggers",
            "http_method": "GET"
        }),
        metadata: None,
    });

    let _http_topology = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "introspect::topology".to_string(),
        config: serde_json::json!({
            "api_path": "introspect/topology",
            "http_method": "GET"
        }),
        metadata: None,
    });

    let _http_diagram = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "introspect::diagram".to_string(),
        config: serde_json::json!({
            "api_path": "introspect/diagram",
            "http_method": "GET"
        }),
        metadata: None,
    });

    let _http_health = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "introspect::health".to_string(),
        config: serde_json::json!({
            "api_path": "introspect/health",
            "http_method": "GET"
        }),
        metadata: None,
    });

    tracing::info!("iii-introspect registered 9 functions and 9 triggers, waiting for invocations");

    tokio::signal::ctrl_c().await?;

    tracing::info!("iii-introspect shutting down");
    iii.shutdown_async().await;

    Ok(())
}
