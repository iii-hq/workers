import type { TestCase } from './cases.ts';
import { expect, expectEqual } from './cases.ts';

/**
 * Transaction edge cases. The function suite covers commit + rollback at
 * failed_index=1; these target shapes the function suite leaves alone:
 * empty / single-statement / mixed-read-write / failure-at-index-0.
 *
 * Each test creates its own scratch table to stay independent of `t`.
 */
export const TRANSACTION_EDGE_CASES: TestCase[] = [
  {
    name: 'transaction with empty statements array',
    async run({ driver, call }) {
      // Spec ambiguity: an empty txn is a no-op. Drivers commit an empty
      // transaction without error; the worker should pass that through.
      const r = await call('iii-database::transaction', { db: driver, statements: [] });
      expectEqual(r.committed, true, 'empty transaction commits');
      expectEqual(Array.isArray(r.results) ? r.results.length : 0, 0, 'no results');
    },
  },
  {
    name: 'transaction with single statement',
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1);
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS tx_single' });
      await call('iii-database::execute', {
        db: driver,
        sql: `CREATE TABLE tx_single (id ${dialect.idColumnDDL()}, n INT NOT NULL)`,
      });
      const r = await call('iii-database::transaction', {
        db: driver,
        statements: [{ sql: `INSERT INTO tx_single (n) VALUES (${ph1})`, params: [7] }],
      });
      expectEqual(r.committed, true, 'committed=true for single-statement txn');
      const verify = await call('iii-database::query', {
        db: driver,
        sql: 'SELECT n FROM tx_single',
      });
      expectEqual(Number(verify.rows[0].n), 7, 'row landed');
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE tx_single' });
    },
  },
  {
    name: 'transaction read-your-writes (mixed INSERT/SELECT/INSERT)',
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1);
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS tx_ryw' });
      await call('iii-database::execute', {
        db: driver,
        sql: `CREATE TABLE tx_ryw (id ${dialect.idColumnDDL()}, n INT NOT NULL)`,
      });
      const r = await call('iii-database::transaction', {
        db: driver,
        statements: [
          { sql: `INSERT INTO tx_ryw (n) VALUES (${ph1})`, params: [100] },
          { sql: `SELECT n FROM tx_ryw`, params: [] },
          { sql: `INSERT INTO tx_ryw (n) VALUES (${ph1})`, params: [200] },
        ],
      });
      expectEqual(r.committed, true, 'committed=true for mixed txn');
      expect(Array.isArray(r.results), 'results is array');
      expectEqual(r.results.length, 3, 'three results');
      // Note: txn results carry rows positionally (Vec<Vec<Value>>) per
      // transaction.rs:64-67 — no column names. This differs from query/runStatement
      // which return column-keyed objects. The first SELECT row is `[100]`, not `{n: 100}`.
      const selectResult = r.results[1];
      expect(
        Array.isArray(selectResult.rows) && selectResult.rows.length === 1,
        `select sees exactly one row, got ${JSON.stringify(selectResult)}`,
      );
      const firstRow = selectResult.rows[0];
      expect(Array.isArray(firstRow), `row is positional array, got ${JSON.stringify(firstRow)}`);
      expectEqual(Number(firstRow[0]), 100, 'read-your-writes: select sees the just-inserted value');
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE tx_ryw' });
    },
  },
  {
    name: 'transaction failure at index 0 reports failed_index=0',
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1);
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS tx_fail0' });
      await call('iii-database::execute', {
        db: driver,
        sql: `CREATE TABLE tx_fail0 (id ${dialect.idColumnDDL()}, n INT NOT NULL)`,
      });
      // First statement violates NOT NULL; second never runs. failed_index must be 0.
      const r = await call('iii-database::transaction', {
        db: driver,
        statements: [
          { sql: `INSERT INTO tx_fail0 (n) VALUES (${ph1})`, params: [null] },
          { sql: `INSERT INTO tx_fail0 (n) VALUES (${ph1})`, params: [99] },
        ],
      });
      expectEqual(r.committed, false, 'committed=false');
      expectEqual(r.failed_index, 0, 'failed_index=0');
      // Confirm rollback: zero rows.
      const verify = await call('iii-database::query', {
        db: driver,
        sql: 'SELECT COUNT(*) AS c FROM tx_fail0',
      });
      expectEqual(Number(verify.rows[0].c), 0, 'rollback dropped all writes');
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE tx_fail0' });
    },
  },
];
