use anyhow::{Context, Result};
use clap::Parser;
use database::config::WorkerConfig;
use database::configuration;
use database::handle::HandleRegistry;
use database::handlers::{
    begin_transaction::{self, BeginTxReq},
    commit_transaction::{self, CommitTxReq},
    execute::{self, ExecuteReq},
    execute_batch::{self, ExecuteBatchReq},
    list_databases::{self, ListDatabasesReq},
    prepare::{self, PrepareReq},
    query::{self, QueryReq},
    rollback_transaction::{self, RollbackTxReq},
    run_statement::{self, RunReq},
    transaction::{self, TxReq},
    transaction_execute::{self, TxExecuteReq},
    transaction_query::{self, TxQueryReq},
    AppState,
};
use database::transaction::TxRegistry;
use iii_helpers::observability::{Logger, OtelConfig};
use iii_sdk::{register_worker, InitOptions, RegisterFunction, RegisterTriggerType};
use std::sync::Arc;
use tokio::sync::RwLock;

/// A row-changed subscriber must not be able to stall the write that already
/// committed; the dispatch is void and bounded by this.
const ROW_CHANGE_DISPATCH_TIMEOUT_MS: u64 = 10_000;

#[derive(Parser, Debug)]
#[command(
    name = "database",
    about = "database worker (PostgreSQL, MySQL, SQLite)"
)]
struct Cli {
    /// Optional seed config.yaml used to populate `initial_value` on first register
    #[arg(long)]
    config: Option<String>,

    /// WebSocket URL of the iii engine
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
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
        name = database::worker_name(),
        seed_config = cli.config.as_deref().unwrap_or("(none)"),
        url = %redact_url(&cli.url),
        "starting"
    );

    // Arc-wrapped for `ui::register` (the console-ui crate clones the client
    // into its hot-reload watcher task); everything else auto-derefs.
    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    ));

    let seed = match &cli.config {
        Some(path) => match WorkerConfig::from_file(path) {
            Ok(cfg) => {
                tracing::info!(path = %path, "loaded seed config for initial registration");
                Some(cfg)
            }
            Err(e) => {
                tracing::warn!(
                    path = %path,
                    error = %e,
                    "failed to load seed config; relying on existing configuration entry"
                );
                None
            }
        },
        None => None,
    };

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering database configuration schema")?;

    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading database configuration")?;

    let pools = configuration::build_pools(&cfg)
        .await
        .map_err(anyhow::Error::msg)
        .context("building initial connection pools")?;

    let handles = Arc::new(HandleRegistry::new());
    let transactions = TxRegistry::new();
    let log = Logger::new();
    let row_changes = Arc::new(database::triggers::RowChangeBus::new(
        iii.clone(),
        ROW_CHANGE_DISPATCH_TIMEOUT_MS,
    ));
    // One LISTEN task per `capture: native` postgres database, live from
    // startup — external writes must be heard before any binding registers.
    let native_listeners = Arc::new(database::triggers::NativeListeners::new(row_changes.clone()));
    native_listeners.sync(&cfg);
    let state = AppState {
        pools: Arc::new(RwLock::new(pools)),
        config: Arc::new(RwLock::new(cfg)),
        handles: handles.clone(),
        transactions: transactions.clone(),
        log: log.clone(),
        row_changes: Some(row_changes.clone()),
    };

    let _evictor = handles.spawn_evictor();
    let _tx_watcher = transactions.spawn_timeout_watcher(log.clone(), Some(row_changes.clone()));

    {
        let st = state.clone();
        iii.register_function(
            "database::query",
            RegisterFunction::new_async(move |req: QueryReq| {
                let st = st.clone();
                async move {
                    query::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description("Run a read-only SQL query and return the result rows."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::execute",
            RegisterFunction::new_async(move |req: ExecuteReq| {
                let st = st.clone();
                async move {
                    execute::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description("Run a write statement (INSERT/UPDATE/DELETE/DDL)."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::executeBatch",
            RegisterFunction::new_async(move |req: ExecuteBatchReq| {
                let st = st.clone();
                async move {
                    execute_batch::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Run an ordered batch of SQL statements atomically (bare strings or \
                 {sql, params} objects); rolls back on first failure.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::prepareStatement",
            RegisterFunction::new_async(move |req: PrepareReq| {
                let st = st.clone();
                async move {
                    prepare::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description("Prepare a parameterized statement once."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::runStatement",
            RegisterFunction::new_async(move |req: RunReq| {
                let st = st.clone();
                async move {
                    run_statement::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description("Run a previously-prepared handle."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::transaction",
            RegisterFunction::new_async(move |req: TxReq| {
                let st = st.clone();
                async move {
                    transaction::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description("Run a sequence of statements atomically."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::beginTransaction",
            RegisterFunction::new_async(move |req: BeginTxReq| {
                let st = st.clone();
                async move {
                    begin_transaction::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
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
            "database::transactionQuery",
            RegisterFunction::new_async(move |req: TxQueryReq| {
                let st = st.clone();
                async move {
                    transaction_query::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description("Run a read-only SQL query inside an interactive transaction."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::transactionExecute",
            RegisterFunction::new_async(move |req: TxExecuteReq| {
                let st = st.clone();
                async move {
                    transaction_execute::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Run a write statement inside an interactive transaction. \
                 BEGIN/COMMIT/ROLLBACK are rejected; use commit/rollbackTransaction.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::commitTransaction",
            RegisterFunction::new_async(move |req: CommitTxReq| {
                let st = st.clone();
                async move {
                    commit_transaction::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description("Commit and finalize an interactive transaction."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::rollbackTransaction",
            RegisterFunction::new_async(move |req: RollbackTxReq| {
                let st = st.clone();
                async move {
                    rollback_transaction::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description("Rollback and finalize an interactive transaction."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::listDatabases",
            RegisterFunction::new_async(move |req: ListDatabasesReq| {
                let st = st.clone();
                async move {
                    list_databases::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "List all configured databases with connection details (driver, \
                 credential-redacted URL, pool settings, TLS mode). Config only — \
                 no health checks or live pool statistics.",
            ),
        );
    }

    // The worker announces its own writes. Registered AFTER the functions so
    // the console can attribute the type, and gated on the databases that
    // actually exist — a binding on a typo'd handle would listen to nothing.
    let _row_changed = iii.register_trigger_type(
        RegisterTriggerType::new(
            database::triggers::ROW_CHANGED_TYPE,
            "Fires after this worker commits a row change, filtered by `db`, optional `table`, and optional `ops`. \
             Reports only mutations made THROUGH this worker — not change data capture.",
            database::triggers::RowChangedHandler {
                bus: row_changes.clone(),
                config: state.config.clone(),
                pools: state.pools.clone(),
            },
        )
        .trigger_request_format::<database::triggers::RowChangedConfig>()
        .call_request_format::<database::triggers::RowChangedEvent>(),
    );

    configuration::register_config_trigger(&iii, state.clone(), Some(native_listeners.clone()))
        .context("registering configuration change trigger")?;

    // Injectable console UI (function-trigger renderer) — after the
    // database::* functions so the console can attribute the assets.
    database::ui::register(&iii);

    tracing::info!(
        "database worker registered 13 functions and 1 trigger type, waiting for invocations"
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
