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
//! user. Row identity (`returning` = primary-key values) additionally needs
//! `binlog_row_metadata=FULL`: only then does the table map carry column
//! names and key flags. Under the `MINIMAL` default the events are
//! count-only, and registration warns.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use mysql_async::binlog::events::{EventData, RowsEventData};
use mysql_async::binlog::row::BinlogRow;
use mysql_async::binlog::value::BinlogValue;
use mysql_async::consts::ColumnFlags;
use mysql_async::prelude::Queryable;
use mysql_async::{BinlogStreamRequest, Conn, Opts, OptsBuilder};
use serde_json::{Map, Value};

use super::bus::{now_ms, RowChangeBus, RowChangedEvent, KEY_CAP};
use super::sql::Op;
use crate::config::TlsConfig;
use crate::driver::mysql::my_to_row_value;
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
/// between simultaneous replicas of the same server: the handle name keeps
/// two `capture: native` databases in ONE worker apart (same-id replicas
/// evict each other and reconnect-thrash forever), and the pid keeps
/// concurrent workers on one host apart.
fn server_id(db_name: &str) -> u32 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    db_name.hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    1_000_000_000 + (hasher.finish() % 1_000_000) as u32
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
                    "the server reports no binlog position — is log_bin enabled?".to_string(),
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

/// The primary-key values of one row image as `{column: json}`, encoded
/// through the same conversion `database::query` uses for the driver. `None`
/// when nothing in the image is key-flagged: the table has no primary key, or
/// the server logs `binlog_row_metadata=MINIMAL` (no column names, no key
/// flags — the registration-time warning names the fix).
fn key_of(row: &BinlogRow) -> Option<Map<String, Value>> {
    let mut key = Map::new();
    for (i, col) in row.columns_ref().iter().enumerate() {
        if !col.flags().contains(ColumnFlags::PRI_KEY_FLAG) {
            continue;
        }
        // A JSON column cannot be a primary key; anything but a plain value
        // here is an image this code does not understand — say nothing
        // rather than invent a key.
        let BinlogValue::Value(v) = row.as_ref(i)? else {
            return None;
        };
        key.insert(
            col.name_str().into_owned(),
            my_to_row_value(v.clone()).into_json(),
        );
    }
    (!key.is_empty()).then_some(key)
}

/// One statement's rows, merged across the binlog's chunking, waiting for
/// the statement boundary that flushes it as a single event.
struct Pending {
    table: String,
    op: Op,
    /// Rows seen — exact, whatever the cap did to `keys`.
    n: u64,
    keys: Vec<Map<String, Value>>,
}

fn flush(
    pending: &mut Option<Pending>,
    db_name: &str,
    out: &tokio::sync::mpsc::UnboundedSender<RowChangedEvent>,
) {
    if let Some(p) = pending.take() {
        let has_keys = !p.keys.is_empty();
        let _ = out.send(RowChangedEvent {
            db: db_name.to_string(),
            table: Some(p.table),
            op: p.op,
            affected_rows: p.n,
            truncated: has_keys && (p.keys.len() as u64) < p.n,
            returning: has_keys.then_some(p.keys),
            at: now_ms(),
        });
    }
}

/// Keep the stream alive forever, reconnecting with capped backoff. Events
/// flow into `bus` via an unbounded channel so the decode loop never blocks
/// on subscriber dispatch.
pub(crate) async fn run_binlog(
    db_name: String,
    url: String,
    tls: TlsConfig,
    bus: Arc<RowChangeBus>,
) {
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
            BinlogStreamRequest::new(server_id(db_name))
                .with_filename(file.as_bytes())
                .with_pos(pos),
        )
        .await
        .map_err(|e| format!("{e}; {GRANT_HINT}"))?;
    tracing::info!(db = %db_name, file = %file, pos, "native capture streaming binlog");

    // One statement's rows can arrive chunked across several events; merge
    // ADJACENT same-(table, op) row events and flush on any other event —
    // every transaction ends with a non-rows event (Xid), so nothing is
    // held past its commit. TableMapEvent is a flush point too: in row
    // format every STATEMENT re-maps its table before its rows events,
    // while the chunks of one statement share a single map — so flushing
    // there yields exactly one event per statement (matching postgres)
    // without breaking chunk merging.
    let mut pending: Option<Pending> = None;

    while let Some(event) = stream.next().await {
        let event = event.map_err(|e| e.to_string())?;
        let data = match event.read_data() {
            Ok(Some(data)) => data,
            // Undecodable/unknown events still delimit statements.
            _ => {
                flush(&mut pending, db_name, out);
                continue;
            }
        };
        match data {
            EventData::RowsEvent(rows) => {
                let op = op_of(&rows);
                let Some(tme) = stream.get_tme(rows.table_id()) else {
                    // No table map — cannot attribute; drop rather than lie.
                    flush(&mut pending, db_name, out);
                    continue;
                };
                if schema.as_deref().is_some_and(|s| tme.database_name() != s) {
                    flush(&mut pending, db_name, out);
                    continue;
                }
                let table = tme.table_name().to_string();
                let decoded: Vec<_> = rows.rows(tme).collect();
                if decoded.is_empty() {
                    continue;
                }
                if pending
                    .as_ref()
                    .is_none_or(|p| p.table != table || p.op != op)
                {
                    flush(&mut pending, db_name, out);
                    pending = Some(Pending {
                        table,
                        op,
                        n: 0,
                        keys: Vec::new(),
                    });
                }
                let p = pending.as_mut().expect("a statement was just opened");
                for row in decoded {
                    // A row that failed to decode still happened — count it,
                    // as `.count()` did before keys existed.
                    p.n += 1;
                    let Ok((before, after)) = row else { continue };
                    // INSERT has only an after image. UPDATE prefers the after
                    // image so the event carries the NEW key, as RETURNING does
                    // on the statements path, and falls back to the before
                    // image when `binlog_row_image` omitted unchanged columns
                    // from it. DELETE has only a before image.
                    let key = match op {
                        Op::Insert => after.as_ref().and_then(key_of),
                        _ => after
                            .as_ref()
                            .and_then(key_of)
                            .or_else(|| before.as_ref().and_then(key_of)),
                    };
                    if let Some(key) = key {
                        if p.keys.len() < KEY_CAP {
                            p.keys.push(key);
                        }
                    }
                }
            }
            // A table map opens the next statement's rows — statement boundary.
            EventData::TableMapEvent(_) => flush(&mut pending, db_name, out),
            // Anything else (Xid, Query, Rotate, Gtid, …) ends a statement.
            _ => flush(&mut pending, db_name, out),
        }
    }
    flush(&mut pending, db_name, out);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mysql_async::consts::ColumnType;
    use mysql_async::{Column, Value as MyValue};

    fn row(cells: Vec<(&str, bool, MyValue)>) -> BinlogRow {
        let columns: Arc<[Column]> = cells
            .iter()
            .map(|(name, is_key, _)| {
                let col = Column::new(ColumnType::MYSQL_TYPE_LONGLONG).with_name(name.as_bytes());
                if *is_key {
                    col.with_flags(ColumnFlags::PRI_KEY_FLAG)
                } else {
                    col
                }
            })
            .collect::<Vec<_>>()
            .into();
        let values = cells
            .into_iter()
            .map(|(_, _, v)| Some(BinlogValue::Value(v)))
            .collect();
        BinlogRow::new(values, columns)
    }

    #[test]
    fn key_of_reads_flagged_columns_through_the_query_encoder() {
        let r = row(vec![
            ("id", true, MyValue::Int(7)),
            ("n", false, MyValue::Bytes(b"a".to_vec())),
        ]);
        assert_eq!(
            Value::Object(key_of(&r).unwrap()),
            serde_json::json!({ "id": 7 })
        );

        // Composite keys keep every flagged column; unsigned values beyond
        // i64 travel as strings, exactly as database::query returns them.
        let r = row(vec![
            ("tenant", true, MyValue::Bytes(b"acme".to_vec())),
            ("seq", true, MyValue::UInt(u64::MAX)),
            ("n", false, MyValue::Int(1)),
        ]);
        assert_eq!(
            Value::Object(key_of(&r).unwrap()),
            serde_json::json!({ "tenant": "acme", "seq": "18446744073709551615" })
        );

        // No key flags — a table without a primary key, or a server logging
        // binlog_row_metadata=MINIMAL — means no identity, not an empty one.
        let r = row(vec![("n", false, MyValue::Int(1))]);
        assert!(key_of(&r).is_none());
    }

    #[test]
    fn server_id_stays_in_range_and_separates_handles() {
        let a = server_id("primary");
        let b = server_id("analytics");
        for id in [a, b] {
            assert!((1_000_000_000..1_001_000_000).contains(&id));
        }
        // Two native handles in one worker must register as DIFFERENT
        // replicas — the server evicts duplicate server ids.
        assert_ne!(a, b);
        // Deterministic within a process: reconnects keep their identity.
        assert_eq!(a, server_id("primary"));
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
        let bulk = format!("{table}_bulk");
        let unkeyed = format!("{table}_unkeyed");
        let writer_pool = mysql_async::Pool::new(url.as_str());
        let mut writer = writer_pool.get_conn().await.unwrap();
        for t in [&table, &bulk, &unkeyed] {
            writer
                .query_drop(format!("DROP TABLE IF EXISTS {t}"))
                .await
                .unwrap();
        }
        writer
            .query_drop(format!("CREATE TABLE {table} (id INT PRIMARY KEY, n INT)"))
            .await
            .unwrap();
        writer
            .query_drop(format!(
                "CREATE TABLE {bulk} (id INT AUTO_INCREMENT PRIMARY KEY, n INT)"
            ))
            .await
            .unwrap();
        writer
            .query_drop(format!("CREATE TABLE {unkeyed} (n INT)"))
            .await
            .unwrap();
        // Keys ride the binlog only when the server logs full row metadata;
        // the e2e compose stack sets it, a stock server does not.
        let meta: Option<String> = writer
            .query_first("SELECT @@binlog_row_metadata")
            .await
            .unwrap();
        let full_metadata = meta
            .as_deref()
            .is_some_and(|m| m.eq_ignore_ascii_case("FULL"));

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
        assert_eq!(events[1].op, Op::Update);
        assert_eq!(events[1].affected_rows, 2);
        assert_eq!(events[2].op, Op::Delete);
        assert_eq!(events[2].affected_rows, 1);
        let id = |i: i64| serde_json::json!({ "id": i }).as_object().unwrap().clone();
        if full_metadata {
            assert_eq!(events[0].returning.as_ref().unwrap(), &vec![id(1), id(2)]);
            assert_eq!(events[1].returning.as_ref().unwrap(), &vec![id(1), id(2)]);
            assert_eq!(events[2].returning.as_ref().unwrap(), &vec![id(1)]);
        } else {
            eprintln!("binlog_row_metadata is not FULL: asserting count-only events");
            assert!(events.iter().all(|e| e.returning.is_none()));
        }
        assert!(events.iter().all(|e| !e.truncated));

        // The cap: 150 rows in one statement arrive as ONE event with the
        // exact count and the first KEY_CAP keys, flagged as truncated.
        let values = (0..150)
            .map(|i| format!("({i})"))
            .collect::<Vec<_>>()
            .join(", ");
        writer
            .query_drop(format!("INSERT INTO {bulk} (n) VALUES {values}"))
            .await
            .unwrap();
        // No primary key at all: today's count-only event, not truncation.
        writer
            .query_drop(format!("INSERT INTO {unkeyed} VALUES (1), (2)"))
            .await
            .unwrap();
        let mut bulk_event = None;
        let mut unkeyed_event = None;
        while bulk_event.is_none() || unkeyed_event.is_none() {
            let event = tokio::time::timeout(Duration::from_secs(10), rx.recv())
                .await
                .expect("binlog event within 10s")
                .expect("stream alive");
            if event.table.as_deref() == Some(bulk.as_str()) {
                bulk_event = Some(event);
            } else if event.table.as_deref() == Some(unkeyed.as_str()) {
                unkeyed_event = Some(event);
            }
        }
        let bulk_event = bulk_event.unwrap();
        assert_eq!(bulk_event.affected_rows, 150);
        if full_metadata {
            let keys = bulk_event.returning.as_ref().unwrap();
            assert_eq!(keys.len(), KEY_CAP);
            assert_eq!(keys[0]["id"], 1);
            assert!(bulk_event.truncated);
        } else {
            assert!(bulk_event.returning.is_none());
            assert!(!bulk_event.truncated);
        }
        let unkeyed_event = unkeyed_event.unwrap();
        assert_eq!(unkeyed_event.affected_rows, 2);
        assert!(unkeyed_event.returning.is_none());
        assert!(!unkeyed_event.truncated);

        streamer.abort();
        for t in [&table, &bulk, &unkeyed] {
            writer.query_drop(format!("DROP TABLE {t}")).await.unwrap();
        }
        drop(writer);
        let _ = writer_pool.disconnect().await;
    }
}
