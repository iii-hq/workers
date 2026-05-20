import type { TestCase } from './cases.ts';
import { expect, expectEqual } from './cases.ts';

/**
 * Interactive-transaction surface end-to-end cases. Drives the full
 * lifecycle (begin → query → execute → commit/rollback) against each
 * driver, plus the negative paths: TRANSACTION_NOT_FOUND after
 * finalization, COMMIT/ROLLBACK SQL rejection through transactionExecute,
 * and timeout-driven auto-rollback.
 *
 * Each case creates and drops its own scratch table so it stays
 * independent of the function-suite's shared `t`.
 */
export const INTERACTIVE_TX_CASES: TestCase[] = [
  {
    name: 'interactive tx commit lands writes',
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1);
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS itx_commit' });
      await call('iii-database::execute', {
        db: driver,
        sql: `CREATE TABLE itx_commit (id ${dialect.idColumnDDL()}, n INT NOT NULL)`,
      });
      try {
        const begin = await call('iii-database::beginTransaction', { db: driver });
        const id = begin.transaction?.id;
        expect(typeof id === 'string' && id.length === 36, `tx id must be UUID, got ${id}`);

        const ins = await call('iii-database::transactionExecute', {
          transaction_id: id,
          sql: `INSERT INTO itx_commit (n) VALUES (${ph1})`,
          params: [42],
        });
        expectEqual(ins.affected_rows, 1, 'insert affected_rows inside tx');

        // Read-your-writes: the INSERT must be visible from inside the same tx.
        const ryw = await call('iii-database::transactionQuery', {
          transaction_id: id,
          sql: 'SELECT n FROM itx_commit',
        });
        expectEqual(ryw.row_count, 1, 'tx sees its own insert');
        expectEqual(Number(ryw.rows[0].n), 42, 'tx read-your-writes value');

        const c = await call('iii-database::commitTransaction', { transaction_id: id });
        expectEqual(c.committed, true, 'commit returns committed=true');

        // Verify outside the tx that the write landed.
        const verify = await call('iii-database::query', { db: driver, sql: 'SELECT n FROM itx_commit' });
        expectEqual(verify.row_count, 1, 'committed write visible after commit');
        expectEqual(Number(verify.rows[0].n), 42, 'post-commit value');
      } finally {
        await call('iii-database::execute', { db: driver, sql: 'DROP TABLE itx_commit' });
      }
    },
  },
  {
    name: 'interactive tx rollback discards writes',
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1);
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS itx_rollback' });
      await call('iii-database::execute', {
        db: driver,
        sql: `CREATE TABLE itx_rollback (id ${dialect.idColumnDDL()}, n INT NOT NULL)`,
      });
      try {
        const begin = await call('iii-database::beginTransaction', { db: driver });
        const id = begin.transaction.id;
        await call('iii-database::transactionExecute', {
          transaction_id: id,
          sql: `INSERT INTO itx_rollback (n) VALUES (${ph1})`,
          params: [99],
        });
        const r = await call('iii-database::rollbackTransaction', { transaction_id: id });
        expectEqual(r.rolled_back, true, 'rollback returns rolled_back=true');

        const verify = await call('iii-database::query', {
          db: driver,
          sql: 'SELECT COUNT(*) AS c FROM itx_rollback',
        });
        expectEqual(Number(verify.rows[0].c), 0, 'rolled-back insert not visible');
      } finally {
        await call('iii-database::execute', { db: driver, sql: 'DROP TABLE itx_rollback' });
      }
    },
  },
  {
    name: 'interactive tx commit then query returns TRANSACTION_NOT_FOUND',
    async run({ driver, call, expectError }) {
      const begin = await call('iii-database::beginTransaction', { db: driver });
      const id = begin.transaction.id;
      await call('iii-database::commitTransaction', { transaction_id: id });
      await expectError(
        () => call('iii-database::transactionQuery', { transaction_id: id, sql: 'SELECT 1' }),
        'TRANSACTION_NOT_FOUND',
      );
    },
  },
  {
    name: 'interactive tx double-commit returns TRANSACTION_NOT_FOUND',
    async run({ driver, call, expectError }) {
      const begin = await call('iii-database::beginTransaction', { db: driver });
      const id = begin.transaction.id;
      await call('iii-database::commitTransaction', { transaction_id: id });
      await expectError(
        () => call('iii-database::commitTransaction', { transaction_id: id }),
        'TRANSACTION_NOT_FOUND',
      );
    },
  },
  {
    name: 'transactionExecute rejects COMMIT/ROLLBACK SQL with INVALID_PARAM',
    async run({ driver, call, expectError }) {
      const begin = await call('iii-database::beginTransaction', { db: driver });
      const id = begin.transaction.id;
      try {
        await expectError(
          () => call('iii-database::transactionExecute', { transaction_id: id, sql: 'COMMIT' }),
          'INVALID_PARAM',
        );
        await expectError(
          () => call('iii-database::transactionExecute', { transaction_id: id, sql: 'ROLLBACK' }),
          'INVALID_PARAM',
        );
        await expectError(
          () => call('iii-database::transactionExecute', { transaction_id: id, sql: 'BEGIN' }),
          'INVALID_PARAM',
        );
      } finally {
        // The tx is still open (rejections don't finalize); clean up.
        try {
          await call('iii-database::rollbackTransaction', { transaction_id: id });
        } catch {/* ignore */}
      }
    },
  },
  {
    name: 'interactive tx beginTransaction unknown isolation returns INVALID_PARAM',
    async run({ driver, call, expectError }) {
      await expectError(
        () =>
          call('iii-database::beginTransaction', { db: driver, isolation: 'bogus_level' }),
        'INVALID_PARAM',
      );
    },
  },
  {
    // Timeout-driven auto-rollback. timeout_ms is set low (300 ms) so the
    // background watcher (1 s poll cadence) fires within a couple of ticks
    // and the next call sees TRANSACTION_NOT_FOUND. Skipped on pg/mysql for
    // brevity — the timeout logic is driver-agnostic and sqlite is the
    // fastest path.
    name: 'interactive tx timeout auto-rolls-back and frees the id',
    applies: ['sqlite_db'],
    async run({ driver, call, expectError }) {
      const begin = await call('iii-database::beginTransaction', {
        db: driver,
        timeout_ms: 300,
      });
      const id = begin.transaction.id;
      // Wait past the deadline + one watcher tick + a safety margin.
      await new Promise((r) => setTimeout(r, 1800));
      await expectError(
        () => call('iii-database::transactionQuery', { transaction_id: id, sql: 'SELECT 1' }),
        'TRANSACTION_NOT_FOUND',
      );
    },
  },
];
