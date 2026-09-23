use super::{random_id, rows::Rows, Data, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const RETENTION_SECONDS: i64 = 7 * 24 * 60 * 60;
pub(super) const DELIVERY_LIMIT: usize = 100_000;
pub(super) const SEEN_LIMIT: usize = 4096;
pub(super) const TERMINAL_LIMIT: usize = 1000;
// Even capacity eviction leaves time for the just-completed watch-status read.
pub(super) const MIN_TERMINAL_SECONDS: i64 = 300;

pub(super) fn initialize(conn: &mut Connection) -> Result<Data> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version > 1 {
        return Err(super::Failure::Invalid(
            "unsupported webhook storage version".into(),
        ));
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS metadata (id INTEGER PRIMARY KEY CHECK(id=1), installation TEXT NOT NULL, tunnel_status TEXT NOT NULL, last_error TEXT);
        CREATE TABLE IF NOT EXISTS jobs (id TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS watches (id TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS repos (id TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS subscribers (id TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS publications (id TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS deliveries (id TEXT PRIMARY KEY, created_at INTEGER NOT NULL);
        CREATE INDEX IF NOT EXISTS deliveries_age ON deliveries(created_at, id);
        CREATE TABLE IF NOT EXISTS watch_seen (watch_id TEXT NOT NULL, fingerprint TEXT NOT NULL, created_at INTEGER NOT NULL, PRIMARY KEY(watch_id, fingerprint));
        CREATE INDEX IF NOT EXISTS watch_seen_age ON watch_seen(watch_id, created_at, fingerprint);
        CREATE TABLE IF NOT EXISTS terminal_watches (id TEXT PRIMARY KEY, cleaned_at INTEGER NOT NULL);
        CREATE INDEX IF NOT EXISTS terminal_age ON terminal_watches(cleaned_at, id);")?;
    let initialized: bool =
        tx.query_row("SELECT EXISTS(SELECT 1 FROM metadata)", [], |r| r.get(0))?;
    if !initialized {
        let legacy: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='state')",
            [],
            |r| r.get(0),
        )?;
        let data: Data = if legacy {
            let text: Option<String> = tx
                .query_row("SELECT value FROM state WHERE id=1", [], |r| r.get(0))
                .optional()?;
            match text {
                Some(text) => serde_json::from_str(&text)?,
                None => Data {
                    installation: random_id(),
                    ..Data::default()
                },
            }
        } else {
            Data {
                installation: random_id(),
                ..Data::default()
            }
        };
        tx.execute(
            "INSERT INTO metadata VALUES(1, ?1, ?2, ?3)",
            params![data.installation, data.tunnel_status, data.last_error],
        )?;
        let now = chrono::Utc::now().timestamp();
        for (id, value) in &data.jobs {
            upsert(&tx, "jobs", id, value)?;
        }
        for (id, value) in &data.watches {
            write_watch(&tx, id, value, now)?;
        }
        for (id, value) in &data.repos {
            upsert(&tx, "repos", id, value)?;
        }
        for (id, value) in &data.subscribers {
            upsert(&tx, "subscribers", id, value)?;
        }
        for (id, value) in &data.publications {
            upsert(&tx, "publications", id, value)?;
        }
        for id in data.deliveries.iter() {
            tx.execute("INSERT INTO deliveries VALUES(?1, ?2)", params![id, now])?;
        }
        // Keep an empty, write-guarded legacy table: old binaries CREATE IF NOT
        // EXISTS + INSERT OR IGNORE here, so rollback fails closed instead of
        // silently creating a second installation over the migrated database.
        tx.execute_batch("CREATE TABLE IF NOT EXISTS state (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL);
            DELETE FROM state;
            CREATE TRIGGER state_migrated_insert BEFORE INSERT ON state BEGIN SELECT RAISE(ABORT, 'webhook storage migrated; old binary unsupported'); END;
            CREATE TRIGGER state_migrated_update BEFORE UPDATE ON state BEGIN SELECT RAISE(ABORT, 'webhook storage migrated; old binary unsupported'); END;")?;
    }
    tx.execute_batch("PRAGMA user_version=1;")?;
    tx.commit()?;
    load(conn)
}

pub(super) fn load(conn: &Connection) -> Result<Data> {
    let mut data = conn.query_row(
        "SELECT installation, tunnel_status, last_error FROM metadata WHERE id=1",
        [],
        |r| {
            Ok(Data {
                installation: r.get(0)?,
                tunnel_status: r.get(1)?,
                last_error: r.get(2)?,
                ..Data::default()
            })
        },
    )?;
    data.jobs = load_rows(conn, "jobs")?;
    data.watches = load_rows(conn, "watches")?;
    data.repos = load_rows(conn, "repos")?;
    data.subscribers = load_rows(conn, "subscribers")?;
    data.publications = load_rows(conn, "publications")?;
    let mut stmt = conn.prepare("SELECT id FROM deliveries")?;
    for id in stmt.query_map([], |r| r.get::<_, String>(0))? {
        data.deliveries.insert(id?);
    }
    let mut stmt = conn.prepare("SELECT watch_id, fingerprint FROM watch_seen")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (id, key) = row?;
        if let Some(watch) = data.watches.get_mut(&id) {
            watch.seen.insert(key);
        }
    }
    clean(&mut data);
    Ok(data)
}
fn load_rows<V: DeserializeOwned>(conn: &Connection, table: &str) -> Result<Rows<V>> {
    let mut stmt = conn.prepare(&format!("SELECT id, value FROM {table}"))?;
    let mut values = Rows::default();
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (id, text) = row?;
        values.insert(id, serde_json::from_str(&text)?);
    }
    values.clean();
    Ok(values)
}
fn upsert<V: Serialize>(tx: &Transaction<'_>, table: &str, id: &str, value: &V) -> Result<()> {
    tx.execute(&format!("INSERT INTO {table}(id,value) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET value=excluded.value WHERE value!=excluded.value"), params![id, serde_json::to_string(value)?])?;
    Ok(())
}
fn sync_rows<V: Serialize>(tx: &Transaction<'_>, table: &str, values: &Rows<V>) -> Result<()> {
    for id in values.dirty() {
        match values.get(id) {
            Some(value) => upsert(tx, table, id, value)?,
            None => {
                tx.execute(&format!("DELETE FROM {table} WHERE id=?1"), [id])?;
            }
        }
    }
    Ok(())
}
fn write_watch(
    tx: &Transaction<'_>,
    id: &str,
    watch: &super::super::types::Watch,
    now: i64,
) -> Result<()> {
    let mut value = serde_json::to_value(watch)?;
    value["seen"] = serde_json::json!([]);
    upsert(tx, "watches", id, &value)?;
    let mut stmt = tx.prepare("SELECT fingerprint FROM watch_seen WHERE watch_id=?1")?;
    let old = stmt
        .query_map([id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    for key in old.difference(&watch.seen) {
        tx.execute(
            "DELETE FROM watch_seen WHERE watch_id=?1 AND fingerprint=?2",
            params![id, key],
        )?;
    }
    for key in watch.seen.difference(&old) {
        tx.execute(
            "INSERT INTO watch_seen VALUES(?1,?2,?3)",
            params![id, key, now],
        )?;
    }
    Ok(())
}

pub(super) fn persist(tx: &Transaction<'_>, old: &Data, data: &mut Data, now: i64) -> Result<()> {
    if old.installation != data.installation
        || old.tunnel_status != data.tunnel_status
        || old.last_error != data.last_error
    {
        tx.execute(
            "UPDATE metadata SET installation=?1,tunnel_status=?2,last_error=?3 WHERE id=1",
            params![data.installation, data.tunnel_status, data.last_error],
        )?;
    }
    sync_rows(tx, "jobs", &data.jobs)?;
    sync_rows(tx, "repos", &data.repos)?;
    sync_rows(tx, "subscribers", &data.subscribers)?;
    // Subscriber cancellation can remove jobs without explicitly touching claims.
    let removed_jobs: Vec<_> = data
        .jobs
        .dirty()
        .filter(|id| !data.jobs.contains_key(id))
        .cloned()
        .collect();
    for id in removed_jobs {
        data.publications.remove(&id);
    }
    sync_rows(tx, "publications", &data.publications)?;
    for id in data.deliveries.rows().dirty() {
        if data.deliveries.contains(id) {
            tx.execute(
                "INSERT OR IGNORE INTO deliveries VALUES(?1,?2)",
                params![id, now],
            )?;
        } else {
            tx.execute("DELETE FROM deliveries WHERE id=?1", [id])?;
        }
    }
    for id in data.watches.dirty() {
        if let Some(watch) = data.watches.get(id) {
            write_watch(tx, id, watch, now)?;
        } else {
            tx.execute("DELETE FROM watches WHERE id=?1", [id])?;
            tx.execute("DELETE FROM watch_seen WHERE watch_id=?1", [id])?;
            tx.execute("DELETE FROM terminal_watches WHERE id=?1", [id])?;
        }
    }
    Ok(())
}

/// Runs periodically, not on each accept. Pending inbox dedupe and terminal
/// snapshots referenced by an outbox job are never evicted for age or capacity.
pub(super) fn prune(tx: &Transaction<'_>, data: &mut Data, now: i64) -> Result<()> {
    let cutoff = now.saturating_sub(RETENTION_SECONDS);
    let mut stmt =
        tx.prepare("SELECT id,created_at FROM deliveries ORDER BY created_at DESC,id DESC")?;
    let mut retained = 0;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (id, at) = row?;
        if data.jobs.contains_key(&format!("inbox:{id}")) {
            continue;
        }
        retained += 1;
        if at < cutoff || retained > DELIVERY_LIMIT {
            tx.execute("DELETE FROM deliveries WHERE id=?1", [&id])?;
            data.deliveries.remove(&id);
        }
    }
    let mut seen = BTreeMap::<String, usize>::new();
    let mut stmt = tx.prepare("SELECT watch_id,fingerprint,created_at FROM watch_seen ORDER BY watch_id,created_at DESC,fingerprint DESC")?;
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })? {
        let (id, key, at) = row?;
        let count = seen.entry(id.clone()).or_default();
        *count += 1;
        if at < cutoff || *count > SEEN_LIMIT {
            tx.execute(
                "DELETE FROM watch_seen WHERE watch_id=?1 AND fingerprint=?2",
                params![id, key],
            )?;
            if let Some(watch) = data.watches.get_mut(&id) {
                watch.seen.remove(&key);
            }
        }
    }
    let pending_watches: BTreeSet<_> = data
        .jobs
        .values()
        .filter_map(|job| match job {
            super::super::types::Job::Notify { event, .. } => Some(event.watch_id.as_str()),
            _ => None,
        })
        .collect();
    let live_repos: BTreeSet<_> = data
        .watches
        .values()
        .filter(|watch| watch.live())
        .map(|watch| watch.spec.repo.as_str())
        .collect();
    for (id, watch) in &data.watches {
        let cleaned = !watch.live()
            && watch.status != super::super::types::WatchState::CleanupPending
            && watch.lease_id.is_none()
            && (!data.repos.contains_key(&watch.spec.repo)
                || live_repos.contains(watch.spec.repo.as_str()))
            && !pending_watches.contains(id.as_str());
        if cleaned {
            tx.execute(
                "INSERT OR IGNORE INTO terminal_watches VALUES(?1,?2)",
                params![id, now],
            )?;
        } else {
            tx.execute("DELETE FROM terminal_watches WHERE id=?1", [id])?;
        }
    }
    let mut stmt =
        tx.prepare("SELECT id,cleaned_at FROM terminal_watches ORDER BY cleaned_at DESC,id DESC")?;
    for (index, row) in stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .enumerate()
    {
        let (id, at) = row?;
        if at < cutoff
            || (index >= TERMINAL_LIMIT && at <= now.saturating_sub(MIN_TERMINAL_SECONDS))
        {
            tx.execute("DELETE FROM watches WHERE id=?1", [&id])?;
            tx.execute("DELETE FROM watch_seen WHERE watch_id=?1", [&id])?;
            tx.execute("DELETE FROM terminal_watches WHERE id=?1", [&id])?;
            data.watches.remove(&id);
        }
    }
    Ok(())
}
pub(super) fn clean(data: &mut Data) {
    data.jobs.clean();
    data.watches.clean();
    data.repos.clean();
    data.subscribers.clean();
    data.publications.clean();
    data.deliveries.clean();
}
