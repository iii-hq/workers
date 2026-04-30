import type { ISdk } from 'iii-sdk';
import type { DriverKey, Dialect } from './dialect.ts';

export interface CaseContext {
  driver: DriverKey;
  dialect: Dialect;
  /** Calls a database worker function; returns parsed JSON or throws on engine error. */
  call: (functionId: string, payload: unknown) => Promise<any>;
  /** Resolves once N rows for `driver` have arrived via the query-poll sink. */
  waitForRows: (n: number, timeoutMs: number) => Promise<Array<Record<string, unknown>>>;
  /** Resets the per-driver received-rows buffer used by `waitForRows`. */
  resetReceived: () => void;
  /** Direct SDK access for trigger-config edge-case tests. */
  iii: ISdk;
  /**
   * Asserts that `fn()` rejects and the rejection message contains `expectedCode`.
   * The worker wraps DbError as `IIIError::Handler(json_string)`, which the engine
   * surfaces as the JS Error message; substring match is more resilient than
   * strict JSON parsing across SDK versions.
   */
  expectError: (fn: () => Promise<unknown>, expectedCode: string) => Promise<void>;
  /**
   * Resolves true if NO rows arrive on the polling sink within `timeoutMs`.
   * Used by trigger validation tests to assert a broken trigger never dispatches.
   */
  expectSilence: (timeoutMs: number) => Promise<void>;
}

export interface TestCase {
  name: string;
  /** If set, this case only runs on the listed drivers; otherwise it runs on all. */
  applies?: readonly DriverKey[];
  run(ctx: CaseContext): Promise<void>;
}

export function expectEqual(actual: unknown, expected: unknown, msg: string): void {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`${msg}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}

export function expect(cond: boolean, msg: string): asserts cond {
  if (!cond) throw new Error(msg);
}

export const SCHEMA_RESET: TestCase = {
  name: 'schema-reset',
  async run({ driver, dialect, call }) {
    await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS outbox' });
    await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS t' });
    // The query-poll trigger persists cursor state in __iii_cursors. Without
    // dropping it here, stale cursor values from a prior run survive (Postgres
    // and MySQL via docker volumes; SQLite via ./data/iii.db) and cause the
    // first poll to filter out the freshly-inserted ids — producing a "got 0
    // rows" timeout. Dropping the table here makes the test idempotent across
    // runs without requiring a manual `docker compose down -v && rm data/iii.db`.
    await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS __iii_cursors' });
    await call('iii-database::execute', {
      db: driver,
      sql: `CREATE TABLE t (id ${dialect.idColumnDDL()}, n INT NOT NULL)`,
    });
    await call('iii-database::execute', {
      db: driver,
      sql: `CREATE TABLE outbox (id ${dialect.idColumnDDL()}, body TEXT NOT NULL)`,
    });
  },
};

export const FUNCTION_CASES: TestCase[] = [
  {
    name: 'query SELECT 1',
    async run({ driver, call }) {
      const r = await call('iii-database::query', { db: driver, sql: 'SELECT 1 AS n' });
      expectEqual(r.row_count, 1, 'row_count');
      expect(Array.isArray(r.columns), 'columns is array');
      expect(r.columns.length === 1, 'one column');
      expectEqual(r.columns[0].name, 'n', 'column name');
      // Value may be number or numeric string depending on driver — accept either.
      const v = r.rows[0].n;
      expect(v === 1 || v === '1', `n value: ${JSON.stringify(v)}`);
    },
  },
  {
    name: 'execute INSERT (multi-row)',
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1);
      const ph2 = dialect.placeholder(2);
      const r = await call('iii-database::execute', {
        db: driver,
        sql: `INSERT INTO t (n) VALUES (${ph1}), (${ph2})`,
        params: [10, 20],
      });
      expectEqual(r.affected_rows, 2, 'affected_rows after multi-row insert');
    },
  },
  {
    name: 'query SELECT after insert',
    async run({ driver, call }) {
      const r = await call('iii-database::query', {
        db: driver,
        sql: 'SELECT n FROM t ORDER BY id',
      });
      expectEqual(r.row_count, 2, 'two rows returned');
      const ns = r.rows.map((row: any) => Number(row.n));
      expectEqual(ns, [10, 20], 'row values');
    },
  },
  {
    name: 'prepareStatement + runStatement',
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1);
      const prep = await call('iii-database::prepareStatement', {
        db: driver,
        sql: `SELECT n FROM t WHERE n = ${ph1}`,
      });
      const handleId = prep.handle?.id;
      expect(typeof handleId === 'string' && handleId.length > 0, 'handle id present');

      const r1 = await call('iii-database::runStatement', { handle_id: handleId, params: [10] });
      expectEqual(r1.row_count, 1, 'first runStatement row_count');
      expectEqual(Number(r1.rows[0].n), 10, 'first runStatement value');

      const r2 = await call('iii-database::runStatement', { handle_id: handleId, params: [20] });
      expectEqual(r2.row_count, 1, 'second runStatement row_count');
      expectEqual(Number(r2.rows[0].n), 20, 'second runStatement value');
    },
  },
  {
    name: 'transaction commit',
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1);
      const r = await call('iii-database::transaction', {
        db: driver,
        statements: [
          { sql: `UPDATE t SET n = n + 1 WHERE n = ${ph1}`, params: [10] },
          { sql: `UPDATE t SET n = n + 1 WHERE n = ${ph1}`, params: [20] },
        ],
      });
      expectEqual(r.committed, true, 'committed');
      const verify = await call('iii-database::query', { db: driver, sql: 'SELECT n FROM t ORDER BY id' });
      const ns = verify.rows.map((row: any) => Number(row.n));
      expectEqual(ns, [11, 21], 'post-commit values');
    },
  },
  {
    name: 'transaction rollback',
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1);
      const before = await call('iii-database::query', { db: driver, sql: 'SELECT COUNT(*) AS c FROM t' });
      const beforeCount = Number(before.rows[0].c);

      const r = await call('iii-database::transaction', {
        db: driver,
        statements: [
          { sql: `INSERT INTO t (n) VALUES (${ph1})`, params: [999] },
          // Second statement violates NOT NULL — forces rollback.
          { sql: `INSERT INTO t (n) VALUES (${ph1})`, params: [null] },
        ],
      });
      expectEqual(r.committed, false, 'committed=false');
      expectEqual(r.failed_index, 1, 'failed_index=1');
      expect(typeof r.error === 'object' && r.error !== null, 'structured error object');

      const after = await call('iii-database::query', { db: driver, sql: 'SELECT COUNT(*) AS c FROM t' });
      expectEqual(Number(after.rows[0].c), beforeCount, 'row count unchanged after rollback');
    },
  },
];

export const POLLING_CASE: TestCase = {
  name: 'query-poll dispatches new rows incrementally',
  async run({ driver, dialect, call, waitForRows, resetReceived }) {
    resetReceived();
    const ph1 = dialect.placeholder(1);
    const ph2 = dialect.placeholder(2);
    const ph3 = dialect.placeholder(3);
    // Seed 3 rows.
    await call('iii-database::execute', {
      db: driver,
      sql: `INSERT INTO outbox (body) VALUES (${ph1}), (${ph2}), (${ph3})`,
      params: ['a', 'b', 'c'],
    });

    const first = await waitForRows(3, 5_000);
    expectEqual(first.length, 3, 'first batch row count');
    const bodies1 = first.map((r) => r.body);
    expectEqual(bodies1, ['a', 'b', 'c'], 'first batch body order');

    // Insert 2 more after the first batch was acked.
    resetReceived();
    await call('iii-database::execute', {
      db: driver,
      sql: `INSERT INTO outbox (body) VALUES (${ph1}), (${ph2})`,
      params: ['d', 'e'],
    });
    const second = await waitForRows(2, 5_000);
    expectEqual(second.length, 2, 'second batch row count');
    const bodies2 = second.map((r) => r.body);
    expectEqual(bodies2, ['d', 'e'], 'second batch is delta only');
  },
};
