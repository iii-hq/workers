//! Live tests against real servers: the Database app's test environment
//! (docker — PostgreSQL 17, MySQL 8.4, an OpenSSH bastion in front of a
//! private PostgreSQL; the env folder lives in the iii app repo as
//! `projects/database-app/env`). They exercise what unit tests cannot: TLS
//! modes and mutual TLS against real servers, SSH tunnels (password and key
//! auth, host-key pins), every column type of the `type_zoo` tables, raw
//! browse filters that try to write, `max_rows`, `insertRow` casts, and a
//! connection whose secret does not resolve next to one that works.
//!
//! Skipped unless `DATABASE_E2E_ENV` points at that env folder (its `tls/`
//! and `ssh/` hold the CA, client certificates and keys). Ports and
//! credentials are the env's (see its README).
//!
//!   DATABASE_E2E_ENV=/path/to/env cargo test --test live -- --test-threads=1

use database::config::WorkerConfig;
use database::configuration;
use database::handle::HandleRegistry;
use database::handlers::{
    browse, execute, insert_row, list_databases, query, schema, test_connection, AppState,
};
use database::runtime::Runtime;
use database::secrets::{Fetch, SecretCache, SecretError, SecretValue};
use database::transaction::TxRegistry;
use iii_helpers::observability::Logger;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

const PG_PW: &str = "shop-e2e-pw";
const PRIVATE_PG_PW: &str = "shop-private-pw";
const ED25519_FP: &str = "SHA256:0rGFgr3lbFUjBJmay4CvRxX16hnJLF7mvototEUXoW0";

fn env_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var("DATABASE_E2E_ENV").ok()?);
    dir.join("tls/ca.crt").exists().then_some(dir)
}

macro_rules! env_or_skip {
    () => {
        match env_dir() {
            Some(dir) => dir,
            None => {
                eprintln!("skipping: DATABASE_E2E_ENV not set");
                return;
            }
        }
    };
}

fn file(dir: &std::path::Path, rel: &str) -> String {
    dir.join(rel).display().to_string()
}

type Secrets = Arc<Mutex<HashMap<String, String>>>;

fn runtime(secrets: Secrets) -> Arc<Runtime> {
    let fetch: Fetch = Arc::new(move |key: String| {
        let secrets = secrets.clone();
        Box::pin(async move {
            match secrets.lock().unwrap().get(&key) {
                Some(v) => Ok(SecretValue::new(v.clone())),
                None => Err(SecretError::NotFound),
            }
        })
    });
    Arc::new(Runtime::new(Arc::new(SecretCache::new(fetch))))
}

async fn state_with(databases: Value, secrets: Secrets) -> AppState {
    let cfg = WorkerConfig::from_json(&json!({ "databases": databases })).unwrap();
    let st = AppState {
        pools: Default::default(),
        config: Arc::new(RwLock::new(cfg.clone())),
        handles: Arc::new(HandleRegistry::new()),
        transactions: TxRegistry::new(),
        log: Logger::new(),
        row_changes: None,
        runtime: runtime(secrets),
    };
    configuration::apply_config(&st, cfg, None).await.unwrap();
    st
}

async fn state(databases: Value) -> AppState {
    state_with(databases, Default::default()).await
}

fn pg(user: &str, password: &str) -> Value {
    json!({ "driver": "postgres", "host": "127.0.0.1", "port": 55432, "user": user,
            "password": password, "database": "shop", "tls": { "mode": "disable" } })
}

/// Tests that create tables use the `iii_test` databases (created next to
/// `shop` for the e2e harness), never `shop`, which the app shows.
fn pg_scratch(user: &str, password: &str) -> Value {
    let mut entry = pg(user, password);
    entry["database"] = json!("iii_test");
    entry
}

fn mysql_scratch(user: &str, password: &str) -> Value {
    let mut entry = mysql(user, password);
    entry["database"] = json!("iii_test");
    entry
}

fn mysql(user: &str, password: &str) -> Value {
    json!({ "driver": "mysql", "host": "127.0.0.1", "port": 53306, "user": user,
            "password": password, "database": "shop", "tls": { "mode": "disable" } })
}

async fn q(st: &AppState, db: &str, sql: &str) -> Result<query::QueryResp, String> {
    query::handle(
        st,
        serde_json::from_value(json!({ "db": db, "sql": sql })).unwrap(),
    )
    .await
}

async fn exec(st: &AppState, db: &str, sql: &str) {
    execute::handle(
        st,
        serde_json::from_value(json!({ "db": db, "sql": sql })).unwrap(),
    )
    .await
    .unwrap_or_else(|e| panic!("{db}: {sql}: {e}"));
}

/// Literal credentials travel in `credentials` (the worker refuses them
/// inside `connection`); references and file paths stay in the entry.
fn split_credentials(mut connection: Value) -> (Value, Value) {
    let mut credentials = json!({});
    let literal = |v: &Value| {
        v.as_str().is_some_and(|s| {
            !s.starts_with("secret://") && !s.starts_with("env://") && !s.starts_with('/')
        })
    };
    for path in [
        vec!["password"],
        vec!["tls", "client_key"],
        vec!["ssh", "password"],
        vec!["ssh", "private_key"],
        vec!["ssh", "passphrase"],
    ] {
        let parent = if path.len() == 1 {
            Some(&mut connection)
        } else {
            connection.get_mut(path[0])
        };
        let Some(obj) = parent.and_then(Value::as_object_mut) else {
            continue;
        };
        let key = path[path.len() - 1];
        if obj.get(key).is_some_and(literal) {
            let value = obj.remove(key).unwrap();
            if path.len() == 1 {
                credentials[key] = value;
            } else {
                if credentials.get(path[0]).is_none() {
                    credentials[path[0]] = json!({});
                }
                credentials[path[0]][key] = value;
            }
        }
    }
    (connection, credentials)
}

async fn probe(st: &AppState, connection: Value) -> test_connection::TestConnectionResp {
    let (connection, credentials) = split_credentials(connection);
    test_connection::handle(
        st,
        serde_json::from_value(
            json!({ "connection": connection, "credentials": credentials, "timeout_ms": 20000 }),
        )
        .unwrap(),
    )
    .await
    .unwrap()
}

fn copy_sqlite(dir: &std::path::Path) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::copy(dir.join("data/shop.db"), tmp.path().join("shop.db")).unwrap();
    tmp
}

#[tokio::test(flavor = "multi_thread")]
async fn type_zoo_decodes_every_column_on_every_driver() {
    let dir = env_or_skip!();
    let tmp = copy_sqlite(&dir);
    let st = state(json!({
        "pg": pg("shop", PG_PW),
        "my": mysql("shop", PG_PW),
        "lite": { "driver": "sqlite", "database": tmp.path().join("shop.db").display().to_string() },
    }))
    .await;

    let rows = q(&st, "pg", "SELECT * FROM type_zoo ORDER BY id")
        .await
        .unwrap()
        .rows;
    assert_eq!(rows.len(), 3);
    let r = &rows[0];
    assert_eq!(r["c_bigint"], json!("9007199254740993"));
    assert_eq!(r["c_numeric"], json!("12345678.1234"));
    assert_eq!(r["c_date"], json!("2026-02-28"));
    assert_eq!(r["c_time"], json!("23:59:59"));
    assert_eq!(r["c_timetz"], json!("12:30:00+03"));
    assert_eq!(r["c_interval"], json!("1 day 02:03:04"));
    assert_eq!(r["c_enum"], json!("shipped"));
    assert_eq!(r["c_int_array"], json!([1, 2, 3]));
    assert_eq!(r["c_text_array"], json!(["a", "b c", "ü"]));
    assert_eq!(r["c_cidr"], json!("10.0.0.0/8"));
    assert_eq!(r["c_macaddr"], json!("08:00:2b:01:02:03"));
    assert_eq!(r["c_money"], json!("1234.56"));
    assert_eq!(r["c_point"], json!("(1.5,-2.25)"));
    assert!(r["c_jsonb"].is_object());
    assert!(r["c_tsvector"].as_str().unwrap().contains("'banana'"));
    assert!(
        rows[1]
            .iter()
            .filter(|(k, _)| k.as_str() != "id"
                && k.as_str() != "c_serial"
                && k.as_str() != "c_bool"
                && k.as_str() != "c_timestamptz")
            .all(|(_, v)| v.is_null()),
        "row 2 is all NULL"
    );
    // Every shop table browses now (they used to fail on date/enum/array columns).
    for table in [
        "products",
        "customers",
        "orders",
        "v_order_totals",
        "mv_daily_sales",
    ] {
        browse::handle(
            &st,
            serde_json::from_value(json!({ "db": "pg", "table": table })).unwrap(),
        )
        .await
        .unwrap_or_else(|e| panic!("browse {table}: {e}"));
    }

    let rows = q(&st, "my", "SELECT * FROM type_zoo ORDER BY id")
        .await
        .unwrap()
        .rows;
    let r = &rows[0];
    assert_eq!(r["c_time"], json!("23:59:59"));
    assert_eq!(r["c_date"], json!("2026-02-28"));
    assert_eq!(r["c_datetime"], json!("2026-10-18T12:00:00.123Z"));
    assert_eq!(r["c_binary"], json!("AAECAw=="));
    assert_eq!(r["c_bit"], json!("10100101"));
    assert_eq!(r["c_bigint_unsigned"], json!("18446744073709551615"));
    assert_eq!(r["c_decimal"], json!("12345678.1234"));
    assert_eq!(r["c_set"], json!("wifi,usb"));
    assert!(r["c_json"].is_object(), "{:?}", r["c_json"]);
    let cols = q(&st, "my", "SELECT c_enum, c_set FROM type_zoo")
        .await
        .unwrap()
        .columns;
    assert_eq!(cols[0].ty, "enum");
    assert_eq!(cols[1].ty, "set");

    let rows = q(&st, "lite", "SELECT * FROM type_zoo ORDER BY id")
        .await
        .unwrap()
        .rows;
    assert!(rows[0]["c_json"].is_object());
    assert_eq!(rows[0]["c_integer"], json!(9007199254740993i64));
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_filters_never_write_even_as_a_superuser() {
    let dir = env_or_skip!();
    let tmp = tempfile::tempdir().unwrap();
    let lite = tmp.path().join("raw.db").display().to_string();
    let st = state(json!({
        "pg": pg_scratch("postgres", "postgres-e2e"),
        "my": mysql_scratch("root", "root-e2e"),
        "lite": { "driver": "sqlite", "database": lite, "pool": { "max": 1 } },
    }))
    .await;
    let _ = dir;
    for db in ["pg", "my", "lite"] {
        exec(&st, db, "DROP TABLE IF EXISTS iii_wcore_raw").await;
        exec(
            &st,
            db,
            "CREATE TABLE iii_wcore_raw (id INT PRIMARY KEY, n INT)",
        )
        .await;
        exec(
            &st,
            db,
            "INSERT INTO iii_wcore_raw VALUES (1, 10), (2, 20), (3, 30)",
        )
        .await;
    }
    exec(&st, "pg", "CREATE SEQUENCE IF NOT EXISTS iii_wcore_seq").await;
    exec(
        &st,
        "pg",
        "CREATE OR REPLACE FUNCTION iii_wcore_write() RETURNS boolean LANGUAGE sql AS \
         $$ INSERT INTO iii_wcore_raw VALUES (99, 99) RETURNING true $$",
    )
    .await;
    // CREATE FUNCTION is not allowed over the prepared protocol: text protocol.
    let root = mysql_async::Pool::new("mysql://root:root-e2e@127.0.0.1:53306/iii_test");
    {
        use mysql_async::prelude::Queryable;
        let mut c = root.get_conn().await.unwrap();
        c.query_drop("DROP FUNCTION IF EXISTS iii_wcore_write")
            .await
            .unwrap();
        c.query_drop(
            "CREATE FUNCTION iii_wcore_write() RETURNS INT DETERMINISTIC MODIFIES SQL DATA \
             BEGIN INSERT INTO iii_wcore_raw VALUES (99, 99); RETURN 1; END",
        )
        .await
        .unwrap();
    }

    let browse_raw = |db: &'static str, raw: &'static str| {
        let st = st.clone();
        async move {
            browse::handle(
                &st,
                serde_json::from_value(
                    json!({ "db": db, "table": "iii_wcore_raw", "filters": [{ "raw": raw }] }),
                )
                .unwrap(),
            )
            .await
        }
    };
    let attempts: &[(&str, &str)] = &[
        ("pg", "1=1; DELETE FROM iii_wcore_raw"),
        (
            "pg",
            "id IN (WITH d AS (DELETE FROM iii_wcore_raw RETURNING id) SELECT id FROM d)",
        ),
        ("pg", "iii_wcore_write()"),
        ("pg", "nextval('iii_wcore_seq') > 0"),
        ("pg", "1=1) OR (1=1"),
        ("my", "1=1; DELETE FROM iii_wcore_raw"),
        ("my", "iii_wcore_write() = 1"),
        ("my", "1=1) INTO OUTFILE '/tmp/iii_wcore' -- "),
        ("my", "id IN (SELECT id FROM (SELECT 1 AS id) x) # )"),
        ("lite", "1=1; DELETE FROM iii_wcore_raw"),
        (
            "lite",
            "id IN (WITH d AS (DELETE FROM iii_wcore_raw RETURNING id) SELECT id FROM d)",
        ),
        ("lite", "1=1) UNION SELECT 1, 2 --"),
    ];
    for (db, raw) in attempts {
        let result = browse_raw(db, raw).await;
        assert!(
            result.is_err(),
            "{db}: raw filter {raw:?} was accepted: {result:?}"
        );
    }
    // The writes that pass the lexer are stopped by the server's read-only
    // transaction, not by luck.
    let err = browse_raw("pg", "iii_wcore_write()").await.unwrap_err();
    assert!(err.contains("read-only transaction"), "{err}");
    let err = browse_raw("pg", "nextval('iii_wcore_seq') > 0")
        .await
        .unwrap_err();
    assert!(err.contains("read-only transaction"), "{err}");
    let err = browse_raw("my", "iii_wcore_write() = 1").await.unwrap_err();
    assert!(err.to_uppercase().contains("READ ONLY"), "{err}");
    // A legitimate raw filter works, and nothing was written anywhere.
    for db in ["pg", "my", "lite"] {
        let page = browse_raw(db, "n >= 20 AND id < 3").await.unwrap();
        assert_eq!(page.rows.len(), 1, "{db}");
        let rows = q(
            &st,
            db,
            "SELECT COUNT(*) AS c, MAX(id) AS m FROM iii_wcore_raw",
        )
        .await
        .unwrap()
        .rows;
        assert_eq!(
            rows[0]["c"].to_string().trim_matches('"'),
            "3",
            "{db}: {rows:?}"
        );
        assert_eq!(rows[0]["m"].to_string().trim_matches('"'), "3", "{db}");
    }
    // sqlite's query_only was switched back off: the same (only) pooled
    // connection still writes.
    exec(&st, "lite", "INSERT INTO iii_wcore_raw VALUES (4, 40)").await;
    exec(&st, "pg", "DROP FUNCTION iii_wcore_write()").await;
    exec(&st, "pg", "DROP SEQUENCE iii_wcore_seq").await;
    {
        use mysql_async::prelude::Queryable;
        let mut c = root.get_conn().await.unwrap();
        c.query_drop("DROP FUNCTION iii_wcore_write").await.unwrap();
    }
    for db in ["pg", "my"] {
        exec(&st, db, "DROP TABLE iii_wcore_raw").await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn max_rows_stops_reading_and_says_so() {
    let dir = env_or_skip!();
    let tmp = copy_sqlite(&dir);
    let st = state(json!({
        "pg": pg("shop", PG_PW),
        "my": mysql("shop", PG_PW),
        "lite": { "driver": "sqlite", "database": tmp.path().join("shop.db").display().to_string() },
    }))
    .await;
    for db in ["pg", "my", "lite"] {
        let cut = query::handle(
            &st,
            serde_json::from_value(
                json!({ "db": db, "sql": "SELECT * FROM orders ORDER BY id", "max_rows": 100 }),
            )
            .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(cut.rows.len(), 100, "{db}");
        assert!(cut.truncated, "{db}");
        let all = query::handle(
            &st,
            serde_json::from_value(
                json!({ "db": db, "sql": "SELECT id FROM orders", "max_rows": 5000 }),
            )
            .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(all.rows.len(), 2000, "{db}");
        assert!(!all.truncated, "{db}");
        // An explicit LIMIT in the SQL is kept; max_rows still caps.
        let limited = query::handle(
            &st,
            serde_json::from_value(
                json!({ "db": db, "sql": "SELECT id FROM orders LIMIT 500", "max_rows": 10 }),
            )
            .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(limited.rows.len(), 10, "{db}");
        assert!(limited.truncated, "{db}");
        // Exactly max_rows rows is not truncated.
        let exact = query::handle(
            &st,
            serde_json::from_value(
                json!({ "db": db, "sql": "SELECT id FROM orders LIMIT 7", "max_rows": 7 }),
            )
            .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(exact.rows.len(), 7, "{db}");
        assert!(!exact.truncated, "{db}");
        // The connection is clean afterwards (no leftover portal or limit).
        let n = q(&st, db, "SELECT COUNT(*) AS c FROM orders")
            .await
            .unwrap();
        assert_eq!(n.rows[0]["c"].to_string().trim_matches('"'), "2000", "{db}");
        let all_rows = q(&st, db, "SELECT id FROM orders").await.unwrap();
        assert_eq!(all_rows.rows.len(), 2000, "{db}: sql_select_limit leaked");
        assert!(!all_rows.truncated);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn insert_row_casts_values_to_each_column_type() {
    let _dir = env_or_skip!();
    let tmp = tempfile::tempdir().unwrap();
    let st = state(json!({
        "pg": pg_scratch("shop", PG_PW),
        "my": mysql_scratch("shop", PG_PW),
        "lite": { "driver": "sqlite", "database": tmp.path().join("ins.db").display().to_string() },
    }))
    .await;
    let insert = |db: &'static str, values: Value| {
        let st = st.clone();
        async move {
            insert_row::handle(
                &st,
                serde_json::from_value(
                    json!({ "db": db, "table": "iii_wcore_ins", "values": values }),
                )
                .unwrap(),
            )
            .await
        }
    };

    exec(&st, "pg", "DROP TABLE IF EXISTS iii_wcore_ins").await;
    exec(
        &st,
        "pg",
        "DO $$ BEGIN CREATE TYPE order_status AS ENUM ('pending', 'paid', 'shipped', 'delivered', \
         'cancelled'); EXCEPTION WHEN duplicate_object THEN NULL; END $$",
    )
    .await;
    exec(
        &st,
        "pg",
        "CREATE TABLE iii_wcore_ins (id bigserial PRIMARY KEY, price numeric(10,2), active boolean, \
         meta jsonb, tags text[], born date, at timestamptz, data bytea, uid uuid, \
         status order_status DEFAULT 'pending', note text DEFAULT 'none')",
    )
    .await;
    let r = insert(
        "pg",
        json!({ "price": "12.5", "active": "true", "meta": { "a": [1, 2] }, "tags": ["x", "y \"z\""],
                "born": "2026-01-02", "at": "2026-01-02T03:04:05Z", "data": "aGVsbG8",
                "uid": "6f1c2a8e-3b5d-4c7e-9f10-2a3b4c5d6e7f", "status": "paid" }),
    )
    .await
    .unwrap();
    let row = r.row.unwrap();
    assert_eq!(r.affected_rows, 1);
    assert_eq!(row["id"], json!("1"));
    assert_eq!(row["price"], json!("12.50"));
    assert_eq!(row["active"], json!(true));
    assert_eq!(row["meta"], json!({ "a": [1, 2] }));
    assert_eq!(row["tags"], json!(["x", "y \"z\""]));
    assert_eq!(row["born"], json!("2026-01-02"));
    assert_eq!(row["at"], json!("2026-01-02T03:04:05Z"));
    assert_eq!(row["data"], json!("aGVsbG8="));
    assert_eq!(row["status"], json!("paid"));
    assert_eq!(
        row["note"],
        json!("none"),
        "an omitted column takes its default"
    );
    let err = insert("pg", json!({ "price": "not a number" }))
        .await
        .unwrap_err();
    assert!(
        err.contains("DRIVER_ERROR") && err.contains("numeric"),
        "{err}"
    );
    exec(&st, "pg", "DROP TABLE iii_wcore_ins").await;

    exec(&st, "my", "DROP TABLE IF EXISTS iii_wcore_ins").await;
    exec(
        &st,
        "my",
        "CREATE TABLE iii_wcore_ins (id bigint AUTO_INCREMENT PRIMARY KEY, price decimal(10,2), \
         active tinyint(1), meta json, born date, data blob, note varchar(16) DEFAULT 'none', \
         at datetime(3))",
    )
    .await;
    let r = insert(
        "my",
        json!({ "price": "12.5", "active": true, "meta": { "a": 1 }, "born": "2026-01-02", "data": "AAEC",
                "at": "2026-01-02T05:04:05.250+02:00" }),
    )
    .await
    .unwrap();
    let row = r.row.expect("mysql re-selects by last_insert_id");
    assert_eq!(row["at"], json!("2026-01-02T03:04:05.250Z"));
    let r = insert("my", json!({ "at": "2026-01-02 03:04" }))
        .await
        .unwrap();
    assert_eq!(r.row.unwrap()["at"], json!("2026-01-02T03:04:00Z"));
    assert_eq!(row["price"], json!("12.50"));
    assert_eq!(row["active"], json!(1));
    assert_eq!(row["meta"], json!({ "a": 1 }));
    assert_eq!(row["born"], json!("2026-01-02"));
    assert_eq!(row["data"], json!("AAEC"));
    assert_eq!(row["note"], json!("none"));
    exec(&st, "my", "DROP TABLE iii_wcore_ins").await;

    exec(
        &st,
        "lite",
        "CREATE TABLE iii_wcore_ins (id INTEGER PRIMARY KEY, price NUMERIC, active BOOLEAN, meta JSON, data BLOB, note TEXT DEFAULT 'none')",
    )
    .await;
    let r = insert(
        "lite",
        json!({ "price": "12.5", "active": "false", "meta": { "a": 1 }, "data": "AAEC" }),
    )
    .await
    .unwrap();
    let row = r.row.unwrap();
    assert_eq!(row["id"], json!(1));
    assert_eq!(row["price"], json!(12.5));
    assert_eq!(row["active"], json!(0));
    assert_eq!(row["meta"], json!({ "a": 1 }));
    assert_eq!(row["data"], json!("AAEC"));
    assert_eq!(row["note"], json!("none"));
}

#[tokio::test(flavor = "multi_thread")]
async fn tls_modes_and_mutual_tls_against_real_servers() {
    let dir = env_or_skip!();
    let st = state(json!({ "pg": pg("shop", PG_PW) })).await;
    let ca = file(&dir, "tls/ca.crt");
    let pg_at = |host: &str, mode: &str, extra: Value| {
        let mut entry = json!({ "driver": "postgres", "host": host, "port": 55432, "user": "shop",
                                "password": PG_PW, "database": "shop", "tls": { "mode": mode } });
        if let Value::Object(extra) = extra {
            for (k, v) in extra {
                entry["tls"][k] = v;
            }
        }
        entry
    };

    let ok = probe(
        &st,
        pg_at("127.0.0.1", "verify-full", json!({ "ca_cert": ca })),
    )
    .await;
    assert!(ok.ok, "{:?}", ok.message);
    let bad_name = probe(
        &st,
        pg_at("127.0.0.2", "verify-full", json!({ "ca_cert": ca })),
    )
    .await;
    assert!(!bad_name.ok);
    assert_eq!(
        serde_json::to_value(bad_name.stage).unwrap(),
        json!("tls"),
        "{:?}",
        bad_name.message
    );
    let ca_only = probe(
        &st,
        pg_at("127.0.0.2", "verify-ca", json!({ "ca_cert": ca })),
    )
    .await;
    assert!(ca_only.ok, "{:?}", ca_only.message);
    let untrusted = probe(&st, pg_at("127.0.0.1", "require", json!({}))).await;
    assert!(!untrusted.ok);
    assert_eq!(serde_json::to_value(untrusted.stage).unwrap(), json!("tls"));
    let wrong_ca = probe(
        &st,
        pg_at(
            "127.0.0.1",
            "verify-ca",
            json!({ "ca_cert": file(&dir, "tls/other-ca.crt"), "trust_native": false }),
        ),
    )
    .await;
    assert!(!wrong_ca.ok);
    // prefer validates like require when the server offers TLS.
    let prefer = probe(&st, pg_at("127.0.0.1", "prefer", json!({ "ca_cert": ca }))).await;
    assert!(prefer.ok, "{:?}", prefer.message);

    let mut tls_only = pg_at("127.0.0.1", "disable", json!({}));
    tls_only["user"] = json!("shop_tls");
    tls_only["password"] = json!("shop-tls-pw");
    let refused = probe(&st, tls_only.clone()).await;
    assert!(!refused.ok);
    assert_eq!(
        serde_json::to_value(refused.stage).unwrap(),
        json!("auth"),
        "{:?}",
        refused.message
    );
    tls_only["tls"] = json!({ "mode": "verify-full", "ca_cert": ca });
    assert!(probe(&st, tls_only).await.ok);

    // Mutual TLS: path, PKCS#1, inline PEM.
    let client_pem = std::fs::read_to_string(dir.join("tls/client.crt")).unwrap();
    let key_pem = std::fs::read_to_string(dir.join("tls/client.key")).unwrap();
    for (cert, key) in [
        (file(&dir, "tls/client.crt"), file(&dir, "tls/client.key")),
        (
            file(&dir, "tls/client.crt"),
            file(&dir, "tls/client.pkcs1.key"),
        ),
        (client_pem.clone(), key_pem.clone()),
    ] {
        let mut mtls = pg_at(
            "127.0.0.1",
            "verify-full",
            json!({ "ca_cert": ca, "client_cert": cert, "client_key": key }),
        );
        mtls["user"] = json!("shop_mtls");
        mtls.as_object_mut().unwrap().remove("password");
        let resp = probe(&st, mtls).await;
        assert!(resp.ok, "{:?}", resp.message);
    }
    let mut no_cert = pg_at("127.0.0.1", "verify-full", json!({ "ca_cert": ca }));
    no_cert["user"] = json!("shop_mtls");
    no_cert.as_object_mut().unwrap().remove("password");
    assert!(!probe(&st, no_cert).await.ok);

    // MySQL: verify-full with the CA, REQUIRE SSL users, mutual TLS.
    let my = |user: &str, password: &str, tls: Value| {
        json!({ "driver": "mysql", "host": "127.0.0.1", "port": 53306, "user": user,
                "password": password, "database": "shop", "tls": tls })
    };
    let resp = probe(
        &st,
        my(
            "shop",
            PG_PW,
            json!({ "mode": "verify-full", "ca_cert": ca }),
        ),
    )
    .await;
    assert!(resp.ok, "{:?}", resp.message);
    let resp = probe(
        &st,
        my("shop_tls", "shop-tls-pw", json!({ "mode": "disable" })),
    )
    .await;
    assert!(!resp.ok);
    let resp = probe(
        &st,
        my("shop_mtls", "shop-mtls-pw", json!({ "mode": "verify-full", "ca_cert": ca,
             "client_cert": file(&dir, "tls/client.crt"), "client_key": file(&dir, "tls/client.key") })),
    )
    .await;
    assert!(resp.ok, "{:?}", resp.message);
    let resp = probe(
        &st,
        my(
            "shop_mtls",
            "shop-mtls-pw",
            json!({ "mode": "verify-full", "ca_cert": ca }),
        ),
    )
    .await;
    assert!(!resp.ok);
}

#[tokio::test(flavor = "multi_thread")]
async fn ssh_tunnels_with_password_keys_and_host_key_pins() {
    let dir = env_or_skip!();
    let ca = file(&dir, "tls/ca.crt");
    let passphrase = std::fs::read_to_string(dir.join("ssh/PASSPHRASE"))
        .unwrap()
        .trim()
        .to_string();
    let rsa_pem = std::fs::read_to_string(dir.join("ssh/id_rsa_pem")).unwrap();
    let secrets: Secrets = Arc::new(Mutex::new(HashMap::from([
        ("secret://WCORE_SSH_KEY".to_string(), rsa_pem),
        (
            "secret://WCORE_SSH_PASSPHRASE".to_string(),
            passphrase.clone(),
        ),
    ])));
    let st = state_with(json!({ "pg": pg("shop", PG_PW) }), secrets.clone()).await;
    let private = |ssh: Value| {
        json!({ "driver": "postgres", "host": "pg-private", "port": 5432, "user": "shop",
                "password": PRIVATE_PG_PW, "database": "shop",
                "tls": { "mode": "verify-full", "ca_cert": ca }, "ssh": ssh })
    };
    let bastion = |auth: Value| {
        let mut ssh = json!({ "host": "127.0.0.1", "port": 52222, "user": "tunnel" });
        for (k, v) in auth.as_object().unwrap() {
            ssh[k] = v.clone();
        }
        ssh
    };

    // Password, Ed25519, Ed25519 with passphrase, RSA PEM with passphrase
    // through secret:// references. TLS through the tunnel verifies
    // `pg-private`, not 127.0.0.1.
    for auth in [
        json!({ "password": "tunnel-e2e-pw" }),
        json!({ "private_key": file(&dir, "ssh/id_ed25519") }),
        json!({ "private_key": file(&dir, "ssh/id_ed25519_pass"), "passphrase": passphrase }),
        json!({ "private_key": "secret://WCORE_SSH_KEY", "passphrase": "secret://WCORE_SSH_PASSPHRASE" }),
    ] {
        let resp = probe(&st, private(bastion(auth.clone()))).await;
        assert!(resp.ok, "{auth}: {:?} {:?}", resp.stage, resp.message);
        let key = resp.ssh_host_key.expect("the tunnel reports the host key");
        assert_eq!(key.fingerprint, ED25519_FP);
        assert_eq!(key.algorithm, "ssh-ed25519");
    }

    let refused = probe(
        &st,
        private(bastion(
            json!({ "private_key": file(&dir, "ssh/id_ed25519_unauthorized") }),
        )),
    )
    .await;
    assert!(!refused.ok);
    assert_eq!(serde_json::to_value(refused.stage).unwrap(), json!("ssh"));
    let wrong_pw = probe(&st, private(bastion(json!({ "password": "nope" })))).await;
    assert_eq!(serde_json::to_value(wrong_pw.stage).unwrap(), json!("ssh"));
    assert!(!wrong_pw.message.unwrap().contains("nope"));

    // Host-key pins: the right one passes, a wrong one fails naming both.
    let pinned = probe(
        &st,
        private(bastion(
            json!({ "password": "tunnel-e2e-pw", "host_key": ED25519_FP }),
        )),
    )
    .await;
    assert!(pinned.ok, "{:?}", pinned.message);
    let wrong_pin = "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let mismatch = probe(
        &st,
        private(bastion(
            json!({ "password": "tunnel-e2e-pw", "host_key": wrong_pin }),
        )),
    )
    .await;
    assert!(!mismatch.ok);
    assert_eq!(serde_json::to_value(mismatch.stage).unwrap(), json!("ssh"));
    let message = mismatch.message.unwrap();
    assert!(
        message.contains(wrong_pin) && message.contains(ED25519_FP),
        "{message}"
    );
    assert_eq!(mismatch.ssh_host_key.unwrap().fingerprint, ED25519_FP);

    // The far end unreachable from the bastion is a tcp failure.
    let mut nowhere = private(bastion(json!({ "password": "tunnel-e2e-pw" })));
    nowhere["host"] = json!("no-such-host");
    let resp = probe(&st, nowhere).await;
    assert!(!resp.ok);
    assert_eq!(
        serde_json::to_value(resp.stage).unwrap(),
        json!("tcp"),
        "{:?}",
        resp.message
    );

    // MySQL through the tunnel (verify-full against `mysql`).
    let resp = probe(
        &st,
        json!({ "driver": "mysql", "host": "mysql", "port": 3306, "user": "shop", "password": PG_PW,
                "database": "shop", "tls": { "mode": "verify-full", "ca_cert": ca },
                "ssh": bastion(json!({ "private_key": file(&dir, "ssh/id_ed25519") })) }),
    )
    .await;
    assert!(resp.ok, "{:?}", resp.message);

    // A configured connection behind the bastion: a pool through the tunnel,
    // and a native-capture-free reload keeps the same pool.
    let st = state_with(
        json!({ "private": private(bastion(json!({ "private_key": "secret://WCORE_SSH_KEY",
                                                   "passphrase": "secret://WCORE_SSH_PASSPHRASE",
                                                   "host_key": ED25519_FP }))) }),
        secrets,
    )
    .await;
    for _ in 0..3 {
        let rows = q(
            &st,
            "private",
            "SELECT value FROM server_info WHERE key = 'instance'",
        )
        .await
        .unwrap()
        .rows;
        assert_eq!(rows[0]["value"], json!("pg-private"));
    }
    let cfg = st.config.read().await.clone();
    let before = st.runtime.connector("private").unwrap();
    configuration::apply_config(&st, cfg, None).await.unwrap();
    assert!(
        Arc::ptr_eq(&before, &st.runtime.connector("private").unwrap()),
        "an unchanged entry keeps its pool and tunnel"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unresolvable_secret_fails_only_its_connection() {
    let _dir = env_or_skip!();
    let secrets: Secrets = Arc::new(Mutex::new(HashMap::from([(
        "secret://WCORE_PG".to_string(),
        PG_PW.to_string(),
    )])));
    let mut missing = pg("shop", "secret://WCORE_MISSING");
    missing["port"] = json!(55432);
    let st = state_with(
        json!({ "good": pg("shop", "secret://WCORE_PG"), "env_ref": pg("shop", "env://WCORE_ENV_PG"), "missing": missing }),
        secrets.clone(),
    )
    .await;
    assert_eq!(
        q(&st, "good", "SELECT 1 AS one").await.unwrap().rows[0]["one"],
        json!(1)
    );
    let err = q(&st, "missing", "SELECT 1").await.unwrap_err();
    assert!(
        err.contains("CONFIG_ERROR") && err.contains("secret://WCORE_MISSING"),
        "{err}"
    );
    let listed = list_databases::handle(&st, Default::default())
        .await
        .unwrap();
    let missing_info = listed
        .databases
        .iter()
        .find(|d| d.name == "missing")
        .unwrap();
    assert!(missing_info
        .error
        .as_deref()
        .unwrap()
        .contains("secret://WCORE_MISSING"));
    assert!(missing_info.uses_secrets);
    assert!(listed
        .databases
        .iter()
        .find(|d| d.name == "good")
        .unwrap()
        .error
        .is_none());
    assert!(!serde_json::to_string(&listed).unwrap().contains(PG_PW));

    // Storing the secret and re-reading it brings the connection up; the
    // working one keeps its pool.
    secrets
        .lock()
        .unwrap()
        .insert("secret://WCORE_MISSING".into(), PG_PW.into());
    secrets
        .lock()
        .unwrap()
        .insert("env://WCORE_ENV_PG".into(), PG_PW.into());
    let good_before = st.runtime.connector("good").unwrap();
    st.runtime
        .secrets
        .refresh(
            [
                "secret://WCORE_MISSING".to_string(),
                "env://WCORE_ENV_PG".to_string(),
            ],
            database::secrets::Refetch::Invalidate,
        )
        .await;
    let cfg = st.config.read().await.clone();
    configuration::apply_config(&st, cfg, None).await.unwrap();
    assert_eq!(
        q(&st, "missing", "SELECT 1 AS one").await.unwrap().rows[0]["one"],
        json!(1)
    );
    assert_eq!(
        q(&st, "env_ref", "SELECT 1 AS one").await.unwrap().rows[0]["one"],
        json!(1)
    );
    assert!(Arc::ptr_eq(
        &good_before,
        &st.runtime.connector("good").unwrap()
    ));

    // A rotated value rebuilds that connection (a wrong password now fails).
    secrets
        .lock()
        .unwrap()
        .insert("secret://WCORE_PG".into(), "rotated-wrong".into());
    st.runtime
        .secrets
        .refresh(
            ["secret://WCORE_PG".to_string()],
            database::secrets::Refetch::Invalidate,
        )
        .await;
    let cfg = st.config.read().await.clone();
    configuration::apply_config(&st, cfg, None).await.unwrap();
    assert!(!Arc::ptr_eq(
        &good_before,
        &st.runtime.connector("good").unwrap()
    ));
    let err = q(&st, "good", "SELECT 1").await.unwrap_err();
    assert!(
        err.contains("auth") && !err.contains("rotated-wrong"),
        "{err}"
    );
}

/// The tunnel reopens its SSH session after the server drops it. Restarts
/// the bastion container it is given, so it never runs against a shared one:
/// set `DATABASE_E2E_RESTARTABLE_BASTION` (a container name, restarted with
/// `sg docker -c "docker restart …"`) and `…_PORT` (its host port), a copy of
/// the env's bastion on the env's default network.
#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_ssh_session_reconnects_on_the_next_connection() {
    let dir = env_or_skip!();
    let (Ok(container), Ok(port)) = (
        std::env::var("DATABASE_E2E_RESTARTABLE_BASTION"),
        std::env::var("DATABASE_E2E_RESTARTABLE_BASTION_PORT"),
    ) else {
        eprintln!("skipping: no restartable bastion");
        return;
    };
    let port: u16 = port.parse().unwrap();
    let st = state(json!({ "tunneled": {
        "driver": "postgres", "host": "pg", "port": 5432, "user": "shop", "password": PG_PW,
        "database": "shop", "tls": { "mode": "verify-full", "ca_cert": file(&dir, "tls/ca.crt") },
        "pool": { "max": 2, "idle_timeout_ms": 1000, "acquire_timeout_ms": 15000 },
        "ssh": { "host": "127.0.0.1", "port": port, "user": "tunnel",
                 "private_key": file(&dir, "ssh/id_ed25519") } } }))
    .await;
    let instance = "SELECT value FROM server_info WHERE key = 'instance'";
    assert_eq!(
        q(&st, "tunneled", instance).await.unwrap().rows[0]["value"],
        json!("pg-public")
    );

    let restarted = std::process::Command::new("sg")
        .args(["docker", "-c", &format!("docker restart {container}")])
        .output()
        .unwrap();
    assert!(restarted.status.success(), "{restarted:?}");
    // Wait for sshd to listen again.
    for _ in 0..60 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    // A pooled connection that rode the dead session may fail once; a fresh
    // one reconnects the session.
    let mut answered = false;
    for _ in 0..5 {
        if let Ok(resp) = q(&st, "tunneled", instance).await {
            assert_eq!(resp.rows[0]["value"], json!("pg-public"));
            answered = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert!(
        answered,
        "the tunnel did not come back after the bastion restarted"
    );
}

/// One caller's session state never reaches the next caller of a pooled
/// postgres connection: settings, advisory locks.
#[tokio::test(flavor = "multi_thread")]
async fn postgres_session_state_is_reset_between_pooled_calls() {
    let _dir = env_or_skip!();
    let mut entry = pg("shop", PG_PW);
    entry["pool"] = json!({ "max": 1 });
    let st = state(json!({ "pg": entry })).await;
    q(
        &st,
        "pg",
        "SELECT set_config('application_name', 'leaked', false)",
    )
    .await
    .unwrap();
    let name = q(&st, "pg", "SELECT current_setting('application_name') AS a")
        .await
        .unwrap();
    assert_ne!(
        name.rows[0]["a"],
        json!("leaked"),
        "a session setting outlived its call"
    );
    q(&st, "pg", "SELECT pg_advisory_lock(424242)")
        .await
        .unwrap();
    // The next checkout recycles the connection (unlocking everything)...
    q(&st, "pg", "SELECT 1").await.unwrap();
    // ...so another session can take the lock.
    let (other, conn) = tokio_postgres::connect(
        &format!("postgres://shop:{PG_PW}@127.0.0.1:55432/shop"),
        tokio_postgres::NoTls,
    )
    .await
    .unwrap();
    tokio::spawn(conn);
    let row = other
        .query_one("SELECT pg_try_advisory_lock(424242) AS got", &[])
        .await
        .unwrap();
    assert!(row.get::<_, bool>(0), "the advisory lock outlived its call");
    other
        .execute("SELECT pg_advisory_unlock_all()", &[])
        .await
        .unwrap();
}

/// A JSON array is a postgres array parameter, nested arrays dimensions.
#[tokio::test(flavor = "multi_thread")]
async fn json_arrays_bind_as_postgres_arrays() {
    let _dir = env_or_skip!();
    let st = state(json!({ "pg": pg("shop", PG_PW) })).await;
    let run = |sql: &'static str, params: Value| {
        let st = st.clone();
        async move {
            query::handle(
                &st,
                serde_json::from_value(json!({ "db": "pg", "sql": sql, "params": params }))
                    .unwrap(),
            )
            .await
        }
    };
    let r = run("SELECT $1::int[] AS a", json!([[1, 2, 3]]))
        .await
        .unwrap();
    assert_eq!(r.rows[0]["a"], json!([1, 2, 3]));
    let r = run("SELECT $1::text[] AS a", json!([["x", null, "y z"]]))
        .await
        .unwrap();
    assert_eq!(r.rows[0]["a"], json!(["x", null, "y z"]));
    let r = run("SELECT $1::int8[] AS a", json!([[[1, 2], [3, 4]]]))
        .await
        .unwrap();
    assert_eq!(r.rows[0]["a"], json!([["1", "2"], ["3", "4"]]));
    let r = run("SELECT cardinality($1::int[]) AS n", json!([[]]))
        .await
        .unwrap();
    assert_eq!(r.rows[0]["n"], json!(0));
    // A jsonb parameter still takes the document.
    let r = run("SELECT $1::jsonb AS j", json!([[1, "a"]]))
        .await
        .unwrap();
    assert_eq!(r.rows[0]["j"], json!([1, "a"]));
}

/// Integers beyond 2^53 (and beyond i64) are stored exactly, as JSON
/// numbers where they fit i64/u64 and as numeric strings everywhere.
#[tokio::test(flavor = "multi_thread")]
async fn big_integers_are_inserted_exactly() {
    let _dir = env_or_skip!();
    let st =
        state(json!({ "pg": pg_scratch("shop", PG_PW), "my": mysql_scratch("shop", PG_PW) })).await;
    let insert = |db: &'static str, values: Value| {
        let st = st.clone();
        async move {
            insert_row::handle(
                &st,
                serde_json::from_value(
                    json!({ "db": db, "table": "iii_wcore_big", "values": values }),
                )
                .unwrap(),
            )
            .await
        }
    };
    exec(&st, "pg", "DROP TABLE IF EXISTS iii_wcore_big").await;
    exec(
        &st,
        "pg",
        "CREATE TABLE iii_wcore_big (id bigint PRIMARY KEY, n numeric)",
    )
    .await;
    let r = insert(
        "pg",
        json!({ "id": 9007199254740993i64, "n": "123456789012345678901234567890" }),
    )
    .await
    .unwrap();
    let row = r.row.unwrap();
    assert_eq!(row["id"], json!("9007199254740993"));
    assert_eq!(row["n"], json!("123456789012345678901234567890"));
    let r = insert("pg", json!({ "id": "9007199254740995" }))
        .await
        .unwrap();
    assert_eq!(r.row.unwrap()["id"], json!("9007199254740995"));
    exec(&st, "pg", "DROP TABLE iii_wcore_big").await;

    exec(&st, "my", "DROP TABLE IF EXISTS iii_wcore_big").await;
    exec(
        &st,
        "my",
        "CREATE TABLE iii_wcore_big (id bigint unsigned PRIMARY KEY, n bigint)",
    )
    .await;
    let r = insert(
        "my",
        json!({ "id": 18446744073709551615u64, "n": 9007199254740993i64 }),
    )
    .await
    .unwrap();
    let row = r.row.expect("re-selected by the supplied key");
    assert_eq!(row["id"], json!("18446744073709551615"));
    assert_eq!(row["n"], json!(9007199254740993i64));
    let r = insert(
        "my",
        json!({ "id": "9007199254740993", "n": "-9007199254740993" }),
    )
    .await
    .unwrap();
    let row = r.row.unwrap();
    assert_eq!(row["id"], json!("9007199254740993"));
    assert_eq!(row["n"], json!(-9007199254740993i64));
    exec(&st, "my", "DROP TABLE iii_wcore_big").await;
}

/// No sqlite connection can attach another file, from `query` or `execute`.
#[tokio::test(flavor = "multi_thread")]
async fn sqlite_attach_is_refused_through_the_worker() {
    let _dir = env_or_skip!();
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("stolen.db");
    let st = state(json!({ "lite": { "driver": "sqlite", "database": tmp.path().join("main.db").display().to_string() } })).await;
    let attach = format!("ATTACH DATABASE '{}' AS stolen", target.display());
    let err = q(&st, "lite", &attach).await.unwrap_err();
    assert!(err.contains("DRIVER_ERROR"), "{err}");
    let err = execute::handle(
        &st,
        serde_json::from_value(json!({ "db": "lite", "sql": attach })).unwrap(),
    )
    .await
    .unwrap_err();
    assert!(
        err.contains("too many attached") || err.contains("DRIVER_ERROR"),
        "{err}"
    );
    assert!(!target.exists(), "ATTACH must not create the file");
}

/// How many rows of `table` the structured `filters` match (browseTable's
/// filtered count).
async fn matching(st: &AppState, db: &str, table: &str, filters: Value) -> i64 {
    let resp = browse::handle(
        st,
        serde_json::from_value(json!({
            "db": db, "table": table, "filters": filters, "page_size": 1, "include_total": true
        }))
        .unwrap(),
    )
    .await
    .unwrap_or_else(|e| panic!("{db}.{table} {filters}: {e}"));
    resp.total.expect("include_total")
}

/// Structured filters compare values as the column's type: on postgres each
/// value is bound as text and cast to the catalog type, so `total > 100` on a
/// NUMERIC, a date, a timestamptz, a uuid, an inet, a bool, an enum and an
/// int8 all mean what they say (they used to fail, or match every row).
/// Counts are the env seed's, checked against psql / mysql / sqlite.
#[tokio::test(flavor = "multi_thread")]
async fn structured_filters_compare_as_the_column_type() {
    let dir = env_or_skip!();
    let tmp = copy_sqlite(&dir);
    let st = state(json!({
        "pg": pg("shop", PG_PW),
        "my": mysql("shop", PG_PW),
        "lite": { "driver": "sqlite", "database": tmp.path().join("shop.db").display().to_string() },
    }))
    .await;

    // orders: 2000 rows on every driver.
    let uuid = "92be3413-b8bb-529c-93e2-f7c6dfb76fb9";
    for (db, since) in [
        ("pg", "2026-06-01T00:00:00Z"),
        ("my", "2026-06-01 00:00:00"),
        ("lite", "2026-06-01"),
    ] {
        let n = |filters: Value| {
            let st = st.clone();
            async move { matching(&st, db, "orders", filters).await }
        };
        assert_eq!(n(json!([])).await, 2000, "{db}");
        assert_eq!(
            n(json!([{ "column": "total", "op": "gt", "value": 100 }])).await,
            1613,
            "{db}"
        );
        assert_eq!(
            n(json!([{ "column": "total", "op": "gt", "value": 100.5 }])).await,
            1613,
            "{db}"
        );
        assert_eq!(
            n(json!([{ "column": "total", "op": "gt", "value": "100" }])).await,
            1613,
            "{db}"
        );
        assert_eq!(
            n(json!([{ "column": "status", "op": "equals", "value": "shipped" }])).await,
            200,
            "{db}"
        );
        assert_eq!(
            n(json!([{ "column": "status", "op": "in", "values": ["shipped", "delivered"] }]))
                .await,
            1400,
            "{db}"
        );
        assert_eq!(
            n(json!([{ "column": "status", "op": "not_in", "values": ["shipped", "delivered"] }]))
                .await,
            600,
            "{db}"
        );
        assert_eq!(
            n(json!([{ "column": "placed_at", "op": "gte", "value": since }])).await,
            897,
            "{db}"
        );
        assert_eq!(
            n(json!([{ "column": "shipped_on", "op": "between", "value": "2026-03-01", "value2": "2026-03-31" }])).await,
            159,
            "{db}"
        );
        assert_eq!(
            n(json!([{ "column": "id", "op": "between", "value": 10, "value2": "19" }])).await,
            10,
            "{db}"
        );
        assert_eq!(
            n(json!([{ "column": "id", "op": "in", "values": [1, "2", 3] }])).await,
            3,
            "{db}"
        );
        assert_eq!(
            n(json!([{ "column": "customer_id", "op": "equals", "value": 5 }])).await,
            45,
            "{db}"
        );
        assert_eq!(
            n(json!([{ "column": "external_ref", "op": "equals", "value": uuid }])).await,
            1,
            "{db}"
        );
        // Pattern operators on non-text columns compare the column's text.
        assert_eq!(
            n(json!([{ "column": "total", "op": "starts_with", "value": "1" }])).await,
            592,
            "{db}"
        );
        assert_eq!(
            n(json!([{ "column": "id", "op": "ends_with", "value": "7" }])).await,
            200,
            "{db}"
        );
    }

    // type_zoo (postgres): rows 1 and 3 carry values, row 2 is NULL.
    let n = |filters: Value| {
        let st = st.clone();
        async move { matching(&st, "pg", "type_zoo", filters).await }
    };
    let one = |column: &str, op: &str, value: Value| json!([{ "column": column, "op": op, "value": value }]);
    assert_eq!(
        n(one("c_bigint", "equals", json!("9007199254740993"))).await,
        1
    );
    assert_eq!(
        n(one("c_bigint", "lt", json!("-9007199254740992"))).await,
        1
    );
    assert_eq!(n(one("c_bigint", "gt", json!(0))).await, 1);
    assert_eq!(
        n(one("c_numeric", "equals", json!("12345678.1234"))).await,
        2
    );
    assert_eq!(n(one("c_numeric", "gte", json!(12345678.1234))).await, 2);
    assert_eq!(n(one("c_numeric", "lt", json!(1))).await, 0);
    assert_eq!(n(one("c_bool", "equals", json!(true))).await, 2);
    assert_eq!(n(one("c_bool", "equals", json!("false"))).await, 0);
    assert_eq!(n(one("c_date", "equals", json!("2026-02-28"))).await, 2);
    assert_eq!(n(one("c_date", "gt", json!("2026-03-01"))).await, 0);
    assert_eq!(
        n(one(
            "c_timestamptz",
            "equals",
            json!("2026-10-18T12:00:00Z")
        ))
        .await,
        2
    );
    assert_eq!(
        n(one(
            "c_timestamptz",
            "equals",
            json!("2026-10-18 14:00:00+02")
        ))
        .await,
        2
    );
    assert_eq!(
        n(one(
            "c_uuid",
            "equals",
            json!("6F1C2A8E-3B5D-4C7E-9F10-2A3B4C5D6E7F")
        ))
        .await,
        2
    );
    assert_eq!(n(one("c_inet", "equals", json!("192.168.0.1/24"))).await, 2);
    assert_eq!(n(one("c_inet", "contains", json!("168.0"))).await, 2);
    assert_eq!(n(one("c_enum", "equals", json!("shipped"))).await, 2);
    assert_eq!(n(one("c_enum", "not_equals", json!("pending"))).await, 2);
    assert_eq!(n(one("c_int", "equals", json!(2147483647))).await, 1);
    assert_eq!(
        n(json!([{ "column": "c_enum", "op": "in", "values": ["paid", "shipped"] }])).await,
        2
    );
    // A value the type cannot read is the caller's error, not every row.
    let err = browse::handle(
        &st,
        serde_json::from_value(json!({
            "db": "pg", "table": "type_zoo",
            "filters": [{ "column": "c_uuid", "op": "equals", "value": "not-a-uuid" }]
        }))
        .unwrap(),
    )
    .await
    .unwrap_err();
    assert!(err.contains("uuid"), "{err}");
    // So is a column the table does not have.
    let err = browse::handle(
        &st,
        serde_json::from_value(json!({
            "db": "pg", "table": "type_zoo",
            "filters": [{ "column": "c_nope", "op": "equals", "value": 1 }]
        }))
        .unwrap(),
    )
    .await
    .unwrap_err();
    assert!(err.contains("no column `c_nope`"), "{err}");
}

async fn columns_of(st: &AppState, db: &str, table: &str) -> Vec<Value> {
    let d = schema::describe_table(
        st,
        serde_json::from_value(json!({ "db": db, "table": table })).unwrap(),
    )
    .await
    .unwrap_or_else(|e| panic!("{db}.{table}: {e}"));
    d.columns
        .iter()
        .map(|c| serde_json::to_value(c).unwrap())
        .collect()
}

fn flags(cols: &[Value], name: &str) -> (bool, bool) {
    let c = cols
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no column {name}"));
    (
        c["auto_increment"].as_bool().unwrap(),
        c["generated"].as_bool().unwrap(),
    )
}

/// describeTable says which columns the database fills in (identity,
/// serial, AUTO_INCREMENT, the sqlite rowid) and which are computed; and
/// listTables carries the planner's row estimate where the catalog has one.
#[tokio::test(flavor = "multi_thread")]
async fn the_catalog_marks_filled_in_and_generated_columns() {
    let _dir = env_or_skip!();
    let tmp = tempfile::tempdir().unwrap();
    let lite = tmp.path().join("gen.db").display().to_string();
    let st = state(json!({
        "pg": pg_scratch("shop", PG_PW),
        "my": mysql_scratch("shop", PG_PW),
        "lite": { "driver": "sqlite", "database": lite, "pool": { "max": 1 } },
    }))
    .await;
    exec(&st, "pg", "DROP TABLE IF EXISTS iii_wcore_gen").await;
    exec(
        &st,
        "pg",
        "CREATE TABLE iii_wcore_gen (id bigint GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, \
         s serial, n int, total numeric GENERATED ALWAYS AS (n * 2) STORED)",
    )
    .await;
    exec(
        &st,
        "pg",
        "INSERT INTO iii_wcore_gen (n) SELECT g FROM generate_series(1, 50) g",
    )
    .await;
    exec(&st, "pg", "ANALYZE iii_wcore_gen").await;
    exec(&st, "my", "DROP TABLE IF EXISTS iii_wcore_gen").await;
    exec(
        &st,
        "my",
        "CREATE TABLE iii_wcore_gen (id bigint AUTO_INCREMENT PRIMARY KEY, n int, \
         at datetime DEFAULT CURRENT_TIMESTAMP, \
         v int GENERATED ALWAYS AS (n * 2) VIRTUAL, st int AS (n + 1) STORED)",
    )
    .await;
    exec(
        &st,
        "my",
        "INSERT INTO iii_wcore_gen (n) VALUES (1), (2), (3)",
    )
    .await;
    exec(&st, "my", "ANALYZE TABLE iii_wcore_gen").await;
    exec(
        &st,
        "lite",
        "CREATE TABLE gen (id INTEGER PRIMARY KEY, n int, v int GENERATED ALWAYS AS (n * 2), \
         st int AS (n + 1) STORED)",
    )
    .await;
    exec(
        &st,
        "lite",
        "CREATE TABLE pair (a INTEGER, b INTEGER, PRIMARY KEY (a, b))",
    )
    .await;
    exec(
        &st,
        "lite",
        "CREATE TABLE norowid (id INTEGER PRIMARY KEY, x) WITHOUT ROWID",
    )
    .await;
    exec(&st, "lite", "CREATE TABLE big (id BIGINT PRIMARY KEY)").await;

    let c = columns_of(&st, "pg", "iii_wcore_gen").await;
    assert_eq!(flags(&c, "id"), (true, false));
    assert_eq!(flags(&c, "s"), (true, false));
    assert_eq!(flags(&c, "n"), (false, false));
    assert_eq!(flags(&c, "total"), (false, true));
    let c = columns_of(&st, "my", "iii_wcore_gen").await;
    assert_eq!(flags(&c, "id"), (true, false));
    assert_eq!(flags(&c, "n"), (false, false));
    // `DEFAULT CURRENT_TIMESTAMP` is DEFAULT_GENERATED in mysql 8: not generated.
    assert_eq!(flags(&c, "at"), (false, false));
    assert_eq!(flags(&c, "v"), (false, true));
    assert_eq!(flags(&c, "st"), (false, true));
    let c = columns_of(&st, "lite", "gen").await;
    assert_eq!(flags(&c, "id"), (true, false));
    assert_eq!(flags(&c, "n"), (false, false));
    assert_eq!(flags(&c, "v"), (false, true));
    assert_eq!(flags(&c, "st"), (false, true));
    for (table, col) in [("pair", "a"), ("norowid", "id"), ("big", "id")] {
        assert_eq!(
            flags(&columns_of(&st, "lite", table).await, col),
            (false, false),
            "{table}"
        );
    }

    let estimate = |db: &'static str| {
        let st = st.clone();
        async move {
            let r = schema::list_tables(&st, serde_json::from_value(json!({ "db": db })).unwrap())
                .await
                .unwrap();
            let t = r
                .tables
                .iter()
                .find(|t| t.name == "iii_wcore_gen" || t.name == "gen")
                .map(|t| serde_json::to_value(t).unwrap())
                .unwrap();
            t.get("row_count_estimate").cloned()
        }
    };
    assert_eq!(estimate("pg").await, Some(json!(50)));
    assert_eq!(estimate("my").await, Some(json!(3)));
    assert_eq!(estimate("lite").await, None, "sqlite has no cheap estimate");

    exec(&st, "pg", "DROP TABLE iii_wcore_gen").await;
    exec(&st, "my", "DROP TABLE iii_wcore_gen").await;
}
