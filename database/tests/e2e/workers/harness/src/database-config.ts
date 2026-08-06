/**
 * E2e `database` configuration value — single source of truth for the
 * configuration worker entry seeded before the database worker starts.
 */

const DEFAULT_POOL = {
  max: 10,
  idle_timeout_ms: 30_000,
  acquire_timeout_ms: 5_000,
} as const;

export const DATABASE_CONFIG_ID = 'database';

export const DATABASE_CONFIG_VALUE = {
  databases: {
    sqlite_db: {
      url: 'sqlite:./data/iii.db',
      pool: { ...DEFAULT_POOL },
    },
    pg_db: {
      url: process.env.TEST_POSTGRES_URL ?? 'postgres://iii:iii@127.0.0.1:55432/iii_test',
      pool: { ...DEFAULT_POOL },
      // Local docker postgres uses a self-signed cert that doesn't chain to
      // any system CA. The worker's tls.mode defaults to require.
      tls: { mode: 'disable' as const },
    },
    // Same postgres instance as pg_db, but with native change capture:
    // database triggers + LISTEN/NOTIFY instead of SQL classification. Writes
    // arriving through OTHER handles (pg_db) are, from this handle's point of
    // view, external clients — which is exactly what the native cases prove.
    pg_native_db: {
      url: process.env.TEST_POSTGRES_URL ?? 'postgres://iii:iii@127.0.0.1:55432/iii_test',
      pool: { ...DEFAULT_POOL },
      tls: { mode: 'disable' as const },
      capture: 'native' as const,
    },
    // Same file as sqlite_db, native capture via changelog + fs watch.
    sqlite_native_db: {
      url: 'sqlite:./data/iii.db',
      pool: { ...DEFAULT_POOL },
      capture: 'native' as const,
    },
    // Same mysql instance as mysql_db, native capture via the binlog
    // replication stream. Requires the replication grants installed by
    // mysql-init/grant-replication.sql on first compose volume init.
    mysql_native_db: {
      url: process.env.TEST_MYSQL_URL ?? 'mysql://iii:iii@127.0.0.1:53306/iii_test',
      pool: { ...DEFAULT_POOL },
      tls: { mode: 'disable' as const },
      capture: 'native' as const,
    },
    mysql_db: {
      url: process.env.TEST_MYSQL_URL ?? 'mysql://iii:iii@127.0.0.1:53306/iii_test',
      pool: { ...DEFAULT_POOL },
      // Local docker mysql:8.4 ships an auto-generated self-signed cert.
      tls: { mode: 'disable' as const },
    },
  },
  // Tiny query-history caps so cases-history.ts exercises entry/byte
  // rotation with a handful of queries. Worker defaults: 200 / 262144.
  history_max_entries: 5,
  history_max_bytes: 8192,
} as const;
