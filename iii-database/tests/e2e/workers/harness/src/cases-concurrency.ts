import type { TestCase } from './cases.ts';
import { expect, expectEqual } from './cases.ts';

/**
 * Pool + handle concurrency cases. Pool max=10 / acquire_timeout=5s per
 * config.yaml. Tests run serially per driver, so pool exhaustion in one
 * test cannot leak into another within the same driver pass.
 */
export const CONCURRENCY_CASES: TestCase[] = [
  {
    name: '10 parallel SELECT 1 against same db',
    async run({ driver, call }) {
      // Pool max = 10. All 10 queries should finish without queue contention.
      const promises = Array.from({ length: 10 }, () =>
        call('iii-database::query', { db: driver, sql: 'SELECT 1 AS n' }),
      );
      const results = await Promise.all(promises);
      expectEqual(results.length, 10, '10 results');
      for (const r of results) {
        expectEqual(r.row_count, 1, 'each query returned 1 row');
      }
    },
  },
  {
    name: 'prepared statement reused 50 times sequentially',
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1);
      // Bare `SELECT ${ph1} AS v` defaults the param to `text` on Postgres
      // because the polymorphic param has no type context — sending int4 binary
      // there triggers SQL state 22021 ("character not in repertoire") at decode.
      // The fix: anchor the param's type via a real schema column (matches the
      // existing prepareStatement + runStatement test in cases.ts).
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS bx_prep_50x' });
      await call('iii-database::execute', { db: driver, sql: 'CREATE TABLE bx_prep_50x (n INT NOT NULL)' });
      // Seed 50 rows so each iteration can match a unique value.
      for (let i = 0; i < 50; i++) {
        await call('iii-database::execute', {
          db: driver,
          sql: `INSERT INTO bx_prep_50x (n) VALUES (${ph1})`,
          params: [i],
        });
      }
      const prep = await call('iii-database::prepareStatement', {
        db: driver,
        sql: `SELECT n FROM bx_prep_50x WHERE n = ${ph1} LIMIT 1`,
      });
      const handleId = prep.handle?.id;
      expect(typeof handleId === 'string' && handleId.length > 0, 'handle id present');
      // Default TTL is far longer than 50 iterations; this catches handle-cache
      // lifetime bugs where a hot handle gets evicted mid-loop.
      for (let i = 0; i < 50; i++) {
        const r = await call('iii-database::runStatement', { handle_id: handleId, params: [i] });
        expectEqual(r.row_count, 1, `iter ${i} row_count`);
        expectEqual(Number(r.rows[0].n), i, `iter ${i} value`);
      }
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE bx_prep_50x' });
    },
  },
  {
    name: 'pool exhaustion surfaces POOL_TIMEOUT',
    // SQLite has no SLEEP equivalent that holds a connection in the same way;
    // gating to pg+mysql keeps this driver-correct. acquire_timeout is 5s in
    // config.yaml; we hold connections 6s to force the timeout.
    applies: ['pg_db', 'mysql_db'],
    async run({ driver, call }) {
      const sleepSql = driver === 'pg_db' ? 'SELECT pg_sleep(6)' : 'SELECT SLEEP(6)';
      // 12 concurrent queries against a max=10 pool. Acquire timeout = 5s, query
      // hold = 6s, so the 11th and 12th waiters must time out before any holder
      // releases. We assert at least one rejection contains POOL_TIMEOUT — the
      // exact count depends on scheduler timing.
      const promises = Array.from({ length: 12 }, () =>
        call('iii-database::query', { db: driver, sql: sleepSql, timeout_ms: 30_000 }),
      );
      const settled = await Promise.allSettled(promises);
      const rejected = settled.filter((s) => s.status === 'rejected') as PromiseRejectedResult[];
      const fulfilled = settled.filter((s) => s.status === 'fulfilled');
      expect(
        rejected.length >= 1,
        `expected at least 1 POOL_TIMEOUT rejection, got 0 (fulfilled=${fulfilled.length})`,
      );
      const sawPoolTimeout = rejected.some((r) => {
        const msg = (r.reason as any)?.message ?? String(r.reason);
        return msg.includes('POOL_TIMEOUT');
      });
      expect(
        sawPoolTimeout,
        `at least one rejection should be POOL_TIMEOUT; reasons: ${rejected
          .map((r) => (r.reason as any)?.message ?? String(r.reason))
          .join(' | ')}`,
      );
    },
  },
];
