//! Native change capture for Postgres: database triggers + LISTEN/NOTIFY.
//!
//! The statements path (`bus.rs`) hears only what this worker executes. A
//! database configured with `capture: native` instead installs an AFTER
//! trigger per bound table that `pg_notify`s a small JSON payload, and the
//! worker holds one dedicated (non-pooled) connection per database doing
//! LISTEN. Any client's committed write — psql, another worker, another
//! process — fires the same `database::row-changed` event.
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

use super::bus::{now_ms, RowChangeBus, RowChangedEvent};
use super::sql::Op;
use crate::config::{CaptureMode, TlsConfig, WorkerConfig};
use crate::pool::tls::make_pg_connector;

/// The NOTIFY channel every iii trigger function raises on.
pub(crate) const CHANNEL: &str = "iii_row_changed";

/// The DDL that makes one table announce its changes: a shared trigger
/// function (idempotent to reinstall) plus three statement-level triggers.
/// Statement-level with transition tables gives a real row count without a
/// NOTIFY per row; `IF n > 0` keeps the existing "no rows, no event" rule.
pub(crate) fn install_sql(table: &str) -> Result<String, String> {
    let target = quote_table(table)?;
    Ok(format!(
        r#"CREATE OR REPLACE FUNCTION iii_row_changed_notify() RETURNS trigger
LANGUAGE plpgsql AS $iii$
DECLARE n bigint := 0;
BEGIN
  IF TG_OP = 'DELETE' THEN
    SELECT count(*) INTO n FROM old_rows;
  ELSE
    SELECT count(*) INTO n FROM new_rows;
  END IF;
  IF n > 0 THEN
    PERFORM pg_notify('{CHANNEL}', json_build_object(
      'table', TG_TABLE_SCHEMA || '.' || TG_TABLE_NAME,
      'op', lower(TG_OP),
      'n', n)::text);
  END IF;
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
fn quote_table(t: &str) -> Result<String, String> {
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
    Some(RowChangedEvent {
        db: db.to_string(),
        table: Some(p.table),
        op: p.op,
        affected_rows: p.n,
        returning: None,
        at: now_ms(),
    })
}

struct ListenerTask {
    /// Serialized DatabaseConfig; a reload that changes url/tls restarts the
    /// listener, one that leaves the db untouched does not.
    fingerprint: String,
    handle: tokio::task::JoinHandle<()>,
}

/// One LISTEN task per `capture: native` database, reconciled against the
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
        let desired: HashMap<String, (String, TlsConfig, String)> = cfg
            .databases
            .iter()
            .filter(|(_, db)| db.capture == CaptureMode::Native)
            .map(|(name, db)| {
                let fingerprint = serde_json::to_string(db).unwrap_or_default();
                (name.clone(), (db.url.clone(), db.tls.clone(), fingerprint))
            })
            .collect();

        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        tasks.retain(|name, task| {
            let keep = desired
                .get(name)
                .is_some_and(|(_, _, fp)| *fp == task.fingerprint);
            if !keep {
                task.handle.abort();
                tracing::info!(db = %name, "native capture listener stopped");
            }
            keep
        });
        for (name, (url, tls, fingerprint)) in desired {
            if tasks.contains_key(&name) {
                continue;
            }
            let handle = tokio::spawn(run_listener(
                name.clone(),
                url,
                tls,
                Arc::clone(&self.bus),
            ));
            tasks.insert(name, ListenerTask { fingerprint, handle });
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

async fn listen_once(db: &str, url: &str, tls: &TlsConfig, bus: &RowChangeBus) -> Result<(), String> {
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
    fn parse_notification_maps_payloads_and_drops_foreign_ones() {
        let ev = parse_notification("primary", r#"{"table":"public.orders","op":"insert","n":3}"#)
            .unwrap();
        assert_eq!(ev.db, "primary");
        assert_eq!(ev.table.as_deref(), Some("public.orders"));
        assert_eq!(ev.op, Op::Insert);
        assert_eq!(ev.affected_rows, 3);
        assert!(ev.returning.is_none());

        // Someone else NOTIFYing on our channel must not become an event.
        assert!(parse_notification("primary", "not json").is_none());
        assert!(parse_notification("primary", r#"{"table":"t","op":"vacuum","n":1}"#).is_none());
        assert!(parse_notification("primary", r#"{"op":"insert","n":1}"#).is_none());
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
            listener.batch_execute("LISTEN iii_row_changed").await.unwrap();
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
        }

        let _ = drive(&mut conn, listener.batch_execute(&format!("DROP TABLE {table}"))).await;
    }
}
