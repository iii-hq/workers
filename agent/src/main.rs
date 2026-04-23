use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::{
    register_worker, InitOptions, OtelConfig, RegisterFunctionMessage, RegisterTriggerInput,
};
use serde_json::json;

mod config;
mod discovery;
mod functions;
mod llm;
mod manifest;
mod state;

#[derive(Parser, Debug)]
#[command(name = "iii-agent", about = "III engine AI agent — chat orchestrator")]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,

    #[arg(long)]
    manifest: bool,

    #[arg(long, env = "ANTHROPIC_API_KEY")]
    api_key: Option<String>,
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

    let agent_config = match config::load_config(&cli.config) {
        Ok(c) => {
            tracing::info!(
                model = %c.anthropic_model,
                max_iterations = c.max_iterations,
                "loaded config from {}",
                cli.config
            );
            c
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::AgentConfig::default()
        }
    };

    let config = Arc::new(agent_config);

    let llm_client = if let Some(ref key) = cli.api_key {
        llm::LlmClient::new(key.clone())
    } else {
        llm::LlmClient::from_env()?
    };
    let llm = Arc::new(llm_client);

    tracing::info!(url = %cli.url, "connecting to III engine");

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );

    let chat_handler = functions::chat::build_handler(iii.clone(), config.clone(), llm.clone());
    let _chat_fn = iii.register_function_with(
        RegisterFunctionMessage {
            id: "agent::chat".to_string(),
            description: Some(
                "Send a message to the AI agent and get a structured response".to_string(),
            ),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID for conversation continuity"
                    },
                    "message": {
                        "type": "string",
                        "description": "User message to the agent"
                    }
                },
                "required": ["message"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "elements": {
                        "type": "array",
                        "description": "JSON-UI elements for console rendering"
                    },
                    "usage": {
                        "type": "object",
                        "properties": {
                            "input_tokens": { "type": "integer" },
                            "output_tokens": { "type": "integer" }
                        }
                    }
                }
            })),
            metadata: None,
            invocation: None,
        },
        chat_handler,
    );

    let chat_stream_handler =
        functions::chat_stream::build_handler(iii.clone(), config.clone(), llm.clone());
    let _chat_stream_fn = iii.register_function_with(
        RegisterFunctionMessage {
            id: "agent::chat_stream".to_string(),
            description: Some(
                "Send a message to the AI agent with streaming response via iii Streams"
                    .to_string(),
            ),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID for conversation continuity"
                    },
                    "message": {
                        "type": "string",
                        "description": "User message to the agent"
                    }
                },
                "required": ["message"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "stream_group": { "type": "string" },
                    "session_id": { "type": "string" },
                    "iterations": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        chat_stream_handler,
    );

    let discover_handler = functions::discover::build_handler(iii.clone());
    let _discover_fn = iii.register_function_with(
        RegisterFunctionMessage {
            id: "agent::discover".to_string(),
            description: Some(
                "List all available functions that the agent can orchestrate".to_string(),
            ),
            request_format: Some(json!({
                "type": "object",
                "properties": {}
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "functions": { "type": "array" },
                    "count": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        discover_handler,
    );

    let plan_handler = functions::plan::build_handler(iii.clone(), config.clone(), llm.clone());
    let _plan_fn = iii.register_function_with(
        RegisterFunctionMessage {
            id: "agent::plan".to_string(),
            description: Some(
                "Generate an execution plan DAG from a query without executing".to_string(),
            ),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The query to generate a plan for"
                    }
                },
                "required": ["query"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "steps": { "type": "array" },
                    "summary": { "type": "string" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        plan_handler,
    );

    let session_create_handler = functions::session::build_create_handler(iii.clone());
    let _session_create_fn = iii.register_function_with(
        RegisterFunctionMessage {
            id: "agent::session_create".to_string(),
            description: Some("Create a new chat session".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {}
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "created_at": { "type": "string" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        session_create_handler,
    );

    let session_history_handler = functions::session::build_history_handler(iii.clone());
    let _session_history_fn = iii.register_function_with(
        RegisterFunctionMessage {
            id: "agent::session_history".to_string(),
            description: Some("Retrieve conversation history for a session".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID to retrieve history for"
                    }
                },
                "required": ["session_id"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "history": {}
                }
            })),
            metadata: None,
            invocation: None,
        },
        session_history_handler,
    );

    let _http_chat = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "agent::chat".to_string(),
        config: json!({
            "api_path": "agent/chat",
            "http_method": "POST"
        }),
        metadata: None,
    });

    let _http_discover = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "agent::discover".to_string(),
        config: json!({
            "api_path": "agent/discover",
            "http_method": "GET"
        }),
        metadata: None,
    });

    let _http_chat_stream = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "agent::chat_stream".to_string(),
        config: json!({
            "api_path": "agent/chat/stream",
            "http_method": "POST"
        }),
        metadata: None,
    });

    let _http_plan = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "agent::plan".to_string(),
        config: json!({
            "api_path": "agent/plan",
            "http_method": "POST"
        }),
        metadata: None,
    });

    let _http_session_create = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "agent::session_create".to_string(),
        config: json!({
            "api_path": "agent/session",
            "http_method": "POST"
        }),
        metadata: None,
    });

    let _http_session_history = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "agent::session_history".to_string(),
        config: json!({
            "api_path": "agent/session/history",
            "http_method": "POST"
        }),
        metadata: None,
    });

    let iii_for_refresh = iii.clone();
    let _functions_guard = iii.on_functions_available(move |functions| {
        let tools = discovery::functions_to_tools(&functions);
        let tools_json = serde_json::to_value(&tools).unwrap_or(json!([]));

        let iii_inner = iii_for_refresh.clone();
        tokio::spawn(async move {
            // Log the outcome honestly — the earlier `let _ = ...` swallowed
            // the Err and always emitted "tool cache refreshed", so an engine
            // outage looked identical to a successful write in the logs.
            match state::state_set(&iii_inner, "agent:tools", "cached", &tools_json).await {
                Ok(_) => tracing::info!(
                    count = tools_json.as_array().map(|a| a.len()).unwrap_or(0),
                    "tool cache refreshed"
                ),
                Err(e) => tracing::error!(
                    error = %e,
                    count = tools_json.as_array().map(|a| a.len()).unwrap_or(0),
                    "tool cache refresh failed"
                ),
            }
        });
    });

    let session_cleanup_handler =
        functions::session::build_cleanup_handler(iii.clone(), config.session_ttl_hours);
    let _cleanup_fn = iii.register_function_with(
        RegisterFunctionMessage {
            id: "agent::session_cleanup".to_string(),
            description: Some("Clean up expired sessions".to_string()),
            request_format: Some(json!({"type": "object", "properties": {}})),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "cleaned_sessions": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        session_cleanup_handler,
    );

    let _cron_cleanup = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "cron".to_string(),
        function_id: "agent::session_cleanup".to_string(),
        config: json!({
            "cron_expression": config.cron_session_cleanup
        }),
        metadata: None,
    });

    tracing::info!(
        "iii-agent registered 7 functions, 6 HTTP triggers, 1 cron trigger, 1 subscribe trigger"
    );

    tokio::signal::ctrl_c().await?;

    tracing::info!("iii-agent shutting down");
    iii.shutdown_async().await;

    Ok(())
}
