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

    tracing::info!(
        "iii-llm-router registered {} functions and HTTP triggers, ready",
        manifest::FUNCTIONS.len()
    );

    tokio::signal::ctrl_c().await?;
    tracing::info!("iii-llm-router shutting down");
    iii.shutdown_async().await;
    Ok(())
}

fn register_functions(iii: &iii_sdk::III, cfg: Arc<config::RouterConfig>) {
    // iii_sdk::III::register_function_with returns an infallible FunctionRef,
    // so no error path to aggregate here — register_triggers below is the one
    // that can fail. If the SDK ever makes this fallible we'll want to
    // collect errors and abort startup.
    let desc_for = |id: &str| -> &'static str {
        manifest::FUNCTIONS
            .iter()
            .find(|(fid, _)| *fid == id)
            .map(|(_, d)| *d)
            .unwrap_or("")
    };

    macro_rules! reg {
        ($id:expr, $handler:expr) => {{
            let msg = RegisterFunctionMessage {
                id: $id.to_string(),
                description: Some(desc_for($id).to_string()),
                request_format: None,
                response_format: None,
                metadata: None,
                invocation: None,
            };
            iii.register_function_with(msg, $handler);
        }};
    }

    reg!("router::decide", functions::decide::build_handler(iii.clone(), cfg.clone()));
    reg!("router::policy_create", functions::policy::create_handler(iii.clone(), cfg.clone()));
    reg!("router::policy_update", functions::policy::update_handler(iii.clone(), cfg.clone()));
    reg!("router::policy_delete", functions::policy::delete_handler(iii.clone(), cfg.clone()));
    reg!("router::policy_list", functions::policy::list_handler(iii.clone(), cfg.clone()));
    reg!("router::policy_test", functions::policy::test_handler(iii.clone(), cfg.clone()));
    reg!("router::classify", functions::classify::classify_handler(iii.clone(), cfg.clone()));
    reg!("router::classifier_config", functions::classify::config_handler(iii.clone(), cfg.clone()));
    reg!("router::ab_create", functions::ab::create_handler(iii.clone(), cfg.clone()));
    reg!("router::ab_record", functions::ab::record_handler(iii.clone(), cfg.clone()));
    reg!("router::ab_report", functions::ab::report_handler(iii.clone(), cfg.clone()));
    reg!("router::ab_conclude", functions::ab::conclude_handler(iii.clone(), cfg.clone()));
    reg!("router::health_update", functions::health::update_handler(iii.clone(), cfg.clone()));
    reg!("router::health_list", functions::health::list_handler(iii.clone(), cfg.clone()));
    reg!("router::model_register", functions::model::register_handler(iii.clone(), cfg.clone()));
    reg!("router::model_unregister", functions::model::unregister_handler(iii.clone(), cfg.clone()));
    reg!("router::model_list", functions::model::list_handler(iii.clone(), cfg.clone()));
    reg!("router::stats", functions::stats::handler(iii.clone(), cfg.clone()));
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
