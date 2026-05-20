use anyhow::{Context, Result};
use clap::Parser;
use iii_database::config::WorkerConfig;
use iii_database::handle::HandleRegistry;
use iii_database::handlers::{
    begin_transaction::{self, BeginTxReq},
    commit_transaction::{self, CommitTxReq},
    execute::{self, ExecuteReq},
    prepare::{self, PrepareReq},
    query::{self, QueryReq},
    rollback_transaction::{self, RollbackTxReq},
    run_statement::{self, RunReq},
    transaction::{self, TxReq},
    transaction_execute::{self, TxExecuteReq},
    transaction_query::{self, TxQueryReq},
    AppState,
};
use iii_database::pool;
use iii_database::transaction::TxRegistry;
use iii_database::triggers::handler::RowChangeTrigger;
use iii_sdk::{
    register_worker, InitOptions, Logger, OtelConfig, RegisterFunction, RegisterTriggerType,
};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(
    name = "database",
    about = "database worker (PostgreSQL, MySQL, SQLite)"
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
    tracing::info!(
        name = iii_database::worker_name(),
        config = %cli.config,
        url = %redact_url(&cli.url),
        "starting"
    );

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
    let transactions = TxRegistry::new();
    let log = Logger::new();
    let state = AppState {
        pools: Arc::new(pools),
        handles: handles.clone(),
        transactions: transactions.clone(),
        log: log.clone(),
    };

    let _evictor = handles.spawn_evictor();
    let _tx_watcher = transactions.spawn_timeout_watcher(log.clone());

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
            RegisterFunction::new_async("database::query", move |req: QueryReq| {
                let st = st.clone();
                async move { query::handle(&st, req).await }
            })
            .description("Run a read-only SQL query and return the result rows."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            RegisterFunction::new_async("database::execute", move |req: ExecuteReq| {
                let st = st.clone();
                async move { execute::handle(&st, req).await }
            })
            .description("Run a write statement (INSERT/UPDATE/DELETE/DDL)."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            RegisterFunction::new_async("database::prepareStatement", move |req: PrepareReq| {
                let st = st.clone();
                async move { prepare::handle(&st, req).await }
            })
            .description("Prepare a parameterized statement once."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            RegisterFunction::new_async("database::runStatement", move |req: RunReq| {
                let st = st.clone();
                async move { run_statement::handle(&st, req).await }
            })
            .description("Run a previously-prepared handle."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            RegisterFunction::new_async("database::transaction", move |req: TxReq| {
                let st = st.clone();
                async move { transaction::handle(&st, req).await }
            })
            .description("Run a sequence of statements atomically."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            RegisterFunction::new_async("database::beginTransaction", move |req: BeginTxReq| {
                let st = st.clone();
                async move { begin_transaction::handle(&st, req).await }
            })
            .description(
                "Open an interactive transaction; returns a handle to use with \
                 transactionQuery/transactionExecute/commitTransaction/rollbackTransaction.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            RegisterFunction::new_async("database::transactionQuery", move |req: TxQueryReq| {
                let st = st.clone();
                async move { transaction_query::handle(&st, req).await }
            })
            .description("Run a read-only SQL query inside an interactive transaction."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            RegisterFunction::new_async(
                "database::transactionExecute",
                move |req: TxExecuteReq| {
                    let st = st.clone();
                    async move { transaction_execute::handle(&st, req).await }
                },
            )
            .description(
                "Run a write statement inside an interactive transaction. \
                 BEGIN/COMMIT/ROLLBACK are rejected; use commit/rollbackTransaction.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            RegisterFunction::new_async("database::commitTransaction", move |req: CommitTxReq| {
                let st = st.clone();
                async move { commit_transaction::handle(&st, req).await }
            })
            .description("Commit and finalize an interactive transaction."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            RegisterFunction::new_async(
                "database::rollbackTransaction",
                move |req: RollbackTxReq| {
                    let st = st.clone();
                    async move { rollback_transaction::handle(&st, req).await }
                },
            )
            .description("Rollback and finalize an interactive transaction."),
        );
    }

    let _row_change = iii.register_trigger_type(RegisterTriggerType::new(
        "database::row-change",
        "Postgres logical replication. Stubbed in v1.0 pending tokio-postgres replication API.",
        RowChangeTrigger,
    ));

    tracing::info!(
        "database worker registered 10 functions and 1 trigger type, waiting for invocations"
    );
    wait_for_shutdown_signal().await?;
    tracing::info!("database worker shutting down");
    iii.shutdown_async().await;
    Ok(())
}

/// Strip userinfo (username:password) from a URL before logging it. The
/// engine websocket URL is operator-controlled and can carry credentials in
/// `wss://user:secret@host` form; `tracing::info!(url = %cli.url, ...)`
/// would otherwise emit them. Falls back to the original string on parse
/// failure (no logging-time panics).
fn redact_url(s: &str) -> String {
    match url::Url::parse(s) {
        Ok(mut u) => {
            let _ = u.set_username("");
            let _ = u.set_password(None);
            u.to_string()
        }
        Err(_) => s.to_string(),
    }
}

/// Wait for SIGINT or, on Unix, SIGTERM. `tokio::signal::ctrl_c()` alone
/// only catches SIGINT, leaving Docker `docker stop` / k8s `kubectl delete`
/// (which send SIGTERM) to bypass `iii.shutdown_async()` entirely — the
/// engine connection would dangle until the process was killed.
async fn wait_for_shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate())?;
        tokio::select! {
            r = tokio::signal::ctrl_c() => r,
            _ = sigterm.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

#[cfg(test)]
mod tests {
    use super::redact_url;

    /// Regression: operator-controlled engine URLs may carry credentials in
    /// `wss://user:secret@host` form. `tracing::info!(url = %cli.url, ...)`
    /// previously emitted them verbatim. The redactor strips userinfo and
    /// preserves the rest so logs remain useful for diagnostics.
    #[test]
    fn redact_url_strips_userinfo_only() {
        // Plain URL without credentials → unchanged (modulo url crate's
        // canonicalization, which adds a trailing `/` for empty paths).
        assert_eq!(redact_url("ws://127.0.0.1:49134"), "ws://127.0.0.1:49134/");
        // Username + password fully stripped.
        assert_eq!(
            redact_url("wss://user:secret@iii.example.com:1234/path"),
            "wss://iii.example.com:1234/path"
        );
        // Username only.
        assert_eq!(
            redact_url("wss://user@iii.example.com/"),
            "wss://iii.example.com/"
        );
        // Garbage strings fall through unchanged — no logging-time panics.
        assert_eq!(redact_url("not a url"), "not a url");
    }
}
