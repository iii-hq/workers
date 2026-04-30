import type { TestCase } from './cases.ts';
import { expect, expectEqual } from './cases.ts';

/**
 * Protocol-misuse cases — assert the worker returns the documented error code
 * for malformed or out-of-bounds requests rather than crashing or silently
 * succeeding. The worker wraps DbError as IIIError::Handler(json_string) per
 * error.rs:58-64; the harness's `expectError` matches the error code as a
 * substring of the rejection message.
 */
export const PROTOCOL_CASES: TestCase[] = [
  {
    name: 'unknown db rejects with UNKNOWN_DB',
    async run({ call, expectError }) {
      await expectError(
        () => call('iii-database::query', { db: 'no_such_db', sql: 'SELECT 1' }),
        'UNKNOWN_DB',
      );
    },
  },
  {
    name: 'empty SQL rejects with DRIVER_ERROR',
    async run({ driver, call, expectError }) {
      await expectError(
        () => call('iii-database::query', { db: driver, sql: '' }),
        'DRIVER_ERROR',
      );
    },
  },
  {
    name: 'unknown handle id rejects with STATEMENT_NOT_FOUND',
    async run({ call, expectError }) {
      await expectError(
        () =>
          call('iii-database::runStatement', {
            handle_id: '00000000-0000-0000-0000-000000000000',
            params: [],
          }),
        'STATEMENT_NOT_FOUND',
      );
    },
  },
  {
    name: 'runStatement with wrong param count rejects',
    async run({ driver, dialect, call, expectError }) {
      const ph1 = dialect.placeholder(1);
      // Use a fresh prepare so test order doesn't matter.
      const prep = await call('iii-database::prepareStatement', {
        db: driver,
        sql: `SELECT ${ph1} AS v`,
      });
      const handleId = prep.handle?.id;
      expect(typeof handleId === 'string' && handleId.length > 0, 'handle id present');
      // Driver should reject param-count mismatch. Exact code varies by driver
      // (DRIVER_ERROR with inner SQL state); we match on DRIVER_ERROR.
      await expectError(
        () => call('iii-database::runStatement', { handle_id: handleId, params: [] }),
        'DRIVER_ERROR',
      );
    },
  },
  {
    name: 'prepared statement after TTL expiry rejects',
    async run({ driver, dialect, call, expectError }) {
      const ph1 = dialect.placeholder(1);
      const prep = await call('iii-database::prepareStatement', {
        db: driver,
        sql: `SELECT ${ph1} AS v`,
        ttl_seconds: 1,
      });
      const handleId = prep.handle?.id;
      expect(typeof handleId === 'string' && handleId.length > 0, 'handle id present');
      // TTL is 1s; the registry evictor sweeps periodically. Wait long enough
      // that any reasonable evictor cadence will have run.
      await new Promise((r) => setTimeout(r, 1500));
      await expectError(
        () => call('iii-database::runStatement', { handle_id: handleId, params: [42] }),
        'STATEMENT_NOT_FOUND',
      );
    },
  },
  {
    name: 'execute() with SELECT returns 0 affected_rows (does not throw)',
    async run({ driver, call }) {
      // Contract: execute() is for write-shape SQL but should accept SELECT
      // gracefully. affected_rows is undefined for SELECT in most drivers; the
      // worker normalizes that to 0. Asserting "no throw" is the main goal —
      // the value of affected_rows is a softer assertion.
      const r = await call('iii-database::execute', { db: driver, sql: 'SELECT 1 AS v' });
      expect(typeof r === 'object' && r !== null, 'response is object');
      expectEqual(typeof r.affected_rows, 'number', 'affected_rows is a number');
    },
  },
];
