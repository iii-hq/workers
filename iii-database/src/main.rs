use anyhow::{Context, Result};
use clap::Parser;
use iii_database::config::WorkerConfig;
use iii_database::handle::HandleRegistry;
use iii_database::handlers::{
    execute::{self, ExecuteReq},
    prepare::{self, PrepareReq},
    query::{self, QueryReq},
    run_statement::{self, RunReq},
    transaction::{self, TxReq},
    AppState,
};
use iii_database::pool;
use iii_database::triggers::handler::{QueryPollTrigger, RowChangeTrigger};
use iii_sdk::{register_worker, InitOptions, OtelConfig, RegisterFunction, RegisterTriggerType};
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

    {
        let st = state.clone();
        iii.register_function(
            RegisterFunction::new_async("iii-database::query", move |req: QueryReq| {
                let st = st.clone();
                async move { query::handle(&st, req).await }
            })
            .description("Run a read-only SQL query and return the result rows."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            RegisterFunction::new_async("iii-database::execute", move |req: ExecuteReq| {
                let st = st.clone();
                async move { execute::handle(&st, req).await }
            })
            .description("Run a write statement (INSERT/UPDATE/DELETE/DDL)."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            RegisterFunction::new_async(
                "iii-database::prepareStatement",
                move |req: PrepareReq| {
                    let st = st.clone();
                    async move { prepare::handle(&st, req).await }
                },
            )
            .description("Prepare a parameterized statement once."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            RegisterFunction::new_async("iii-database::runStatement", move |req: RunReq| {
                let st = st.clone();
                async move { run_statement::handle(&st, req).await }
            })
            .description("Run a previously-prepared handle."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            RegisterFunction::new_async("iii-database::transaction", move |req: TxReq| {
                let st = st.clone();
                async move { transaction::handle(&st, req).await }
            })
            .description("Run a sequence of statements atomically."),
        );
    }

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
