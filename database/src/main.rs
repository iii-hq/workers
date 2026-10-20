use anyhow::{Context, Result};
use clap::Parser;
use database::config::WorkerConfig;
use database::configuration;
use database::handle::HandleRegistry;
use database::handlers::{
    begin_transaction::{self, BeginTxReq},
    browse::{self, BrowseTableReq},
    column_stats::{self, ColumnStatsReq},
    commit_transaction::{self, CommitTxReq},
    connections::{self, DeleteConnectionReq, GetConnectionReq, SaveConnectionReq},
    diagram::{self, SchemaDiagramReq},
    distinct_values::{self, DistinctValuesReq},
    execute::{self, ExecuteReq},
    execute_batch::{self, ExecuteBatchReq},
    explain::{self, ExplainReq},
    format_sql::{self, FormatSqlReq},
    health::{self, HealthReq, TerminateReq},
    insert_row::{self, InsertRowReq},
    list_databases::{self, ListDatabasesReq},
    prepare::{self, PrepareReq},
    query::{self, QueryReq},
    rollback_transaction::{self, RollbackTxReq},
    run_statement::{self, RunReq},
    saved::{self, DeleteSavedReq, HistoryReq, ListSavedReq, SaveQueryReq},
    schema::{self, DescribeSchemaReq, DescribeTableReq, ListTablesReq},
    table_view::{self, GetTableViewReq, SaveTableViewReq},
    test_connection::{self, TestConnectionReq},
    transaction::{self, TxReq},
    transaction_execute::{self, TxExecuteReq},
    transaction_query::{self, TxQueryReq},
    AppState,
};
use database::transaction::TxRegistry;
use database::triggers::NativeCapture;
use iii_helpers::observability::{Logger, OtelConfig};
use iii_sdk::{register_worker, InitOptions, RegisterFunction};
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
                // russh logs protocol detail at debug; keep it out of the
                // default output (credentials are never logged either way).
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,russh=warn")),
        )
        .init();

    let cli = Cli::parse();
    tracing::info!(
        name = database::worker_name(),
        seed_config = cli.config.as_deref().unwrap_or("(none)"),
        url = %redact_url(&cli.url),
        "starting"
    );

    // Identify as `database` in the console's workers view instead of the
    // SDK's hostname:pid fallback. III_WORKER_NAME still wins — that is the
    // managed-spawn identity contract (the engine exports it for workers it
    // owns), and hand-run instances (workers-dev) can use it to tag
    // themselves per worktree.
    let mut metadata = iii_sdk::runtime::WorkerMetadata::default();
    if std::env::var("III_WORKER_NAME").map_or(true, |v| v.is_empty()) {
        metadata.name = database::worker_name().to_string();
    }
    metadata.description = Some(
        "SQL for PostgreSQL, MySQL, and SQLite: queries, statements, interactive \
         transactions, and database::on-row-changed events (statements or native capture)."
            .to_string(),
    );

    // Arc-wrapped for `ui::register` (the console-ui crate clones the client
    // into its hot-reload watcher task); everything else auto-derefs.
    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(metadata),
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

    // `secret://` / `env://` values resolve through the secrets worker; the
    // cache is filled when the configuration is applied below.
    let runtime = Arc::new(database::runtime::Runtime::new(Arc::new(
        database::secrets::SecretCache::new(database::secrets::bus_fetch(iii.clone())),
    )));

    let handles = Arc::new(HandleRegistry::new());
    let transactions = TxRegistry::new();
    let log = Logger::new();
    // Filled by `apply_config` below: a connection whose pool cannot be built
    // (unresolvable secret, SSH failure) is reported, not fatal.
    let pools = Arc::new(RwLock::new(std::collections::HashMap::new()));
    let live_config = Arc::new(RwLock::new(cfg.clone()));
    // Native capture and the routing index of `database::on-row-changed`,
    // driven by its observers (and, against a pre-0.25 engine, by the old
    // trigger type's bindings).
    let capture = Arc::new(NativeCapture::new(
        iii.clone(),
        live_config.clone(),
        pools.clone(),
        ROW_CHANGE_DISPATCH_TIMEOUT_MS,
    ));
    let row_changes = Arc::new(database::triggers::RowChangeBus::with_capture(
        iii.clone(),
        capture.clone(),
    ));
    // One LISTEN task per `capture: native` postgres database, live from
    // startup — external writes must be heard before any observer registers.
    let native_listeners = Arc::new(database::triggers::NativeListeners::new(
        row_changes.clone(),
    ));
    runtime.set_listeners(native_listeners.clone());
    let state = AppState {
        pools,
        config: live_config,
        handles: handles.clone(),
        transactions: transactions.clone(),
        log: log.clone(),
        row_changes: Some(row_changes.clone()),
        runtime,
    };
    configuration::apply_config(&state, cfg, Some(&native_listeners))
        .await
        .map_err(anyhow::Error::msg)
        .context("building initial connection pools")?;

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
            // The const carries the discovery rule (first sentence ≤ 160
            // bytes — all `directory::search_functions` shows an agent) and
            // is pinned by a test in `handlers::execute`.
            .description(execute::DESCRIPTION),
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
                 {sql, params} objects); rolls back on first failure. A statement's \
                 RETURNING rows come back in results[].rows.",
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
            .description("Prepare a parameterized SQL statement once for repeated runs."),
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
            .description("Run a prepared SQL statement by handle, passing fresh parameters."),
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
            .description(
                "Run a list of SQL statements in one atomic transaction. A statement's \
                 RETURNING rows come back in results[].rows and ride its row-changed event.",
            ),
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
            .description("Query rows inside an open transaction."),
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
                "Execute a write inside an open transaction; put RETURNING in the SQL to get \
                 rows back. BEGIN/COMMIT/ROLLBACK are rejected; use commit/rollbackTransaction.",
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
            .description("Commit an open SQL transaction, making its writes permanent."),
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
            .description("Roll back an open SQL transaction, discarding its uncommitted writes."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::testConnection",
            RegisterFunction::new_async(move |req: TestConnectionReq| {
                let st = st.clone();
                async move {
                    test_connection::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Test a candidate connection entry (url or structured form, TLS, SSH tunnel, \
                 secret:// values) with one throwaway connection. Configured pools are \
                 untouched; reports ok/driver/latency/server version, the failing stage and \
                 the SSH host key; failures are data, not errors.",
            )
            // Candidate credentials ride these payloads: no observer may
            // subscribe to them.
            .observe(iii_sdk::Observe::auto().allow(false)),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::getConnection",
            RegisterFunction::new_async(move |req: GetConnectionReq| {
                let st = st.clone();
                async move {
                    connections::get(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Read one stored connection entry for editing: references and templates \
                 intact, literal passwords, passphrases and keys as \"__redacted__\".",
            )
            // Candidate credentials ride these payloads: no observer may
            // subscribe to them.
            .observe(iii_sdk::Observe::auto().allow(false)),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::saveConnection",
            RegisterFunction::new_async(move |req: SaveConnectionReq| {
                let st = st.clone();
                async move {
                    connections::save(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Create, update (previous_name = name) or rename (previous_name = old) a \
                 connection in the database configuration; \"__redacted__\" keeps a stored \
                 value. Returns after the pools reloaded, with the connection's error if any.",
            )
            // Candidate credentials ride these payloads: no observer may
            // subscribe to them.
            .observe(iii_sdk::Observe::auto().allow(false)),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::deleteConnection",
            RegisterFunction::new_async(move |req: DeleteConnectionReq| {
                let st = st.clone();
                async move {
                    connections::delete(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Remove a connection from the database configuration and close its pool. \
                 The last connection cannot be removed.",
            ),
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
    {
        let st = state.clone();
        let client = iii.clone();
        iii.register_function(
            "database::getTableView",
            RegisterFunction::new_async(move |req: GetTableViewReq| {
                let (st, client) = (st.clone(), client.clone());
                async move {
                    let db = st
                        .resolve_db(req.db.clone())
                        .await
                        .map_err(database::handlers::query::err_to_str)?;
                    table_view::get(&client, &db, &req.table)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Get a table's column layout for reading: widths, hidden columns and \
                 column order. Stored in the state worker rather than a browser, so it \
                 survives a restart and any caller can set it up for someone else.",
            ),
        );
    }
    {
        let st = state.clone();
        let client = iii.clone();
        iii.register_function(
            "database::saveTableView",
            RegisterFunction::new_async(move |req: SaveTableViewReq| {
                let (st, client) = (st.clone(), client.clone());
                async move {
                    let db = st
                        .resolve_db(req.db.clone())
                        .await
                        .map_err(database::handlers::query::err_to_str)?;
                    table_view::save(&client, &db, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Replace the stored layout for a table. Widths are clamped to a usable \
                 range; columns the table no longer has are kept rather than rejected, \
                 so a rename degrades to a missing width instead of an error.",
            ),
        );
    }
    {
        let st = state.clone();
        let client = iii.clone();
        iii.register_function(
            "database::saveQuery",
            RegisterFunction::new_async(move |req: SaveQueryReq| {
                let (st, client) = (st.clone(), client.clone());
                async move {
                    let db = st
                        .resolve_db(req.db.clone())
                        .await
                        .map_err(database::handlers::query::err_to_str)?;
                    saved::save(&client, &db, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Save a named snippet to the web console's saved-query list (state \
                 worker): console-only, not a file. To save a query as a .sql file, write \
                 it into the connection's queries folder (listDatabases `folder`) with the \
                 file tools — that is what the apps list. Saving under an existing name \
                 replaces it.",
            ),
        );
    }
    {
        let st = state.clone();
        let client = iii.clone();
        iii.register_function(
            "database::listSavedQueries",
            RegisterFunction::new_async(move |req: ListSavedReq| {
                let (st, client) = (st.clone(), client.clone());
                async move {
                    let db = st
                        .resolve_db(req.db.clone())
                        .await
                        .map_err(database::handlers::query::err_to_str)?;
                    saved::list(&client, &db)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description("List the saved queries for a database, sorted by name."),
        );
    }
    {
        let st = state.clone();
        let client = iii.clone();
        iii.register_function(
            "database::deleteSavedQuery",
            RegisterFunction::new_async(move |req: DeleteSavedReq| {
                let (st, client) = (st.clone(), client.clone());
                async move {
                    let db = st
                        .resolve_db(req.db.clone())
                        .await
                        .map_err(database::handlers::query::err_to_str)?;
                    saved::delete(&client, &db, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description("Delete a saved query by id or by name."),
        );
    }
    {
        let st = state.clone();
        let client = iii.clone();
        iii.register_function(
            "database::history",
            RegisterFunction::new_async(move |req: HistoryReq| {
                let (st, client) = (st.clone(), client.clone());
                async move {
                    let db = st
                        .resolve_db(req.db.clone())
                        .await
                        .map_err(database::handlers::query::err_to_str)?;
                    saved::history(&client, &db, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "List recent queries run against a database, newest first. Best effort \
                 — recording never blocks or fails a query, so it is a convenience, not \
                 an audit log. For an audit trail bind database::row-changed.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::schemaDiagram",
            RegisterFunction::new_async(move |req: SchemaDiagramReq| {
                let st = st.clone();
                async move {
                    diagram::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Diagram the database schema: tables as positioned nodes, foreign \
                 keys as routed edges. Also reports hub degree per table, isolated \
                 tables and remaining edge crossings. Reads the whole catalog in a \
                 handful of queries rather than one per table.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::columnStats",
            RegisterFunction::new_async(move |req: ColumnStatsReq| {
                let st = st.clone();
                async move {
                    column_stats::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Profile a table's columns. Reads the planner's own statistics by \
                 default, which is free and approximate; `exact` runs real aggregates \
                 and scans the table. To profile rows you already hold, pipe a \
                 browseTable result through the fp worker instead.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::health",
            RegisterFunction::new_async(move |req: HealthReq| {
                let st = st.clone();
                async move {
                    health::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Check live pool occupancy plus active queries, table sizes, blocking \
                 locks and cache hit ratio. Each section reports separately as \
                 available, unsupported or denied, so a driver gap or a restricted role \
                 never reads as an empty result.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::terminateQuery",
            RegisterFunction::new_async(move |req: TerminateReq| {
                let st = st.clone();
                async move {
                    health::terminate(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Terminate a backend session, or cancel just its running statement \
                 with `cancel_only`. Takes an id from database::health. Separate from \
                 health because it is a write.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::explain",
            RegisterFunction::new_async(move |req: ExplainReq| {
                let st = st.clone();
                async move {
                    explain::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Explain a statement's query plan as a tree with per-node costs, row \
                 estimates and warnings. `analyze` collects real timings by RUNNING the \
                 statement, so it defaults to false and is refused for anything that is \
                 not a single read.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::browseTable",
            RegisterFunction::new_async(move |req: BrowseTableReq| {
                let st = st.clone();
                async move {
                    browse::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Read a table page by page with typed filters and sorts, without \
                 writing SQL. Filters are structured (column, op, value) and \
                 compile to a parameterised WHERE for the driver in hand; the \
                 total honours the same filters. Use an equality filter at \
                 page_size 1 to follow a foreign key.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::listTables",
            RegisterFunction::new_async(move |req: ListTablesReq| {
                let st = st.clone();
                async move {
                    schema::list_tables(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "List every table and view in a database, with its kind and (on \
                 postgres) its schema. Reads the driver's own catalog, so no \
                 dialect-specific SQL is needed from the caller.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::describeTable",
            RegisterFunction::new_async(move |req: DescribeTableReq| {
                let st = st.clone();
                async move {
                    schema::describe_table(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Describe one table or view: columns with type, nullability, \
                 default, primary-key membership and foreign-key target; plus \
                 indexes and a planner row estimate. Foreign keys are structured \
                 (schema, table, column), not a joined string.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::describeSchema",
            RegisterFunction::new_async(move |req: DescribeSchemaReq| {
                let st = st.clone();
                async move {
                    schema::describe_schema(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Describe every table at once — the same shape as describeTable, \
                 but one catalog query per aspect across the whole database \
                 instead of one call per table. Use this to reason about \
                 relationships; set include_indexes only when you need them.",
            ),
        );
    }

    {
        let st = state.clone();
        iii.register_function(
            "database::distinctValues",
            RegisterFunction::new_async(move |req: DistinctValuesReq| {
                let st = st.clone();
                async move {
                    distinct_values::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "List the distinct values of one column, most frequent first, with their row \
                 counts — for a filter's value picker. `search` narrows by a case-insensitive \
                 substring of the text form.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::insertRow",
            RegisterFunction::new_async(move |req: InsertRowReq| {
                let st = st.clone();
                async move {
                    insert_row::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Insert one row from a {column: value} map; values are cast to each column's \
                 catalog type (numbers and dates as strings, bytea/blob as base64) and the \
                 inserted row comes back. Columns left out take their default.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "database::formatSql",
            RegisterFunction::new_async(move |req: FormatSqlReq| {
                let st = st.clone();
                async move {
                    format_sql::handle(&st, req)
                        .await
                        .map_err(iii_sdk::errors::Error::from)
                }
            })
            .description(
                "Pretty-print SQL for a driver's dialect: keywords cased, clauses indented, \
                 statements and comments kept. Pure text; nothing runs.",
            ),
        );
    }

    {
        let st = state.clone();
        iii.register_function(
            database::handlers::mentions::SEARCH_FN,
            RegisterFunction::new_async(move |req: mention_contract::MentionSearchRequest| {
                let st = st.clone();
                async move {
                    Ok::<_, iii_sdk::errors::Error>(
                        database::handlers::mentions::search(&st, req).await,
                    )
                }
            })
            .description(
                "Search the configured connections for the chat @database mention menu, by \
                 name, driver or host; best match first.",
            )
            .metadata(database::handlers::mentions::search_metadata()),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            database::handlers::mentions::GET_FN,
            RegisterFunction::new_async(move |req: mention_contract::MentionGetRequest| {
                let st = st.clone();
                async move {
                    Ok::<_, iii_sdk::errors::Error>(
                        database::handlers::mentions::get(&st, req).await,
                    )
                }
            })
            .description(
                "Resolve a @database(id=…) chat mention to its pill, preview card and agent \
                 summary; null for an unknown connection. Declares the database mention provider.",
            )
            .metadata(database::handlers::mentions::provider().metadata()),
        );
    }

    // The worker announces its own writes through an owner-delivered function
    // its observers name. Registered AFTER the other functions so the console
    // can attribute it.
    database::triggers::capture::register(&iii, &capture);

    // What depends on the engine's observe support follows it, as the SDK
    // learns it on each connection: an engine older than 0.25 gets the old
    // trigger type over the same index (old consumers keep hearing their
    // writes) and the old `configuration` trigger. An engine upgraded or
    // rolled back under the worker switches both without a restart.
    let fallback = Arc::new(database::triggers::legacy::Fallback::new(
        iii.as_ref().clone(),
        capture.clone(),
    ));
    let config_binding =
        configuration::register_config_trigger(&iii, state.clone(), Some(native_listeners.clone()));
    let _engine_follower = iii.on_engine_supports_observe(move |engine_observes| {
        fallback.follow(engine_observes);
        config_binding.follow(engine_observes);
    });
    configuration::register_secret_trigger(&iii, state.clone(), Some(native_listeners.clone()));
    let _maintenance =
        configuration::spawn_maintenance(state.clone(), Some(native_listeners.clone()));

    // Injectable console UI (function-trigger renderer) — after the
    // database::* functions so the console can attribute the assets.
    database::ui::register(&iii);

    tracing::info!(
        engine_observes = ?iii.engine_supports_observe(),
        "database worker registered 39 functions, waiting for invocations"
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
