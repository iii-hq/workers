//! Native change capture for MySQL: the binlog replication stream.
//!
//! MySQL has no LISTEN/NOTIFY, but it has something stronger — the binary
//! log every replica reads. The worker connects as a replica (one dedicated
//! connection per `capture: native` database), starts at the server's
//! current position, and decodes row events into `database::row-changed`
//! events. Nothing is installed in the user's schema: no triggers, no
//! changelog table, no DDL at binding registration.
//!
//! Semantics match the other drivers: only committed writes appear in the
//! binlog (row events are flushed at commit), so commit gating is free; a
//! reconnect re-snapshots the position, so events raised while the stream
//! was down are lost — at-most-once, a doorbell not a ledger.
//!
//! Server prerequisites, checked loudly at binding registration:
//! `log_bin=ON`, `binlog_format=ROW` (both 8.x defaults), and the
//! `REPLICATION SLAVE, REPLICATION CLIENT` global grants for the worker's
//! user.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use mysql_async::binlog::events::{EventData, RowsEventData};
use mysql_async::prelude::Queryable;
use mysql_async::{BinlogStreamRequest, Conn, Opts, OptsBuilder};

use super::bus::{now_ms, RowChangeBus, RowChangedEvent};
use super::sql::Op;
use crate::config::TlsConfig;
use crate::pool::tls::make_mysql_ssl_opts;

/// Statements that report the current binlog file/position. 8.2 renamed the
/// classic one; try newest first, fall back on "unknown statement".
const POSITION_STATEMENTS: [&str; 2] = ["SHOW BINARY LOG STATUS", "SHOW MASTER STATUS"];

/// The GRANT hint quoted in every privilege-shaped failure. Kept in one
/// place so registration errors and stream errors say the same thing.
pub(crate) const GRANT_HINT: &str =
    "the worker's user needs: GRANT REPLICATION SLAVE, REPLICATION CLIENT ON *.* TO '<user>'@'%'";

/// A server id for COM_REGISTER_SLAVE that stays clear of the low range
/// operators typically hand-assign to real replicas. Collisions only matter
/// between simultaneous replicas of the same server; pid keeps concurrent
/// workers on one host apart.
fn server_id() -> u32 {
    1_000_000_000 + (std::process::id() % 1_000_000)
}

pub(crate) fn build_opts(url: &str, tls: &TlsConfig) -> Result<Opts, String> {
    let base = Opts::from_url(url).map_err(|_| "invalid mysql url".to_string())?;
    let mut builder = OptsBuilder::from_opts(base);
    if let Some(ssl) = make_mysql_ssl_opts(tls).map_err(|e| format!("{e:?}"))? {
        builder = builder.ssl_opts(ssl);
    }
    Ok(builder.into())
}

/// The current binlog (file, position), or an actionable error. Requires
/// REPLICATION CLIENT — this doubles as the registration-time privilege
/// probe.
pub(crate) async fn binlog_position(conn: &mut Conn) -> Result<(String, u64), String> {
    let mut last_err = String::new();
    for sql in POSITION_STATEMENTS {
        match conn.query_first::<mysql_async::Row, _>(sql).await {
            Ok(Some(row)) => {
                let file: Option<String> = row.get(0);
                let pos: Option<u64> = row.get(1);
                match (file, pos) {
                    (Some(file), Some(pos)) => return Ok((file, pos)),
                    _ => return Err(format!("`{sql}` returned an unreadable row")),
                }
            }
            Ok(None) => {
                return Err(
                    "the server reports no binlog position — is log_bin enabled?".to_string()
                )
            }
            Err(e) => {
                let msg = e.to_string();
                // 8.2 removed SHOW MASTER STATUS' predecessor and older
                // servers don't know the new form — try the other spelling.
                if msg.contains("error in your SQL syntax") || msg.contains("Unknown") {
                    last_err = msg;
                    continue;
                }
                return Err(format!("{msg}; {GRANT_HINT}"));
            }
        }
    }
    Err(format!("{last_err}; {GRANT_HINT}"))
}

fn op_of(rows: &RowsEventData<'_>) -> Op {
    match rows {
        RowsEventData::WriteRowsEvent(_) | RowsEventData::WriteRowsEventV1(_) => Op::Insert,
        RowsEventData::UpdateRowsEvent(_)
        | RowsEventData::UpdateRowsEventV1(_)
        | RowsEventData::PartialUpdateRowsEvent(_) => Op::Update,
        RowsEventData::DeleteRowsEvent(_) | RowsEventData::DeleteRowsEventV1(_) => Op::Delete,
    }
}

/// Keep the stream alive forever, reconnecting with capped backoff. Events
/// flow into `bus` via an unbounded channel so the decode loop never blocks
/// on subscriber dispatch.
pub(crate) async fn run_binlog(db_name: String, url: String, tls: TlsConfig, bus: Arc<RowChangeBus>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<RowChangedEvent>();
    // The forwarder dies with this task: aborting run_binlog drops `tx`,
    // recv() yields None, and the spawned task returns.
    let bus_for_forwarder = Arc::clone(&bus);
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            bus_for_forwarder.emit_event(event).await;
        }
    });

    let mut delay = Duration::from_secs(1);
    loop {
        match stream_once(&db_name, &url, &tls, &tx).await {
            Ok(()) => {
                tracing::warn!(db = %db_name, "binlog stream ended; reconnecting");
                delay = Duration::from_secs(1);
            }
            Err(e) => {
                tracing::warn!(db = %db_name, error = %e, "binlog capture error; retrying");
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(30));
    }
}

/// One replica session: snapshot position, stream, decode, emit.
async fn stream_once(
    db_name: &str,
    url: &str,
    tls: &TlsConfig,
    out: &tokio::sync::mpsc::UnboundedSender<RowChangedEvent>,
) -> Result<(), String> {
    let opts = build_opts(url, tls)?;
    // Events for OTHER databases on the same server are not this handle's
    // business — a binding names a db handle, and the handle names a schema.
    let schema = opts.db_name().map(str::to_string);
    let mut conn = Conn::new(opts).await.map_err(|e| e.to_string())?;
    let (file, pos) = binlog_position(&mut conn).await?;
    let mut stream = conn
        .get_binlog_stream(
            BinlogStreamRequest::new(server_id())
                .with_filename(file.as_bytes())
                .with_pos(pos),
        )
        .await
        .map_err(|e| format!("{e}; {GRANT_HINT}"))?;
    tracing::info!(db = %db_name, file = %file, pos, "native capture streaming binlog");

    // One statement's rows can arrive chunked across several events; merge
    // ADJACENT same-(table, op) row events and flush on any other event —
    // every transaction ends with a non-rows event (Xid), so nothing is
    // held past its commit.
    let mut pending: Option<(String, Op, u64)> = None;
    let flush = |pending: &mut Option<(String, Op, u64)>| {
        if let Some((table, op, n)) = pending.take() {
            let _ = out.send(RowChangedEvent {
                db: db_name.to_string(),
                table: Some(table),
                op,
                affected_rows: n,
                returning: None,
                at: now_ms(),
            });
        }
    };

    while let Some(event) = stream.next().await {
        let event = event.map_err(|e| e.to_string())?;
        let data = match event.read_data() {
            Ok(Some(data)) => data,
            // Undecodable/unknown events still delimit statements.
            _ => {
                flush(&mut pending);
                continue;
            }
        };
        match data {
            EventData::RowsEvent(rows) => {
                let op = op_of(&rows);
                let Some(tme) = stream.get_tme(rows.table_id()) else {
                    // No table map — cannot attribute; drop rather than lie.
                    flush(&mut pending);
                    continue;
                };
                if schema.as_deref().is_some_and(|s| tme.database_name() != s) {
                    flush(&mut pending);
                    continue;
                }
                let table = tme.table_name().to_string();
                let n = rows.rows(tme).count() as u64;
                if n == 0 {
                    continue;
                }
                match &mut pending {
                    Some((last_table, last_op, total)) if *last_table == table && *last_op == op => {
                        *total += n;
                    }
                    slot => {
                        flush(slot);
                        *slot = Some((table, op, n));
                    }
                }
            }
            // Table maps prefix their rows events — not a boundary.
            EventData::TableMapEvent(_) => {}
            // Anything else (Xid, Query, Rotate, Gtid, …) ends a statement.
            _ => flush(&mut pending),
        }
    }
    flush(&mut pending);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_id_stays_in_the_reserved_range() {
        let id = server_id();
        assert!((1_000_000_000..1_001_000_000).contains(&id));
    }

    /// The cross-client claim, mysql edition: writes from a plain client
    /// connection arrive through the replica stream. Requires
    /// TEST_MYSQL_URL and replication grants for that user; fails (not
    /// skips) without the grants — the error names the exact GRANT.
    #[tokio::test(flavor = "multi_thread")]
    async fn binlog_capture_hears_writes_from_another_connection() {
        let Some(url) = std::env::var("TEST_MYSQL_URL").ok() else {
            eprintln!("skipping: TEST_MYSQL_URL not set");
            return;
        };
        let tls = TlsConfig {
            mode: crate::config::TlsMode::Disable,
            ..Default::default()
        };

        let table = format!("iii_binlog_capture_{}", std::process::id());
        let writer_pool = mysql_async::Pool::new(url.as_str());
        let mut writer = writer_pool.get_conn().await.unwrap();
        writer
            .query_drop(format!("DROP TABLE IF EXISTS {table}"))
            .await
            .unwrap();
        writer
            .query_drop(format!("CREATE TABLE {table} (id INT PRIMARY KEY, n INT)"))
            .await
            .unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let streamer = {
            let url = url.clone();
            let db = "primary".to_string();
            tokio::spawn(async move {
                if let Err(e) = stream_once(&db, &url, &tls, &tx).await {
                    panic!("stream_once failed: {e}");
                }
            })
        };
        // Give the replica session a moment to snapshot + attach.
        tokio::time::sleep(Duration::from_millis(500)).await;

        writer
            .query_drop(format!("INSERT INTO {table} VALUES (1, 10), (2, 20)"))
            .await
            .unwrap();
        writer
            .query_drop(format!("UPDATE {table} SET n = n + 1"))
            .await
            .unwrap();
        writer
            .query_drop(format!("DELETE FROM {table} WHERE id = 1"))
            .await
            .unwrap();

        let mut events = Vec::new();
        while events.len() < 3 {
            let event = tokio::time::timeout(Duration::from_secs(10), rx.recv())
                .await
                .expect("binlog event within 10s")
                .expect("stream alive");
            // The server may interleave writes from other databases/tests;
            // keep only our table's events.
            if event.table.as_deref() == Some(table.as_str()) {
                events.push(event);
            }
        }
        assert_eq!(events[0].op, Op::Insert);
        assert_eq!(events[0].affected_rows, 2);
        assert_eq!(events[0].db, "primary");
        assert!(events[0].returning.is_none());
        assert_eq!(events[1].op, Op::Update);
        assert_eq!(events[1].affected_rows, 2);
        assert_eq!(events[2].op, Op::Delete);
        assert_eq!(events[2].affected_rows, 1);

        streamer.abort();
        writer
            .query_drop(format!("DROP TABLE {table}"))
            .await
            .unwrap();
        drop(writer);
        let _ = writer_pool.disconnect().await;
    }
}
