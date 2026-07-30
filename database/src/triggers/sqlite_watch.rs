//! Native change capture for file-backed SQLite: triggers + changelog + watch.
//!
//! SQLite is embedded — there is no server to broadcast "someone wrote".
//! What does exist: SQL triggers fire for ANY process's writes, and their
//! inserts into a changelog table commit atomically with the write itself
//! (a rollback removes them — commit gating is free and exact). Delivery is
//! this worker draining that changelog. Wake-up is event-driven: an fs watch
//! on the database file (inotify & co. via `notify`) plus `PRAGMA
//! data_version` — which changes iff another connection committed — as the
//! cheap confirm gate, with a slow fallback tick so a missed fs event
//! degrades latency instead of dropping anything. The changelog is the
//! source of truth; the watch only decides when to look.
//!
//! Boot behavior matches the postgres path: the cursor starts at the current
//! changelog head, so writes made while no worker was running are not
//! replayed. At-most-once, a doorbell not a ledger.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use rusqlite::Connection;

use super::bus::{now_ms, RowChangedEvent};
use super::native::quote_table;
use super::sql::Op;

/// The changelog every captured table's triggers append to.
pub(crate) const CHANGELOG: &str = "_iii_row_changes";

/// How long the watcher sleeps when no fs event arrives. Pure insurance —
/// on a filesystem where the watch misses events (NFS, some overlayfs),
/// capture latency degrades to this instead of failing.
const FALLBACK_TICK: Duration = Duration::from_secs(2);

/// The database file behind a `sqlite:` url, or None for `:memory:` forms
/// (which config validation already rejects for native capture).
pub(crate) fn sqlite_file_path(url: &str) -> Option<PathBuf> {
    let path = url.strip_prefix("sqlite:").unwrap_or(url);
    let path = path.strip_prefix("file:").unwrap_or(path);
    let path = path.split('?').next().unwrap_or(path);
    if path.is_empty() || path.contains(":memory:") {
        return None;
    }
    Some(PathBuf::from(path))
}

/// DDL for one captured table: the shared changelog plus three row-level
/// triggers (SQLite has no statement-level triggers or transition tables).
/// Idempotent to reinstall; trigger names embed the table because SQLite
/// trigger names are schema-global, not per-table like postgres.
pub(crate) fn install_sql(table: &str) -> Result<String, String> {
    let target = quote_table(table)?;
    let name = |suffix: &str| {
        format!(
            "\"iii_row_changed_{suffix}_{}\"",
            table.trim().replace('"', "\"\"")
        )
    };
    let ins = name("ins");
    let upd = name("upd");
    let del = name("del");
    Ok(format!(
        r#"CREATE TABLE IF NOT EXISTS {CHANGELOG} (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  tbl TEXT NOT NULL,
  op TEXT NOT NULL
);
DROP TRIGGER IF EXISTS {ins};
CREATE TRIGGER {ins} AFTER INSERT ON {target} FOR EACH ROW
BEGIN INSERT INTO {CHANGELOG} (tbl, op) VALUES ('{tbl}', 'insert'); END;
DROP TRIGGER IF EXISTS {upd};
CREATE TRIGGER {upd} AFTER UPDATE ON {target} FOR EACH ROW
BEGIN INSERT INTO {CHANGELOG} (tbl, op) VALUES ('{tbl}', 'update'); END;
DROP TRIGGER IF EXISTS {del};
CREATE TRIGGER {del} AFTER DELETE ON {target} FOR EACH ROW
BEGIN INSERT INTO {CHANGELOG} (tbl, op) VALUES ('{tbl}', 'delete'); END;
"#,
        tbl = table.trim().replace('\'', "''"),
    ))
}

/// One coalesced run of changelog rows: (table, op, row count).
type Run = (String, Op, u64);

/// Collapse per-row changelog entries into per-run events: a 1000-row UPDATE
/// is one event with `affected_rows: 1000`, not a thousand events. Order is
/// preserved; only adjacent same-(table, op) rows merge.
pub(crate) fn coalesce(rows: Vec<(String, Op)>) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for (tbl, op) in rows {
        match out.last_mut() {
            Some((last_tbl, last_op, n)) if *last_tbl == tbl && *last_op == op => *n += 1,
            _ => out.push((tbl, op, 1)),
        }
    }
    out
}

fn parse_op(s: &str) -> Option<Op> {
    match s {
        "insert" => Some(Op::Insert),
        "update" => Some(Op::Update),
        "delete" => Some(Op::Delete),
        _ => None,
    }
}

/// Read everything past `cursor`, in id order. Returns the coalesced runs
/// and the new cursor. A missing changelog table (no binding installed DDL
/// yet) is an empty result, not an error.
fn drain(conn: &Connection, cursor: i64) -> rusqlite::Result<(Vec<Run>, i64)> {
    let mut stmt = match conn.prepare(&format!(
        "SELECT id, tbl, op FROM {CHANGELOG} WHERE id > ?1 ORDER BY id"
    )) {
        Ok(stmt) => stmt,
        Err(e) if e.to_string().contains("no such table") => return Ok((Vec::new(), cursor)),
        Err(e) => return Err(e),
    };
    let mut rows = stmt.query([cursor])?;
    let mut latest = cursor;
    let mut raw: Vec<(String, Op)> = Vec::new();
    while let Some(row) = rows.next()? {
        latest = row.get(0)?;
        let tbl: String = row.get(1)?;
        let op: String = row.get(2)?;
        // Unknown ops (a future schema writing richer rows) are skipped, not
        // fatal — the cursor still advances past them.
        if let Some(op) = parse_op(&op) {
            raw.push((tbl, op));
        }
    }
    Ok((coalesce(raw), latest))
}

/// The watcher thread body: one dedicated connection (the pool cannot serve
/// this — `data_version` is per-connection), an fs watch for wake-up, drain
/// on every wake. Each event goes to `on_event`; returning `false` from it
/// ends the watcher (the bus side is gone). Returns when `stop` is set.
pub(crate) fn run_watcher(
    db_name: &str,
    path: &Path,
    stop: &AtomicBool,
    mut on_event: impl FnMut(RowChangedEvent) -> bool,
) {
    let conn = match Connection::open(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(db = %db_name, error = %e, "sqlite watcher could not open database");
            return;
        }
    };
    let _ = conn.busy_timeout(Duration::from_secs(5));
    // The changelog may not exist yet (first binding not registered); create
    // it here too so MAX(id) and data_version have something to run against.
    if let Err(e) = conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {CHANGELOG} (id INTEGER PRIMARY KEY AUTOINCREMENT, tbl TEXT NOT NULL, op TEXT NOT NULL)"
    )) {
        tracing::warn!(db = %db_name, error = %e, "sqlite watcher could not ensure changelog");
        return;
    }

    // Skip history: only changes committed from now on are announced.
    let mut cursor: i64 = conn
        .query_row(
            &format!("SELECT COALESCE(MAX(id), 0) FROM {CHANGELOG}"),
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // Watch the parent directory, filtered to this database's files — the
    // -wal/-journal siblings appear and disappear (checkpoints), so watching
    // the paths themselves would race their recreation.
    let (wake_tx, wake_rx) = mpsc::channel::<()>();
    let file_prefix = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let mut watcher = {
        let wake = wake_tx.clone();
        let prefix = file_prefix.clone();
        notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            let Ok(event) = res else { return };
            let relevant = event.paths.iter().any(|p| match (&prefix, p.file_name()) {
                (Some(prefix), Some(name)) => name.to_string_lossy().starts_with(prefix.as_str()),
                _ => true,
            });
            if relevant {
                let _ = wake.send(());
            }
        })
        .ok()
    };
    let watch_dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let watching = match (&mut watcher, watch_dir) {
        (Some(w), Some(dir)) => w.watch(dir, RecursiveMode::NonRecursive).is_ok(),
        (Some(w), None) => w.watch(Path::new("."), RecursiveMode::NonRecursive).is_ok(),
        _ => false,
    };
    if !watching {
        tracing::warn!(
            db = %db_name,
            "sqlite watcher running without fs events; falling back to {}s polling",
            FALLBACK_TICK.as_secs()
        );
    }
    tracing::info!(db = %db_name, path = %path.display(), fs_events = watching, "native capture watching");

    let mut data_version: i64 = pragma_data_version(&conn).unwrap_or(0);
    let mut first = true;
    while !stop.load(Ordering::Relaxed) {
        if !first {
            // Block until something happens (or the fallback tick), then
            // collapse any burst of fs events into one drain pass.
            let _ = wake_rx.recv_timeout(FALLBACK_TICK);
            while wake_rx.try_recv().is_ok() {}
        }
        first = false;
        if stop.load(Ordering::Relaxed) {
            break;
        }

        // `data_version` changes iff some OTHER connection committed —
        // exactly the writes this watcher exists to see. Unchanged → the fs
        // event was noise (reads, -shm traffic) and the drain is skipped.
        let version = match pragma_data_version(&conn) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(db = %db_name, error = %e, "sqlite watcher data_version failed");
                continue;
            }
        };
        if version == data_version {
            continue;
        }
        data_version = version;

        match drain(&conn, cursor) {
            Ok((runs, new_cursor)) => {
                for (tbl, op, n) in runs {
                    let event = RowChangedEvent {
                        db: db_name.to_string(),
                        table: Some(tbl),
                        op,
                        affected_rows: n,
                        returning: None,
                        at: now_ms(),
                    };
                    if !on_event(event) {
                        return; // bus side gone — shutting down
                    }
                }
                if new_cursor != cursor {
                    cursor = new_cursor;
                    // ponytail: GC assumes this worker is the only watcher of
                    // this file; two workers watching one db would starve each
                    // other. Per-watcher cursor rows if that ever exists.
                    let _ =
                        conn.execute(&format!("DELETE FROM {CHANGELOG} WHERE id <= ?1"), [cursor]);
                }
            }
            Err(e) => {
                tracing::warn!(db = %db_name, error = %e, "sqlite watcher drain failed");
            }
        }
    }
}

fn pragma_data_version(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("PRAGMA data_version", [], |r| r.get(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn sqlite_file_path_strips_schemes_and_rejects_memory() {
        assert_eq!(
            sqlite_file_path("sqlite:./data/iii.db"),
            Some(PathBuf::from("./data/iii.db"))
        );
        assert_eq!(
            sqlite_file_path("sqlite:file:./x.db?mode=rwc"),
            Some(PathBuf::from("./x.db"))
        );
        assert_eq!(sqlite_file_path("sqlite::memory:"), None);
        assert_eq!(sqlite_file_path("sqlite:file::memory:?cache=shared"), None);
    }

    #[test]
    fn install_sql_quotes_and_embeds_table_names() {
        let sql = install_sql("orders").unwrap();
        assert!(sql.contains("AFTER INSERT ON \"orders\""));
        assert!(sql.contains("\"iii_row_changed_del_orders\""));
        assert!(sql.contains("VALUES ('orders', 'update')"));
        assert!(install_sql("  ").is_err());

        // Hostile names stay inside identifier quotes and string literals.
        let evil = "t'; DROP TABLE x; --";
        let sql = install_sql(evil).unwrap();
        assert!(sql.contains("VALUES ('t''; DROP TABLE x; --', 'insert')"));
    }

    #[test]
    fn coalesce_merges_adjacent_runs_only() {
        let rows = vec![
            ("a".to_string(), Op::Insert),
            ("a".to_string(), Op::Insert),
            ("a".to_string(), Op::Update),
            ("b".to_string(), Op::Update),
            ("a".to_string(), Op::Insert),
        ];
        assert_eq!(
            coalesce(rows),
            vec![
                ("a".to_string(), Op::Insert, 2),
                ("a".to_string(), Op::Update, 1),
                ("b".to_string(), Op::Update, 1),
                ("a".to_string(), Op::Insert, 1),
            ]
        );
        assert!(coalesce(Vec::new()).is_empty());
    }

    /// The cross-process claim, sqlite edition: a write on a completely
    /// separate connection (stand-in for another process) reaches the
    /// watcher through triggers + changelog + fs wake-up.
    #[test]
    fn watcher_hears_writes_from_another_connection() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("watched.db");

        // "External" client: creates the table and installs capture DDL the
        // way the handler does, then writes.
        let external = Connection::open(&db_path).unwrap();
        external
            .execute_batch("CREATE TABLE items (id INTEGER PRIMARY KEY, n INT)")
            .unwrap();
        external
            .execute_batch(&install_sql("items").unwrap())
            .unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let thread = {
            let stop = Arc::clone(&stop);
            let path = db_path.clone();
            std::thread::spawn(move || {
                run_watcher("primary", &path, &stop, move |ev| tx.send(ev).is_ok())
            })
        };

        // Wait for the watcher to establish its baseline (it logs first,
        // then reads MAX(id)); a short settle keeps the test deterministic
        // without exposing internals.
        std::thread::sleep(Duration::from_millis(300));

        external
            .execute_batch(
                "INSERT INTO items (n) VALUES (1), (2), (3); \
                 UPDATE items SET n = n + 1; \
                 DELETE FROM items WHERE n > 2;",
            )
            .unwrap();

        let mut events = Vec::new();
        while events.len() < 3 {
            events.push(
                rx.recv_timeout(Duration::from_secs(10))
                    .expect("watcher event within 10s"),
            );
        }
        assert_eq!(events[0].op, Op::Insert);
        assert_eq!(events[0].affected_rows, 3);
        assert_eq!(events[0].table.as_deref(), Some("items"));
        assert_eq!(events[0].db, "primary");
        assert_eq!(events[1].op, Op::Update);
        assert_eq!(events[1].affected_rows, 3);
        assert_eq!(events[2].op, Op::Delete);
        assert_eq!(events[2].affected_rows, 2);

        // A rolled-back write is invisible: the changelog rows die with it.
        external
            .execute_batch("BEGIN; INSERT INTO items (n) VALUES (9); ROLLBACK;")
            .unwrap();
        // And a zero-row statement appends nothing.
        external
            .execute_batch("UPDATE items SET n = 0 WHERE n = -777")
            .unwrap();
        assert!(
            rx.recv_timeout(Duration::from_millis(600)).is_err(),
            "rolled-back / zero-row writes must not produce events"
        );

        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap();
    }

    /// History from before the watcher started is skipped (at-most-once,
    /// postgres parity) — and the GC keeps the changelog from growing.
    #[test]
    fn watcher_skips_history_and_gcs_the_changelog() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("watched.db");
        let external = Connection::open(&db_path).unwrap();
        external
            .execute_batch("CREATE TABLE items (id INTEGER PRIMARY KEY, n INT)")
            .unwrap();
        external
            .execute_batch(&install_sql("items").unwrap())
            .unwrap();
        // Rows written before any watcher exists.
        external
            .execute_batch("INSERT INTO items (n) VALUES (1), (2)")
            .unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let thread = {
            let stop = Arc::clone(&stop);
            let path = db_path.clone();
            std::thread::spawn(move || {
                run_watcher("primary", &path, &stop, move |ev| tx.send(ev).is_ok())
            })
        };
        std::thread::sleep(Duration::from_millis(300));

        // Nothing replayed…
        assert!(rx.recv_timeout(Duration::from_millis(400)).is_err());
        // …but a new write arrives, and afterwards the changelog is drained.
        external.execute_batch("DELETE FROM items").unwrap();
        let ev = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(ev.op, Op::Delete);
        assert_eq!(ev.affected_rows, 2);

        // GC happened: nothing at or below the cursor survives. Retry
        // briefly — the DELETE runs just after the event is sent.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let left: i64 = external
                .query_row(&format!("SELECT count(*) FROM {CHANGELOG}"), [], |r| {
                    r.get(0)
                })
                .unwrap();
            if left == 0 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "changelog was not GC'd, {left} rows left"
            );
            std::thread::sleep(Duration::from_millis(50));
        }

        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap();
    }
}
