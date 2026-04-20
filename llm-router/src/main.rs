use anyhow::Result;
use clap::Parser;
use iii_sdk::{
    register_worker, InitOptions, OtelConfig, RegisterFunctionMessage, RegisterTriggerInput,
};
use serde_json::json;
use std::sync::Arc;

mod config;
mod functions;
mod manifest;
mod router;
mod state;
mod types;

#[derive(Parser, Debug)]
#[command(name = "iii-llm-router", about = "Policy-based LLM routing brain for iii")]
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
        let m = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m).unwrap());
        return Ok(());
    }

    let router_config = match config::load_config(&cli.config) {
        Ok(c) => {
            tracing::info!(
                scope = %c.state_scope,
                classifier = %c.classifier_default_id,
                stats_days = c.stats_default_days,
                "loaded config"
            );
            c
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::RouterConfig::default()
        }
    };
    let cfg = Arc::new(router_config);

    tracing::info!(url = %cli.url, "connecting to III engine");
    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );

    register_functions(&iii, cfg.clone());
    register_triggers(&iii);

    tracing::info!("iii-llm-router registered 18 functions and 18 HTTP triggers, ready");

    tokio::signal::ctrl_c().await?;
    tracing::info!("iii-llm-router shutting down");
    iii.shutdown_async().await;
    Ok(())
}

fn register_functions(iii: &iii_sdk::III, cfg: Arc<config::RouterConfig>) {
    let mk = |id: &str, desc: &str| RegisterFunctionMessage {
        id: id.to_string(),
        description: Some(desc.to_string()),
        request_format: None,
        response_format: None,
        metadata: None,
        invocation: None,
    };

    let _ = iii.register_function_with(
        mk("router::decide", "Pick a model for a request (hot path)"),
        functions::decide::build_handler(iii.clone(), cfg.clone()),
    );
    let _ = iii.register_function_with(
        mk("router::policy_create", "Register a routing policy"),
        functions::policy::create_handler(iii.clone(), cfg.clone()),
    );
    let _ = iii.register_function_with(
        mk("router::policy_update", "Patch a policy"),
        functions::policy::update_handler(iii.clone(), cfg.clone()),
    );
    let _ = iii.register_function_with(
        mk("router::policy_delete", "Remove a policy"),
        functions::policy::delete_handler(iii.clone(), cfg.clone()),
    );
    let _ = iii.register_function_with(
        mk("router::policy_list", "List all policies"),
        functions::policy::list_handler(iii.clone(), cfg.clone()),
    );
    let _ = iii.register_function_with(
        mk("router::policy_test", "Dry-run router::decide without logging"),
        functions::policy::test_handler(iii.clone(), cfg.clone()),
    );

    let _ = iii.register_function_with(
        mk("router::classify", "Run prompt-complexity classifier"),
        functions::classify::classify_handler(iii.clone(), cfg.clone()),
    );
    let _ = iii.register_function_with(
        mk(
            "router::classifier_config",
            "Configure the category→model mapping",
        ),
        functions::classify::config_handler(iii.clone(), cfg.clone()),
    );

    let _ = iii.register_function_with(
        mk("router::ab_create", "Create an A/B test"),
        functions::ab::create_handler(iii.clone(), cfg.clone()),
    );
    let _ = iii.register_function_with(
        mk(
            "router::ab_record",
            "Record a quality/latency/cost outcome",
        ),
        functions::ab::record_handler(iii.clone(), cfg.clone()),
    );
    let _ = iii.register_function_with(
        mk("router::ab_report", "Aggregate A/B samples"),
        functions::ab::report_handler(iii.clone(), cfg.clone()),
    );
    let _ = iii.register_function_with(
        mk("router::ab_conclude", "Mark an A/B test concluded"),
        functions::ab::conclude_handler(iii.clone(), cfg.clone()),
    );

    let _ = iii.register_function_with(
        mk("router::health_update", "Update per-model health"),
        functions::health::update_handler(iii.clone(), cfg.clone()),
    );
    let _ = iii.register_function_with(
        mk("router::health_list", "List health for all models"),
        functions::health::list_handler(iii.clone(), cfg.clone()),
    );

    let _ = iii.register_function_with(
        mk(
            "router::model_register",
            "Register a model with quality and pricing",
        ),
        functions::model::register_handler(iii.clone(), cfg.clone()),
    );
    let _ = iii.register_function_with(
        mk("router::model_unregister", "Remove a model registration"),
        functions::model::unregister_handler(iii.clone(), cfg.clone()),
    );
    let _ = iii.register_function_with(
        mk("router::model_list", "List registered models"),
        functions::model::list_handler(iii.clone(), cfg.clone()),
    );

    let _ = iii.register_function_with(
        mk("router::stats", "Usage stats over a window"),
        functions::stats::handler(iii.clone(), cfg.clone()),
    );
}

fn register_triggers(iii: &iii_sdk::III) {
    for (fn_id, path, method) in [
        ("router::decide", "router/decide", "POST"),
        ("router::policy_create", "router/policy/create", "POST"),
        ("router::policy_update", "router/policy/update", "POST"),
        ("router::policy_delete", "router/policy/delete", "POST"),
        ("router::policy_list", "router/policy/list", "GET"),
        ("router::policy_test", "router/policy/test", "POST"),
        ("router::classify", "router/classify", "POST"),
        ("router::classifier_config", "router/classifier", "POST"),
        ("router::ab_create", "router/ab/create", "POST"),
        ("router::ab_record", "router/ab/record", "POST"),
        ("router::ab_report", "router/ab/report", "POST"),
        ("router::ab_conclude", "router/ab/conclude", "POST"),
        ("router::health_update", "router/health/update", "POST"),
        ("router::health_list", "router/health/list", "GET"),
        ("router::model_register", "router/model/register", "POST"),
        ("router::model_unregister", "router/model/unregister", "POST"),
        ("router::model_list", "router/model/list", "GET"),
        ("router::stats", "router/stats", "GET"),
    ] {
        if let Err(e) = iii.register_trigger(RegisterTriggerInput {
            trigger_type: "http".to_string(),
            function_id: fn_id.to_string(),
            config: json!({ "api_path": path, "http_method": method }),
            metadata: None,
        }) {
            tracing::warn!(error = %e, "failed to register trigger for {}", fn_id);
        }
    }
}
