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
use serde_json::{Map, Value};

use super::bus::{now_ms, RowChangedEvent, KEY_CAP};
use super::native::quote_table;
use super::sql::Op;
use crate::value::RowValue;

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
///
/// Each trigger records the row's primary key (`pk`, discovered by
/// [`pk_columns`] at install time) as a JSON object beside the table and op,
/// so the drained event can say WHICH rows changed: `NEW` for inserts and
/// updates (the new key, as RETURNING reports it), `OLD` for deletes. A table
/// without a declared primary key records `NULL` — its bare rowid is not an
/// identity (VACUUM renumbers it). Blob keys cannot ride inside
/// `json_object` directly, so they are wrapped as `{"$blob": hex}` and
/// decoded back to base64 on drain.
///
/// The trigger runs inside the WRITER's process, so `json_object` has to
/// exist there: any client on libsqlite ≥ 3.38 (JSON built in). A client
/// compiled without JSON1 fails its own INSERT with `no such function`.
///
/// Two properties matter here:
/// * The whole script runs inside ONE explicit transaction. `execute_batch`
///   autocommits per statement, so without it a reinstall (second binding on
///   the same table) would open a window with the triggers dropped — an
///   external write landing there would be lost silently.
/// * The trigger-name suffix and the recorded `tbl` value are lowercased.
///   SQLite resolves identifiers case-insensitively, so bindings spelled in
///   different cases must converge on the SAME trigger set and the same
///   changelog spelling — never a second set double-logging every write.
pub(crate) fn install_sql(table: &str, pk: &[String]) -> Result<String, String> {
    let target = quote_table(table)?;
    let spelling = table.trim().to_lowercase();
    let name = |suffix: &str| {
        format!(
            "\"iii_row_changed_{suffix}_{}\"",
            spelling.replace('"', "\"\"")
        )
    };
    let ins = name("ins");
    let upd = name("upd");
    let del = name("del");
    let key_new = key_expr("NEW", pk);
    let key_old = key_expr("OLD", pk);
    Ok(format!(
        r#"BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS {CHANGELOG} (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  tbl TEXT NOT NULL,
  op TEXT NOT NULL,
  key TEXT
);
DROP TRIGGER IF EXISTS {ins};
CREATE TRIGGER {ins} AFTER INSERT ON {target} FOR EACH ROW
BEGIN INSERT INTO {CHANGELOG} (tbl, op, key) VALUES ('{tbl}', 'insert', {key_new}); END;
DROP TRIGGER IF EXISTS {upd};
CREATE TRIGGER {upd} AFTER UPDATE ON {target} FOR EACH ROW
BEGIN INSERT INTO {CHANGELOG} (tbl, op, key) VALUES ('{tbl}', 'update', {key_new}); END;
DROP TRIGGER IF EXISTS {del};
CREATE TRIGGER {del} AFTER DELETE ON {target} FOR EACH ROW
BEGIN INSERT INTO {CHANGELOG} (tbl, op, key) VALUES ('{tbl}', 'delete', {key_old}); END;
COMMIT;
"#,
        tbl = spelling.replace('\'', "''"),
    ))
}

/// The JSON-object expression a trigger records as the row's key, or `NULL`
/// for a table without a primary key. `row` is `NEW` or `OLD`.
fn key_expr(row: &str, pk: &[String]) -> String {
    if pk.is_empty() {
        return "NULL".into();
    }
    let parts: Vec<String> = pk
        .iter()
        .map(|col| {
            let ident = format!("{row}.\"{}\"", col.replace('"', "\"\""));
            let label = col.replace('\'', "''");
            format!(
                "'{label}', CASE WHEN typeof({ident}) = 'blob' \
                 THEN json(json_object('$blob', hex({ident}))) ELSE {ident} END"
            )
        })
        .collect();
    format!("json_object({})", parts.join(", "))
}

/// Create the changelog if missing and bring an older one up to the current
/// shape (the `key` column arrived after the first release). Must run before
/// [`install_sql`] lands: a trigger writing `key` into a changelog without
/// that column would fail the WRITER's statement, not ours.
pub(crate) fn ensure_changelog(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {CHANGELOG} (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         tbl TEXT NOT NULL, op TEXT NOT NULL, key TEXT)"
    ))?;
    // ADD COLUMN is O(1) in sqlite (no table rewrite); "duplicate column" is
    // the steady state after the first upgrade.
    match conn.execute_batch(&format!("ALTER TABLE {CHANGELOG} ADD COLUMN key TEXT")) {
        Ok(()) => Ok(()),
        Err(e) if e.to_string().contains("duplicate column") => Ok(()),
        Err(e) => Err(e),
    }
}

/// The declared primary-key columns of `table`, in key order; empty for a
/// table without one. The name is taken bare: `pragma_table_info` takes it
/// as a value, and a schema qualifier is meaningless here (a trigger cannot
/// write across attached databases anyway).
pub(crate) fn pk_columns(conn: &Connection, table: &str) -> rusqlite::Result<Vec<String>> {
    let last = table.trim().rsplit('.').next().unwrap_or(table).trim();
    let bare = last
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .map(|s| s.replace("\"\"", "\""))
        .unwrap_or_else(|| last.to_string());
    let mut stmt =
        conn.prepare("SELECT name FROM pragma_table_info(?1) WHERE pk > 0 ORDER BY pk")?;
    let names = stmt.query_map([bare], |r| r.get::<_, String>(0))?;
    names.collect()
}

/// One coalesced run of changelog rows: adjacent same-(table, op) entries,
/// with the keys of the first `KEY_CAP` rows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Run {
    pub table: String,
    pub op: Op,
    /// Rows in the run — exact, whatever the cap did to `keys`.
    pub n: u64,
    pub keys: Vec<Map<String, Value>>,
    /// Some rows had a key but not all made it into `keys`: the cap, or a
    /// run straddling a reinstall from a version that recorded no keys.
    pub truncated: bool,
}

/// One changelog row as drained: table, op, and its decoded key if any.
type RawChange = (String, Op, Option<Map<String, Value>>);

/// Collapse per-row changelog entries into per-run events: a 1000-row UPDATE
/// is one event with `affected_rows: 1000`, not a thousand events. Order is
/// preserved; only adjacent same-(table, op) rows merge.
pub(crate) fn coalesce(rows: Vec<RawChange>) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for (tbl, op, key) in rows {
        if out
            .last()
            .is_none_or(|last| last.table != tbl || last.op != op)
        {
            out.push(Run {
                table: tbl,
                op,
                n: 0,
                keys: Vec::new(),
                truncated: false,
            });
        }
        let run = out.last_mut().expect("a run was just pushed");
        run.n += 1;
        if let Some(key) = key {
            if run.keys.len() < KEY_CAP {
                run.keys.push(key);
            }
        }
    }
    for run in &mut out {
        run.truncated = !run.keys.is_empty() && (run.keys.len() as u64) < run.n;
    }
    out
}

/// A changelog `key` cell back into the event's row object. `{"$blob": hex}`
/// wrappers become base64 strings, matching how `database::query` encodes
/// bytes. Anything unparseable counts as "no key" rather than failing the
/// drain.
fn parse_key(text: &str) -> Option<Map<String, Value>> {
    let Value::Object(mut key) = serde_json::from_str(text).ok()? else {
        return None;
    };
    for v in key.values_mut() {
        let hex = match v {
            Value::Object(o) if o.len() == 1 => {
                o.get("$blob").and_then(Value::as_str).map(str::to_string)
            }
            _ => None,
        };
        if let Some(hex) = hex {
            *v = RowValue::Bytes(decode_hex(&hex)?).into_json();
        }
    }
    Some(key)
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
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
        "SELECT id, tbl, op, key FROM {CHANGELOG} WHERE id > ?1 ORDER BY id"
    )) {
        Ok(stmt) => stmt,
        Err(e) if e.to_string().contains("no such table") => return Ok((Vec::new(), cursor)),
        Err(e) => return Err(e),
    };
    let mut rows = stmt.query([cursor])?;
    let mut latest = cursor;
    let mut raw: Vec<RawChange> = Vec::new();
    while let Some(row) = rows.next()? {
        latest = row.get(0)?;
        let tbl: String = row.get(1)?;
        let op: String = row.get(2)?;
        let key: Option<String> = row.get(3)?;
        // Unknown ops (a future schema writing richer rows) are skipped, not
        // fatal — the cursor still advances past them.
        if let Some(op) = parse_op(&op) {
            raw.push((tbl, op, key.as_deref().and_then(parse_key)));
        }
    }
    Ok((coalesce(raw), latest))
}

/// The watcher thread body: one dedicated connection (the pool cannot serve
/// this — `data_version` is per-connection), an fs watch for wake-up, drain
/// on every wake. Each event goes to `on_event`; returning `false` from it
/// ends the watcher (the bus side is gone). Returns when `stop` is set —
/// promptly, because the spawner holds the `wake` sender and pokes it after
/// setting the flag; without that poke a stopped watcher would linger for up
/// to [`FALLBACK_TICK`], overlapping its replacement on the same changelog.
pub(crate) fn run_watcher(
    db_name: &str,
    path: &Path,
    stop: &AtomicBool,
    wake_tx: mpsc::Sender<()>,
    wake_rx: &mpsc::Receiver<()>,
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
    if let Err(e) = ensure_changelog(&conn) {
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
                for run in runs {
                    let has_keys = !run.keys.is_empty();
                    let event = RowChangedEvent {
                        db: db_name.to_string(),
                        table: Some(run.table),
                        op: run.op,
                        affected_rows: run.n,
                        returning: has_keys.then_some(run.keys),
                        truncated: run.truncated,
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
        let sql = install_sql("orders", &[]).unwrap();
        assert!(sql.contains("AFTER INSERT ON \"orders\""));
        assert!(sql.contains("\"iii_row_changed_del_orders\""));
        // No primary key: the key cell is NULL, the event stays count-only.
        assert!(sql.contains("VALUES ('orders', 'update', NULL)"));
        // Reinstall must be atomic: execute_batch autocommits per statement,
        // so the script carries its own transaction.
        assert!(sql.starts_with("BEGIN IMMEDIATE;"));
        assert!(sql.trim_end().ends_with("COMMIT;"));
        assert!(install_sql("  ", &[]).is_err());

        // Hostile names stay inside identifier quotes and string literals
        // (the recorded spelling is lowercased along with everything else),
        // and so do hostile key column names.
        let evil = "t'; DROP TABLE x; --";
        let sql = install_sql(evil, &[]).unwrap();
        assert!(sql.contains("VALUES ('t''; drop table x; --', 'insert', NULL)"));
        let sql = install_sql("t", &["k'; DROP TABLE x; --\"".into()]).unwrap();
        assert!(
            sql.contains(r#"json_object('k''; DROP TABLE x; --"', "#),
            "{sql}"
        );
        assert!(sql.contains(r#"NEW."k'; DROP TABLE x; --"""#), "{sql}");
    }

    /// The recorded key is the row's primary key as JSON, encoded like
    /// `database::query` encodes the same column once drained: integers as
    /// numbers, text as strings, blobs as base64 (via the `$blob` wrapper),
    /// every column of a composite key, no key at all for a table without a
    /// declared primary key.
    #[test]
    fn install_sql_records_primary_key_json() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE a (id INTEGER PRIMARY KEY, n INT); \
             CREATE TABLE c (x INT, y TEXT, n INT, PRIMARY KEY (y, x)); \
             CREATE TABLE bl (id BLOB PRIMARY KEY, n INT); \
             CREATE TABLE nk (n INT);",
        )
        .unwrap();
        for table in ["a", "c", "bl", "nk"] {
            let pk = pk_columns(&conn, table).unwrap();
            conn.execute_batch(&install_sql(table, &pk).unwrap())
                .unwrap();
        }
        conn.execute_batch(
            "INSERT INTO a (n) VALUES (1); \
             UPDATE a SET n = 2; \
             DELETE FROM a; \
             INSERT INTO c VALUES (1, 'x', 0); \
             INSERT INTO bl VALUES (X'0102', 0); \
             INSERT INTO nk VALUES (0);",
        )
        .unwrap();

        let rows: Vec<(String, String, Option<String>)> = conn
            .prepare(&format!("SELECT tbl, op, key FROM {CHANGELOG} ORDER BY id"))
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let key = |i: usize| rows[i].2.as_deref().and_then(parse_key);
        assert_eq!(rows[0].1, "insert");
        assert_eq!(
            key(0).unwrap(),
            serde_json::json!({ "id": 1 }).as_object().unwrap().clone()
        );
        assert_eq!(rows[1].1, "update");
        assert_eq!(key(1).unwrap()["id"], 1);
        assert_eq!(rows[2].1, "delete");
        assert_eq!(key(2).unwrap()["id"], 1);
        // Composite: every key column, whatever the PRIMARY KEY order.
        assert_eq!(
            serde_json::Value::Object(key(3).unwrap()),
            serde_json::json!({ "y": "x", "x": 1 })
        );
        // Blob: stored wrapped, drained as base64 — what database::query
        // returns for a BLOB cell.
        assert_eq!(rows[4].2.as_deref(), Some(r#"{"id":{"$blob":"0102"}}"#));
        assert_eq!(key(4).unwrap()["id"], "AQI=");
        // No primary key: NULL, not a rowid.
        assert!(rows[5].2.is_none());
        assert!(key(5).is_none());
    }

    #[test]
    fn pk_columns_discovers_declared_keys() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE a (id INTEGER PRIMARY KEY, n INT); \
             CREATE TABLE c (x INT, y TEXT, PRIMARY KEY (y, x)); \
             CREATE TABLE w (k TEXT PRIMARY KEY, v INT) WITHOUT ROWID; \
             CREATE TABLE nk (n INT);",
        )
        .unwrap();
        assert_eq!(pk_columns(&conn, "a").unwrap(), vec!["id"]);
        assert_eq!(pk_columns(&conn, "c").unwrap(), vec!["y", "x"]);
        assert_eq!(pk_columns(&conn, "w").unwrap(), vec!["k"]);
        assert!(pk_columns(&conn, "nk").unwrap().is_empty());
        // Spelled the way a binding might spell it: cased, quoted, qualified.
        assert_eq!(pk_columns(&conn, "A").unwrap(), vec!["id"]);
        assert_eq!(pk_columns(&conn, "\"a\"").unwrap(), vec!["id"]);
        assert_eq!(pk_columns(&conn, "main.a").unwrap(), vec!["id"]);
        // Unknown table: no columns, no error — CREATE TRIGGER reports it.
        assert!(pk_columns(&conn, "missing").unwrap().is_empty());
    }

    /// A changelog created by an older worker has no `key` column; the new
    /// triggers write one. Upgrading in place, twice, must be a no-op the
    /// second time.
    #[test]
    fn ensure_changelog_adds_key_column_to_legacy_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE {CHANGELOG} (id INTEGER PRIMARY KEY AUTOINCREMENT, tbl TEXT NOT NULL, op TEXT NOT NULL)"
        ))
        .unwrap();
        ensure_changelog(&conn).unwrap();
        ensure_changelog(&conn).unwrap();
        let columns: Vec<String> = conn
            .prepare(&format!(
                "SELECT name FROM pragma_table_info('{CHANGELOG}')"
            ))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(columns, vec!["id", "tbl", "op", "key"]);
        // And on a database with no changelog at all it creates the full shape.
        let fresh = Connection::open_in_memory().unwrap();
        ensure_changelog(&fresh).unwrap();
        ensure_changelog(&fresh).unwrap();
    }

    /// Bindings spelled in different cases must converge on ONE trigger set
    /// writing ONE changelog row per change — never a second set that
    /// double-logs every write. (SQLite resolves identifiers
    /// case-insensitively, and install_sql normalizes the embedded spelling;
    /// this pins both halves against a real database.)
    #[test]
    fn differently_cased_reinstall_never_double_logs() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE items (id INTEGER PRIMARY KEY, n INT)")
            .unwrap();
        let pk = vec!["id".to_string()];
        conn.execute_batch(&install_sql("items", &pk).unwrap())
            .unwrap();
        conn.execute_batch(&install_sql("ITEMS", &pk).unwrap())
            .unwrap();

        let triggers: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'trigger'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(triggers, 3, "reinstall must replace, not accumulate");

        conn.execute_batch("INSERT INTO items (n) VALUES (1)")
            .unwrap();
        let rows: Vec<(String, String)> = conn
            .prepare(&format!("SELECT tbl, op FROM {CHANGELOG}"))
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows, vec![("items".to_string(), "insert".to_string())]);
    }

    fn key(id: i64) -> Map<String, Value> {
        serde_json::json!({ "id": id }).as_object().unwrap().clone()
    }

    #[test]
    fn coalesce_merges_adjacent_runs_only() {
        let rows = vec![
            ("a".to_string(), Op::Insert, Some(key(1))),
            ("a".to_string(), Op::Insert, Some(key(2))),
            ("a".to_string(), Op::Update, Some(key(1))),
            ("b".to_string(), Op::Update, None),
            ("a".to_string(), Op::Insert, Some(key(3))),
        ];
        let runs = coalesce(rows);
        let shape: Vec<(&str, Op, u64)> =
            runs.iter().map(|r| (r.table.as_str(), r.op, r.n)).collect();
        assert_eq!(
            shape,
            vec![
                ("a", Op::Insert, 2),
                ("a", Op::Update, 1),
                ("b", Op::Update, 1),
                ("a", Op::Insert, 1),
            ]
        );
        assert_eq!(runs[0].keys, vec![key(1), key(2)]);
        assert!(runs[2].keys.is_empty());
        assert!(runs.iter().all(|r| !r.truncated));
        assert!(coalesce(Vec::new()).is_empty());
    }

    #[test]
    fn coalesce_caps_keys_and_flags_truncation() {
        let rows: Vec<_> = (0..=KEY_CAP as i64)
            .map(|i| ("a".to_string(), Op::Insert, Some(key(i))))
            .collect();
        let runs = coalesce(rows);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].n, KEY_CAP as u64 + 1, "the count stays exact");
        assert_eq!(runs[0].keys.len(), KEY_CAP);
        assert!(runs[0].truncated);

        // A run straddling a reinstall from a worker that recorded no keys:
        // some rows keyed, some not — say so rather than pass off a partial
        // list as complete.
        let mixed = coalesce(vec![
            ("a".to_string(), Op::Insert, None),
            ("a".to_string(), Op::Insert, Some(key(2))),
        ]);
        assert_eq!(mixed[0].keys, vec![key(2)]);
        assert!(mixed[0].truncated);

        // No keys anywhere (no primary key) is not truncation.
        let unkeyed = coalesce(vec![("a".to_string(), Op::Delete, None); 3]);
        assert!(unkeyed[0].keys.is_empty());
        assert!(!unkeyed[0].truncated);
    }

    #[test]
    fn parse_key_decodes_blob_wrappers_and_rejects_garbage() {
        assert_eq!(
            parse_key(r#"{"id":7,"t":"x"}"#).unwrap(),
            serde_json::json!({ "id": 7, "t": "x" })
                .as_object()
                .unwrap()
                .clone()
        );
        assert_eq!(
            parse_key(r#"{"b":{"$blob":"FF0010"}}"#).unwrap()["b"],
            "/wAQ"
        );
        // A one-key object that is not the wrapper stays as it is.
        assert_eq!(
            parse_key(r#"{"j":{"other":1}}"#).unwrap()["j"],
            serde_json::json!({ "other": 1 })
        );
        assert!(parse_key("[1]").is_none());
        assert!(parse_key("nope").is_none());
        assert!(parse_key(r#"{"b":{"$blob":"zz"}}"#).is_none());
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
            .execute_batch(&install_sql("items", &["id".into()]).unwrap())
            .unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (wake_tx, wake_rx) = mpsc::channel();
        let stop_wake = wake_tx.clone();
        let thread = {
            let stop = Arc::clone(&stop);
            let path = db_path.clone();
            std::thread::spawn(move || {
                run_watcher("primary", &path, &stop, wake_tx, &wake_rx, move |ev| {
                    tx.send(ev).is_ok()
                })
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
        // The event says WHICH rows: the primary keys, in row order.
        assert_eq!(
            events[0].returning.as_ref().unwrap(),
            &vec![key(1), key(2), key(3)]
        );
        assert_eq!(events[1].op, Op::Update);
        assert_eq!(events[1].affected_rows, 3);
        assert_eq!(events[1].returning.as_ref().unwrap().len(), 3);
        assert_eq!(events[2].op, Op::Delete);
        assert_eq!(events[2].affected_rows, 2);
        // After the update n is 2,3,4 → rows 2 and 3 go.
        assert_eq!(events[2].returning.as_ref().unwrap(), &vec![key(2), key(3)]);
        assert!(events.iter().all(|e| !e.truncated));

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
        let _ = stop_wake.send(());
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
            .execute_batch(&install_sql("items", &["id".into()]).unwrap())
            .unwrap();
        // Rows written before any watcher exists.
        external
            .execute_batch("INSERT INTO items (n) VALUES (1), (2)")
            .unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (wake_tx, wake_rx) = mpsc::channel();
        let stop_wake = wake_tx.clone();
        let thread = {
            let stop = Arc::clone(&stop);
            let path = db_path.clone();
            std::thread::spawn(move || {
                run_watcher("primary", &path, &stop, wake_tx, &wake_rx, move |ev| {
                    tx.send(ev).is_ok()
                })
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
        assert_eq!(ev.returning.as_ref().unwrap(), &vec![key(1), key(2)]);

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
        let _ = stop_wake.send(());
        thread.join().unwrap();
    }
}
