import type { TestCase } from './cases.ts';
import { expect } from './cases.ts';

/**
 * Trigger config + lifecycle edge cases. Gated to sqlite_db because the
 * trigger logic is driver-agnostic and sqlite is the fastest path; running
 * across all three drivers buys nothing here.
 *
 * SDK note: `iii.registerTrigger` returns synchronously without awaiting the
 * worker's `register_trigger` handler, so worker-side validation errors do
 * NOT surface as a JS rejection. We test broken triggers by *absence of
 * dispatches* — register, seed rows, assert nothing arrives.
 */
export const TRIGGER_CASES: TestCase[] = [
  {
    name: 'trigger with invalid cursor_table never dispatches',
    applies: ['sqlite_db'],
    async run({ driver, dialect, iii, call, resetReceived, expectSilence }) {
      // Use a unique scratch table per test to stay isolated from the polling case's outbox.
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS tg_bad_cursor_table' });
      await call('iii-database::execute', {
        db: driver,
        sql: `CREATE TABLE tg_bad_cursor_table (id ${dialect.idColumnDDL()}, body TEXT NOT NULL)`,
      });
      resetReceived();
      const ph1 = dialect.placeholder(1);
      // 'bad-name' contains a hyphen → fails validate_sql_identifier. Per
      // query_poll.rs:70-78, validate() rejects at first tick before any
      // dispatch can land.
      const handle = iii.registerTrigger({
        type: 'iii-database::query-poll',
        function_id: 'harness::on_outbox_row',
        config: {
          trigger_id: `harness-bad-cursor-table-${driver}`,
          db: driver,
          sql: `SELECT id, body, '${driver}' AS db FROM tg_bad_cursor_table WHERE id > COALESCE(${ph1}, 0) ORDER BY id LIMIT 50`,
          interval_ms: 200,
          cursor_column: 'id',
          cursor_table: 'bad-name',
        },
      });
      try {
        await call('iii-database::execute', {
          db: driver,
          sql: 'INSERT INTO tg_bad_cursor_table (body) VALUES (?)',
          params: ['x'],
        });
        await expectSilence(1500);
      } finally {
        try { handle.unregister(); } catch { /* ignore */ }
        await call('iii-database::execute', { db: driver, sql: 'DROP TABLE tg_bad_cursor_table' });
      }
    },
  },
  {
    name: 'trigger with cursor_column not in result never dispatches',
    applies: ['sqlite_db'],
    async run({ driver, dialect, iii, call, resetReceived, expectSilence }) {
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS tg_bad_col' });
      await call('iii-database::execute', {
        db: driver,
        sql: `CREATE TABLE tg_bad_col (id ${dialect.idColumnDDL()}, body TEXT NOT NULL)`,
      });
      resetReceived();
      const ph1 = dialect.placeholder(1);
      // SQL only selects `id` and `body`, but cursor_column is `nonexistent`.
      // Per query_poll.rs:107-116, run_one_tick errors at column-lookup time
      // and the loop logs+swallows. No dispatch ever lands.
      const handle = iii.registerTrigger({
        type: 'iii-database::query-poll',
        function_id: 'harness::on_outbox_row',
        config: {
          trigger_id: `harness-bad-col-${driver}`,
          db: driver,
          sql: `SELECT id, body, '${driver}' AS db FROM tg_bad_col WHERE id > COALESCE(${ph1}, 0) ORDER BY id LIMIT 50`,
          interval_ms: 200,
          cursor_column: 'nonexistent',
        },
      });
      try {
        await call('iii-database::execute', {
          db: driver,
          sql: 'INSERT INTO tg_bad_col (body) VALUES (?)',
          params: ['y'],
        });
        await expectSilence(1500);
      } finally {
        try { handle.unregister(); } catch { /* ignore */ }
        await call('iii-database::execute', { db: driver, sql: 'DROP TABLE tg_bad_col' });
      }
    },
  },
  {
    name: 'unregister of already-unregistered trigger does not throw',
    applies: ['sqlite_db'],
    async run({ driver, dialect, iii }) {
      const ph1 = dialect.placeholder(1);
      const handle = iii.registerTrigger({
        type: 'iii-database::query-poll',
        function_id: 'harness::on_outbox_row',
        config: {
          trigger_id: `harness-double-unreg-${driver}`,
          db: driver,
          sql: `SELECT id, body, '${driver}' AS db FROM outbox WHERE id > COALESCE(${ph1}, 0) ORDER BY id LIMIT 50`,
          interval_ms: 1000,
          cursor_column: 'id',
        },
      });
      // First unregister: real cleanup.
      handle.unregister();
      // Second unregister: should be a no-op rather than throwing.
      let threw: unknown = null;
      try {
        handle.unregister();
      } catch (e) {
        threw = e;
      }
      expect(threw === null, `second unregister threw: ${threw}`);
    },
  },
];
