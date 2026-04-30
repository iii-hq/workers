use anyhow::{Context, Result};
use clap::Parser;
use iii_database::config::WorkerConfig;
use iii_database::handle::HandleRegistry;
use iii_database::handlers::{execute, prepare, query, run_statement, transaction, AppState};
use iii_database::pool;
use iii_database::triggers::handler::{QueryPollTrigger, RowChangeTrigger};
use iii_sdk::{
    register_worker, InitOptions, OtelConfig, RegisterFunctionMessage, RegisterTriggerType,
};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(
    name = "iii-database",
    about = "iii-database worker (PostgreSQL, MySQL, SQLite)"
)]
struct Cli {
    /// Path to config.yaml file
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    /// WebSocket URL of the iii engine
    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,
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
    tracing::info!(name = iii_database::worker_name(), config = %cli.config, url = %cli.url, "starting");

    let cfg = WorkerConfig::from_file(&cli.config)
        .map_err(|e| anyhow::anyhow!(e))
        .with_context(|| format!("loading config from {}", cli.config))?;

    let mut pools = HashMap::new();
    for (name, db) in &cfg.databases {
        let p = pool::build(name, db)
            .await
            .map_err(|e| anyhow::anyhow!(serde_json::to_string(&e).unwrap_or_default()))
            .with_context(|| format!("building pool for db `{name}`"))?;
        tracing::info!(db = %name, driver = ?p.driver(), "pool ready");
        pools.insert(name.clone(), p);
    }

    let handles = Arc::new(HandleRegistry::new());
    let state = AppState {
        pools: Arc::new(pools),
        handles: handles.clone(),
    };

    let _evictor = handles.spawn_evictor();

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );

    register_function(
        &iii,
        &state,
        "query",
        "Run a read-only SQL query and return the result rows.",
        |st, p| Box::pin(query::handle(st, p)),
    );
    register_function(
        &iii,
        &state,
        "execute",
        "Run a write statement (INSERT/UPDATE/DELETE/DDL).",
        |st, p| Box::pin(execute::handle(st, p)),
    );
    register_function(
        &iii,
        &state,
        "prepareStatement",
        "Prepare a parameterized statement once.",
        |st, p| Box::pin(prepare::handle(st, p)),
    );
    register_function(
        &iii,
        &state,
        "runStatement",
        "Run a previously-prepared handle.",
        |st, p| Box::pin(run_statement::handle(st, p)),
    );
    register_function(
        &iii,
        &state,
        "transaction",
        "Run a sequence of statements atomically.",
        |st, p| Box::pin(transaction::handle(st, p)),
    );

    let _query_poll = iii.register_trigger_type(RegisterTriggerType::new(
        "iii-database::query-poll",
        "Polls a SQL query at a fixed interval and dispatches new rows since the last cursor.",
        QueryPollTrigger::new(state.clone(), iii.clone()),
    ));
    let _row_change = iii.register_trigger_type(RegisterTriggerType::new(
        "iii-database::row-change",
        "Postgres logical replication. Stubbed in v1.0 pending tokio-postgres replication API.",
        RowChangeTrigger,
    ));

    tracing::info!(
        "iii-database worker registered 5 functions and 2 trigger types, waiting for invocations"
    );
    tokio::signal::ctrl_c().await?;
    tracing::info!("iii-database worker shutting down");
    iii.shutdown_async().await;
    Ok(())
}

type HandlerFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<serde_json::Value, String>> + Send + 'a>,
>;

fn register_function<F>(
    iii: &iii_sdk::III,
    state: &AppState,
    name: &str,
    description: &str,
    handler: F,
) where
    F: for<'a> Fn(&'a AppState, serde_json::Value) -> HandlerFuture<'a>
        + Send
        + Sync
        + Copy
        + 'static,
{
    let id = format!("iii-database::{name}");
    let state = state.clone();
    let id_for_msg = id.clone();
    let _ = iii.register_function_with(
        RegisterFunctionMessage {
            id: id.clone(),
            description: Some(description.to_string()),
            request_format: None,
            response_format: None,
            metadata: None,
            invocation: None,
        },
        move |payload: serde_json::Value| {
            let state = state.clone();
            let id_for_msg = id_for_msg.clone();
            Box::pin(async move {
                handler(&state, payload).await.map_err(|s| {
                    tracing::warn!(function = %id_for_msg, body = %s, "handler returned error");
                    iii_sdk::IIIError::Handler(s)
                })
            })
        },
    );
}
