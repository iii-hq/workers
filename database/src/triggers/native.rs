//! Native change capture for Postgres: database triggers + LISTEN/NOTIFY.
//!
//! The statements path (`bus.rs`) hears only what this worker executes. A
//! database configured with `capture: native` instead installs an AFTER
//! trigger per bound table that `pg_notify`s a small JSON payload, and the
//! worker holds one dedicated (non-pooled) connection per database doing
//! LISTEN. Any client's committed write — psql, another worker, another
//! process — fires the same `database::row-changed` event, carrying the
//! primary-key values of the changed rows (capped, see `bus::KEY_CAP`) so a
//! listener learns WHICH rows changed without the writer's cooperation.
//!
//! Delivery is NOTIFY's: commit-gated (nothing fires for rolled-back
//! transactions) but at-most-once — notifications raised while the listener
//! connection is down are lost. Subscribers that cannot tolerate a gap must
//! reconcile on their own schedule; this is a doorbell, not a ledger.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_postgres::{AsyncMessage, Client, Connection, NoTls, Socket};

use super::bus::{now_ms, RowChangeBus, RowChangedEvent, KEY_CAP};
use super::sql::Op;
use crate::config::{CaptureMode, TlsConfig, WorkerConfig};
use crate::pool::tls::make_pg_connector;

/// The NOTIFY channel every iii trigger function raises on.
pub(crate) const CHANNEL: &str = "iii_row_changed";

/// The DDL that makes one table announce its changes: a shared trigger
/// function (idempotent to reinstall) plus three statement-level triggers.
/// Statement-level with transition tables gives a real row count without a
/// NOTIFY per row; `IF n = 0 THEN RETURN` keeps the "no rows, no event" rule.
///
/// The function looks the firing table's primary key up at fire time
/// (`TG_RELID` → `pg_index`), so one function body serves every bound table
/// and a key added later needs no reinstall. Key values are wrapped per type
/// so their JSON matches what `database::query` returns for the same column
/// (bigint/numeric as strings, bytea as base64); other types, including
/// timestamps and domains, fall to the bare column.
/// ponytail: timestamp/domain PKs are passed through unwrapped and will not
/// match `database::query`'s encoding; add a `to_char`/`typbasetype` arm when
/// someone actually keys a table on one.
///
/// `pg_notify` refuses payloads of 8000 bytes or more, and inside an AFTER
/// trigger that error would abort the WRITER's statement — so the key count
/// is halved until the payload fits, and any failure in key extraction falls
/// back to the count-only payload. Capturing keys must never break a write.
pub(crate) fn install_sql(table: &str) -> Result<String, String> {
    let target = quote_table(table)?;
    Ok(format!(
        r#"CREATE OR REPLACE FUNCTION iii_row_changed_notify() RETURNS trigger
LANGUAGE plpgsql AS $iii$
DECLARE
  n bigint := 0;
  src text := CASE WHEN TG_OP = 'DELETE' THEN 'old_rows' ELSE 'new_rows' END;
  cols text;
  keys jsonb;
  lim int := {KEY_CAP};
  payload text;
BEGIN
  IF TG_OP = 'DELETE' THEN
    SELECT count(*) INTO n FROM old_rows;
  ELSE
    SELECT count(*) INTO n FROM new_rows;
  END IF;
  IF n = 0 THEN
    RETURN NULL;
  END IF;
  SELECT string_agg(format('%L, %s', a.attname,
           CASE a.atttypid::regtype::text
             WHEN 'bigint'  THEN format('r.%I::text', a.attname)
             WHEN 'numeric' THEN format('r.%I::text', a.attname)
             WHEN 'bytea'   THEN format('translate(encode(r.%I, ''base64''), E''\n'', '''')', a.attname)
             ELSE format('r.%I', a.attname)
           END), ', ' ORDER BY a.attnum)
    INTO cols
    FROM pg_catalog.pg_index i
    JOIN pg_catalog.pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey)
   WHERE i.indrelid = TG_RELID AND i.indisprimary;
  LOOP
    IF cols IS NULL THEN
      payload := json_build_object(
        'table', TG_TABLE_SCHEMA || '.' || TG_TABLE_NAME,
        'op', lower(TG_OP),
        'n', n)::text;
      EXIT;
    END IF;
    keys := NULL;
    BEGIN
      EXECUTE format('SELECT jsonb_agg(jsonb_build_object(%s)) FROM (SELECT * FROM %I LIMIT %s) r',
                     cols, src, lim) INTO keys;
    EXCEPTION WHEN OTHERS THEN
      cols := NULL;
    END;
    IF cols IS NULL THEN
      CONTINUE;
    END IF;
    payload := json_build_object(
      'table', TG_TABLE_SCHEMA || '.' || TG_TABLE_NAME,
      'op', lower(TG_OP),
      'n', n,
      'keys', coalesce(keys, '[]'::jsonb))::text;
    EXIT WHEN octet_length(payload) < 8000 OR lim = 0;
    lim := lim / 2;
  END LOOP;
  PERFORM pg_notify('{CHANNEL}', payload);
  RETURN NULL;
END
$iii$;
DROP TRIGGER IF EXISTS iii_row_changed_ins ON {target};
CREATE TRIGGER iii_row_changed_ins AFTER INSERT ON {target}
  REFERENCING NEW TABLE AS new_rows FOR EACH STATEMENT
  EXECUTE FUNCTION iii_row_changed_notify();
DROP TRIGGER IF EXISTS iii_row_changed_upd ON {target};
CREATE TRIGGER iii_row_changed_upd AFTER UPDATE ON {target}
  REFERENCING NEW TABLE AS new_rows FOR EACH STATEMENT
  EXECUTE FUNCTION iii_row_changed_notify();
DROP TRIGGER IF EXISTS iii_row_changed_del ON {target};
CREATE TRIGGER iii_row_changed_del AFTER DELETE ON {target}
  REFERENCING OLD TABLE AS old_rows FOR EACH STATEMENT
  EXECUTE FUNCTION iii_row_changed_notify();
"#
    ))
}

/// Quote a `table` or `schema.table` reference so it is only ever an
/// identifier — binding config is a trust boundary and this string lands in
/// DDL. A name that does not exist fails loudly at CREATE TRIGGER.
/// Shared with the sqlite watcher: `"…"` quoting is valid in both dialects.
pub(crate) fn quote_table(t: &str) -> Result<String, String> {
    let t = t.trim();
    if t.is_empty() {
        return Err("table name is empty".into());
    }
    let parts: Vec<&str> = t.split('.').collect();
    if parts.len() > 2 {
        return Err(format!("table `{t}` must be `table` or `schema.table`"));
    }
    Ok(parts
        .iter()
        .map(|p| {
            let p = p.trim();
            // Accept an already-quoted part without double-wrapping it.
            let bare = p
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(p);
            format!("\"{}\"", bare.replace('"', "\"\""))
        })
        .collect::<Vec<_>>()
        .join("."))
}

/// What the trigger function sends. `op` reuses the wire enum, so
/// `lower(TG_OP)` maps directly onto insert/update/delete.
#[derive(Deserialize)]
struct Payload {
    table: String,
    op: Op,
    n: u64,
    /// Primary-key values of up to `KEY_CAP` changed rows (fewer when the
    /// NOTIFY size limit bit). Absent when the table has no primary key —
    /// or when the trigger function predates this field.
    #[serde(default)]
    keys: Option<Vec<serde_json::Map<String, serde_json::Value>>>,
}

/// A NOTIFY payload as a bus event, or None (with a warning) for payloads
/// this worker did not shape — someone else may NOTIFY on our channel.
pub(crate) fn parse_notification(db: &str, payload: &str) -> Option<RowChangedEvent> {
    let p: Payload = match serde_json::from_str(payload) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(db = %db, error = %e, "unparseable payload on {CHANNEL}; dropped");
            return None;
        }
    };
    // The trigger sends no flag: it returned min(n, lim) keys, so fewer keys
    // than rows means the cap bit. No keys at all (no primary key) is not
    // truncation — there was nothing to cap.
    let truncated = p.keys.as_ref().is_some_and(|k| (k.len() as u64) < p.n);
    Some(RowChangedEvent {
        db: db.to_string(),
        table: Some(p.table),
        op: p.op,
        affected_rows: p.n,
        returning: p.keys.filter(|k| !k.is_empty()),
        truncated,
        at: now_ms(),
    })
}

enum TaskHandle {
    /// Postgres LISTEN / mysql binlog task — a tokio task, aborted on removal.
    Async(tokio::task::JoinHandle<()>),
    /// Sqlite watcher — a dedicated OS thread (rusqlite is sync and the
    /// connection must stay put); told to stop via flag plus a wake poke so
    /// it exits within one drain instead of one fallback tick — a stopped
    /// watcher lingering next to its replacement would double-drain and
    /// double-GC the same changelog.
    Thread {
        stop: Arc<std::sync::atomic::AtomicBool>,
        wake: std::sync::mpsc::Sender<()>,
    },
}

impl TaskHandle {
    fn stop(&self) {
        match self {
            TaskHandle::Async(handle) => handle.abort(),
            TaskHandle::Thread { stop, wake } => {
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
                let _ = wake.send(());
            }
        }
    }
}

struct ListenerTask {
    /// Serialized DatabaseConfig; a reload that changes url/tls restarts the
    /// listener, one that leaves the db untouched does not.
    fingerprint: String,
    handle: TaskHandle,
}

/// One capture task per `capture: native` database — a LISTEN connection for
/// postgres, a changelog watcher thread for sqlite — reconciled against the
/// live config at startup and on every hot reload.
pub struct NativeListeners {
    bus: Arc<RowChangeBus>,
    tasks: Mutex<HashMap<String, ListenerTask>>,
}

impl NativeListeners {
    pub fn new(bus: Arc<RowChangeBus>) -> Self {
        Self {
            bus,
            tasks: Mutex::new(HashMap::new()),
        }
    }

    /// Start missing listeners, stop removed ones, restart changed ones.
    /// Must run inside a tokio runtime.
    pub fn sync(&self, cfg: &WorkerConfig) {
        let desired: HashMap<String, (crate::config::DatabaseConfig, String)> = cfg
            .databases
            .iter()
            .filter(|(_, db)| db.capture == CaptureMode::Native)
            .map(|(name, db)| {
                let fingerprint = serde_json::to_string(db).unwrap_or_default();
                (name.clone(), (db.clone(), fingerprint))
            })
            .collect();

        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        tasks.retain(|name, task| {
            let keep = desired
                .get(name)
                .is_some_and(|(_, fp)| *fp == task.fingerprint);
            if !keep {
                task.handle.stop();
                tracing::info!(db = %name, "native capture listener stopped");
            }
            keep
        });
        for (name, (db, fingerprint)) in desired {
            if tasks.contains_key(&name) {
                continue;
            }
            let Some(handle) = self.spawn(&name, &db) else {
                continue;
            };
            tasks.insert(
                name,
                ListenerTask {
                    fingerprint,
                    handle,
                },
            );
        }
    }

    fn spawn(&self, name: &str, db: &crate::config::DatabaseConfig) -> Option<TaskHandle> {
        match db.driver {
            crate::config::DriverKind::Postgres => {
                Some(TaskHandle::Async(tokio::spawn(run_listener(
                    name.to_string(),
                    db.url.clone(),
                    db.tls.clone(),
                    Arc::clone(&self.bus),
                ))))
            }
            crate::config::DriverKind::Sqlite => {
                let url = db.resolved_url();
                let Some(path) = super::sqlite_watch::sqlite_file_path(&url) else {
                    // Config validation rejects `:memory:`; reaching this
                    // means drift — fail visible, not silent.
                    tracing::warn!(db = %name, "native capture needs a file-backed sqlite url");
                    return None;
                };
                let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let (wake_tx, wake_rx) = std::sync::mpsc::channel::<()>();
                let bus = Arc::clone(&self.bus);
                let rt = tokio::runtime::Handle::current();
                let db_name = name.to_string();
                let thread_stop = Arc::clone(&stop);
                let thread_wake = wake_tx.clone();
                if let Err(e) = std::thread::Builder::new()
                    .name(format!("sqlite-capture-{name}"))
                    .spawn(move || {
                        super::sqlite_watch::run_watcher(
                            &db_name,
                            &path,
                            &thread_stop,
                            thread_wake,
                            &wake_rx,
                            |event| {
                                // Bridge sync → async: the watcher thread parks
                                // on the runtime while the bus fans out.
                                rt.block_on(bus.emit_event(event));
                                true
                            },
                        );
                    })
                {
                    // A database with no watcher hears nothing — that must
                    // never happen silently.
                    tracing::warn!(db = %name, error = %e, "sqlite capture watcher thread failed to spawn");
                    return None;
                }
                Some(TaskHandle::Thread {
                    stop,
                    wake: wake_tx,
                })
            }
            crate::config::DriverKind::Mysql => Some(TaskHandle::Async(tokio::spawn(
                super::mysql_binlog::run_binlog(
                    name.to_string(),
                    db.url.clone(),
                    db.tls.clone(),
                    Arc::clone(&self.bus),
                ),
            ))),
        }
    }

    #[cfg(test)]
    pub(crate) fn task_count(&self) -> usize {
        self.tasks.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// Hold a LISTEN connection open forever, reconnecting with capped backoff.
/// Events raised while disconnected are lost — see the module doc.
async fn run_listener(db: String, url: String, tls: TlsConfig, bus: Arc<RowChangeBus>) {
    let mut delay = Duration::from_secs(1);
    loop {
        match listen_once(&db, &url, &tls, &bus).await {
            Ok(()) => {
                tracing::warn!(db = %db, "native capture connection closed; reconnecting");
                delay = Duration::from_secs(1);
            }
            Err(e) => {
                tracing::warn!(db = %db, error = %e, "native capture listener error; retrying");
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(30));
    }
}

async fn listen_once(
    db: &str,
    url: &str,
    tls: &TlsConfig,
    bus: &RowChangeBus,
) -> Result<(), String> {
    // Dedicated connection, never the pool: LISTEN state is per-session, and
    // a pooled session's notifications would go to whoever holds the object.
    match make_pg_connector(tls).map_err(|e| format!("{e:?}"))? {
        Some(connector) => {
            let (client, conn) = tokio_postgres::connect(url, connector)
                .await
                .map_err(|e| e.to_string())?;
            session(db, client, conn, bus).await
        }
        None => {
            let (client, conn) = tokio_postgres::connect(url, NoTls)
                .await
                .map_err(|e| e.to_string())?;
            session(db, client, conn, bus).await
        }
    }
}

/// Drive one connection: issue LISTEN, then pump messages until the server
/// closes. `poll_message` both performs the connection's I/O and yields
/// notifications, so this single loop is the whole event pump.
async fn session<S>(
    db: &str,
    client: Client,
    mut conn: Connection<Socket, S>,
    bus: &RowChangeBus,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let listen = client.batch_execute("LISTEN iii_row_changed");
    tokio::pin!(listen);
    let mut listening = false;
    loop {
        tokio::select! {
            r = &mut listen, if !listening => {
                r.map_err(|e| e.to_string())?;
                listening = true;
                tracing::info!(db = %db, channel = CHANNEL, "native capture listening");
            }
            msg = std::future::poll_fn(|cx| conn.poll_message(cx)) => match msg {
                None => return Ok(()),
                Some(Err(e)) => return Err(e.to_string()),
                Some(Ok(AsyncMessage::Notification(n))) => {
                    if n.channel() == CHANNEL {
                        if let Some(event) = parse_notification(db, n.payload()) {
                            bus.emit_event(event).await;
                        }
                    }
                }
                // Notices and any future message kinds (enum is non_exhaustive).
                Some(Ok(_)) => {}
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_sql_quotes_identifiers_and_rejects_garbage() {
        let sql = install_sql("orders").unwrap();
        assert!(sql.contains("AFTER INSERT ON \"orders\""));
        assert!(sql.contains("AFTER UPDATE ON \"orders\""));
        assert!(sql.contains("AFTER DELETE ON \"orders\""));

        let sql = install_sql("public.orders").unwrap();
        assert!(sql.contains("ON \"public\".\"orders\""));

        // Already-quoted input is not double-wrapped.
        let sql = install_sql("\"Orders\"").unwrap();
        assert!(sql.contains("ON \"Orders\""));

        assert!(install_sql("  ").is_err());
        assert!(install_sql("a.b.c").is_err());
    }

    #[test]
    fn install_sql_neutralizes_injection_attempts() {
        // The binding config is a trust boundary; a hostile table name must
        // come out as one (weird) quoted identifier, never as loose SQL.
        let evil = r#"orders"; DROP TABLE users; --"#;
        let sql = install_sql(evil).unwrap();
        assert!(sql.contains(r#"ON "orders""; DROP TABLE users; --""#));
        assert!(!sql.contains(r#"ON orders"#));
    }

    #[test]
    fn install_sql_extracts_primary_keys_and_bounds_the_payload() {
        let sql = install_sql("orders").unwrap();
        // Key lookup is by the firing table's primary key, at fire time.
        assert!(
            sql.contains("i.indrelid = TG_RELID AND i.indisprimary"),
            "{sql}"
        );
        // Encoding parity with database::query: bigint/numeric as text,
        // bytea as unwrapped base64.
        assert!(
            sql.contains("WHEN 'bigint'  THEN format('r.%I::text'"),
            "{sql}"
        );
        assert!(sql.contains("encode(r.%I, ''base64'')"), "{sql}");
        // The cap is the shared one, and the payload is measured against the
        // NOTIFY limit before it is sent — an oversized pg_notify would abort
        // the writer's statement.
        assert!(sql.contains(&format!("lim int := {KEY_CAP};")), "{sql}");
        assert!(sql.contains("LIMIT %s"), "{sql}");
        assert!(sql.contains("octet_length(payload) < 8000"), "{sql}");
        // Key extraction failures degrade to the count-only payload.
        assert!(sql.contains("EXCEPTION WHEN OTHERS THEN"), "{sql}");
    }

    #[test]
    fn parse_notification_maps_payloads_and_drops_foreign_ones() {
        let ev = parse_notification(
            "primary",
            r#"{"table":"public.orders","op":"insert","n":3}"#,
        )
        .unwrap();
        assert_eq!(ev.db, "primary");
        assert_eq!(ev.table.as_deref(), Some("public.orders"));
        assert_eq!(ev.op, Op::Insert);
        assert_eq!(ev.affected_rows, 3);
        // No keys (no primary key, or a pre-keys trigger): no identity, and
        // that is not truncation.
        assert!(ev.returning.is_none());
        assert!(!ev.truncated);

        // Someone else NOTIFYing on our channel must not become an event.
        assert!(parse_notification("primary", "not json").is_none());
        assert!(parse_notification("primary", r#"{"table":"t","op":"vacuum","n":1}"#).is_none());
        assert!(parse_notification("primary", r#"{"op":"insert","n":1}"#).is_none());
        assert!(
            parse_notification("primary", r#"{"table":"t","op":"insert","n":1,"keys":"x"}"#)
                .is_none()
        );
    }

    #[test]
    fn parse_notification_maps_keys_onto_returning_and_derives_truncation() {
        let full = parse_notification(
            "primary",
            r#"{"table":"public.orders","op":"update","n":2,"keys":[{"id":"1","tenant":"a"},{"id":"2","tenant":"a"}]}"#,
        )
        .unwrap();
        let keys = full.returning.as_ref().unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[1]["id"], "2");
        assert_eq!(keys[1]["tenant"], "a");
        assert!(!full.truncated);

        // Fewer keys than rows: the cap (or the NOTIFY byte limit) bit.
        let capped = parse_notification(
            "primary",
            r#"{"table":"public.orders","op":"insert","n":3,"keys":[{"id":1}]}"#,
        )
        .unwrap();
        assert_eq!(capped.returning.as_ref().unwrap().len(), 1);
        assert_eq!(capped.affected_rows, 3);
        assert!(capped.truncated);

        // LIMIT 0 — not even one key fit: identity absent, truncation flagged.
        let none_fit = parse_notification(
            "primary",
            r#"{"table":"public.orders","op":"delete","n":5,"keys":[]}"#,
        )
        .unwrap();
        assert!(none_fit.returning.is_none());
        assert!(none_fit.truncated);
    }

    #[tokio::test]
    async fn sync_reconciles_listener_tasks_with_config() {
        let bus = Arc::new(RowChangeBus::new(
            Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:9")),
            100,
        ));
        let listeners = NativeListeners::new(bus);

        let native = |url: &str| {
            crate::config::WorkerConfig::from_yaml(&format!(
                "databases:\n  p:\n    url: {url}\n    capture: native\n    tls:\n      mode: disable\n"
            ))
            .unwrap()
        };

        // Port 1 refuses connections; the task just retries in background.
        listeners.sync(&native("postgres://u@127.0.0.1:1/db"));
        assert_eq!(listeners.task_count(), 1);

        // Same config → same task, not a restart.
        listeners.sync(&native("postgres://u@127.0.0.1:1/db"));
        assert_eq!(listeners.task_count(), 1);

        // Changed url → replaced. Removed → stopped.
        listeners.sync(&native("postgres://u@127.0.0.1:2/db"));
        assert_eq!(listeners.task_count(), 1);
        listeners.sync(&crate::config::WorkerConfig::default());
        assert_eq!(listeners.task_count(), 0);
    }

    /// The claim this feature exists for: a write from a *different
    /// connection* (stand-in for a different process) raises a notification
    /// the worker can parse. Requires TEST_POSTGRES_URL, like the pool tests.
    #[tokio::test(flavor = "multi_thread")]
    async fn native_capture_hears_writes_from_another_connection() {
        let Some(url) = std::env::var("TEST_POSTGRES_URL").ok() else {
            eprintln!("skipping: TEST_POSTGRES_URL not set");
            return;
        };

        let (listener, mut conn) = tokio_postgres::connect(&url, NoTls).await.unwrap();
        let (writer, writer_conn) = tokio_postgres::connect(&url, NoTls).await.unwrap();
        tokio::spawn(async move {
            let _ = writer_conn.await;
        });

        /// Await a client call while pumping its connection — client futures
        /// only resolve while someone polls the connection (`session` does
        /// this for the real listener). Notices are consumed and dropped.
        async fn drive<T>(
            conn: &mut Connection<Socket, tokio_postgres::tls::NoTlsStream>,
            fut: impl std::future::Future<Output = T>,
        ) -> T {
            tokio::pin!(fut);
            loop {
                tokio::select! {
                    r = &mut fut => return r,
                    msg = std::future::poll_fn(|cx| conn.poll_message(cx)) => {
                        msg.expect("connection open").expect("no protocol error");
                    }
                }
            }
        }

        // Table + triggers, installed the way the handler installs them.
        let table = format!("iii_native_capture_{}", std::process::id());
        drive(&mut conn, async {
            listener
                .batch_execute(&format!(
                    "DROP TABLE IF EXISTS {table}; CREATE TABLE {table} (id int, n int);"
                ))
                .await
                .unwrap();
            listener
                .batch_execute(&install_sql(&table).unwrap())
                .await
                .unwrap();
            listener
                .batch_execute("LISTEN iii_row_changed")
                .await
                .unwrap();
        })
        .await;

        // The "other process" writes: 2 inserts, 1 update, 1 delete.
        writer
            .batch_execute(&format!(
                "INSERT INTO {table} VALUES (1, 10), (2, 20); \
                 UPDATE {table} SET n = 5; \
                 DELETE FROM {table} WHERE id = 1; \
                 UPDATE {table} SET n = 9 WHERE id = 999;" // 0 rows → no event
            ))
            .await
            .unwrap();

        let mut events = Vec::new();
        while events.len() < 3 {
            let msg = tokio::time::timeout(
                Duration::from_secs(5),
                std::future::poll_fn(|cx| conn.poll_message(cx)),
            )
            .await
            .expect("notification within 5s")
            .expect("connection open")
            .expect("no protocol error");
            if let AsyncMessage::Notification(n) = msg {
                assert_eq!(n.channel(), CHANNEL);
                events.push(parse_notification("primary", n.payload()).unwrap());
            }
        }

        assert_eq!(events[0].op, Op::Insert);
        assert_eq!(events[0].affected_rows, 2);
        assert_eq!(events[1].op, Op::Update);
        assert_eq!(events[1].affected_rows, 2);
        assert_eq!(events[2].op, Op::Delete);
        assert_eq!(events[2].affected_rows, 1);
        for ev in &events {
            assert!(crate::triggers::sql::same_table(
                ev.table.as_deref().unwrap(),
                &table
            ));
            // `(id int, n int)` has no primary key: count-only, as before.
            assert!(ev.returning.is_none(), "{ev:?}");
            assert!(!ev.truncated, "{ev:?}");
        }

        // A keyed table: the event names the changed rows. Composite key with
        // the three types that need wrapping to match database::query —
        // bigint as string, uuid as text, bytea as base64.
        let keyed = format!("{table}_keyed");
        let wide = format!("{table}_wide");
        drive(&mut conn, async {
            listener
                .batch_execute(&format!(
                    "DROP TABLE IF EXISTS {keyed}; \
                     CREATE TABLE {keyed} (id bigint, u uuid, b bytea, n int, PRIMARY KEY (id, u, b)); \
                     DROP TABLE IF EXISTS {wide}; \
                     CREATE TABLE {wide} (k text PRIMARY KEY);"
                ))
                .await
                .unwrap();
            listener
                .batch_execute(&install_sql(&keyed).unwrap())
                .await
                .unwrap();
            listener
                .batch_execute(&install_sql(&wide).unwrap())
                .await
                .unwrap();
        })
        .await;

        // 20 one-kilobyte keys do not fit one NOTIFY payload: the trigger must
        // halve the key count until it fits rather than abort the INSERT.
        let wide_values = (0..20)
            .map(|i| format!("('{}')", format!("{i:04}").repeat(250)))
            .collect::<Vec<_>>()
            .join(", ");
        writer
            .batch_execute(&format!(
                "INSERT INTO {keyed} VALUES (9007199254740993, '550e8400-e29b-41d4-a716-446655440000', '\\x010203'::bytea, 1); \
                 DELETE FROM {keyed}; \
                 INSERT INTO {wide} VALUES {wide_values};"
            ))
            .await
            .unwrap();

        let mut keyed_events = Vec::new();
        while keyed_events.len() < 3 {
            let msg = tokio::time::timeout(
                Duration::from_secs(5),
                std::future::poll_fn(|cx| conn.poll_message(cx)),
            )
            .await
            .expect("notification within 5s")
            .expect("connection open")
            .expect("no protocol error");
            if let AsyncMessage::Notification(n) = msg {
                keyed_events.push(parse_notification("primary", n.payload()).unwrap());
            }
        }

        let expected_key = serde_json::json!({
            "id": "9007199254740993",
            "u": "550e8400-e29b-41d4-a716-446655440000",
            "b": "AQID"
        });
        for ev in &keyed_events[..2] {
            assert!(crate::triggers::sql::same_table(
                ev.table.as_deref().unwrap(),
                &keyed
            ));
            let keys = ev.returning.as_ref().expect("keyed table carries keys");
            assert_eq!(keys.len(), 1, "{ev:?}");
            assert_eq!(serde_json::Value::Object(keys[0].clone()), expected_key);
            assert!(!ev.truncated);
        }
        assert_eq!(keyed_events[0].op, Op::Insert);
        assert_eq!(keyed_events[1].op, Op::Delete);

        let capped = &keyed_events[2];
        assert!(crate::triggers::sql::same_table(
            capped.table.as_deref().unwrap(),
            &wide
        ));
        assert_eq!(capped.op, Op::Insert);
        assert_eq!(capped.affected_rows, 20, "the count stays exact");
        let keys = capped.returning.as_ref().expect("some keys still fit");
        assert!(
            keys.len() < 20,
            "{} keys should not all fit 8000 bytes",
            keys.len()
        );
        assert!(capped.truncated);

        drive(&mut conn, async {
            listener
                .batch_execute(&format!("DROP TABLE {keyed}; DROP TABLE {wide};"))
                .await
                .unwrap();
        })
        .await;

        let _ = drive(
            &mut conn,
            listener.batch_execute(&format!("DROP TABLE {table}")),
        )
        .await;
    }
}
