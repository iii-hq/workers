import { writeFileSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import type { ISdk } from 'iii-sdk';
import { DRIVER_KEYS, dialects, type DriverKey } from './dialect.ts';
import {
  SCHEMA_RESET,
  FUNCTION_CASES,
  POLLING_CASE,
  type CaseContext,
  type TestCase,
} from './cases.ts';
import { BOUNDARY_CASES } from './cases-boundary.ts';
import { PROTOCOL_CASES } from './cases-protocol.ts';
import { TRANSACTION_EDGE_CASES } from './cases-transaction.ts';
import { CONCURRENCY_CASES } from './cases-concurrency.ts';
import { TRIGGER_CASES } from './cases-trigger.ts';
import { ROW_CHANGE_CASES } from './cases-row-change.ts';

interface CaseResult {
  driver: DriverKey;
  case: string;
  status: 'PASS' | 'FAIL';
  error?: string;
  duration_ms: number;
}

type Pending = {
  n: number;
  resolve: (rows: Array<Record<string, unknown>>) => void;
  reject: (err: Error) => void;
  timer: NodeJS.Timeout;
};

type ReceivedBuffer = {
  rows: Array<Record<string, unknown>>;
  pending: Pending[];
};

export interface RunnerOptions {
  iii: ISdk;
  reportPath: string;
  filterDriver?: DriverKey;
}

export class Runner {
  private buffers: Record<DriverKey, ReceivedBuffer> = {
    sqlite_db: { rows: [], pending: [] },
    pg_db: { rows: [], pending: [] },
    mysql_db: { rows: [], pending: [] },
  };

  /**
   * Active poll-trigger handles keyed by driver. We track them so we can
   * unregister at end-of-run; without that, every harness invocation against
   * a long-running worker leaves a zombie polling task alive in the worker
   * process. On the next run, the zombie keeps polling against freshly-reset
   * tables and races with the new task that registerPollingTrigger spawns,
   * producing "second batch is delta only" failures even though the trigger
   * implementation itself is correct.
   */
  private triggers: Partial<Record<DriverKey, { unregister: () => void }>> = {};

  constructor(private opts: RunnerOptions) {}

  /** Sink for `iii-database::query-poll` dispatches; routes by `payload.db`. */
  onOutboxBatch = async (payload: any): Promise<{ ack: boolean; commit_cursor?: string }> => {
    const db = String(payload?.db ?? '') as DriverKey;
    if (!this.buffers[db]) {
      console.error(`[harness] unexpected db in dispatch: ${db}`);
      return { ack: false };
    }
    const rows = (payload.rows ?? []) as Array<Record<string, unknown>>;
    const buf = this.buffers[db];
    buf.rows.push(...rows);

    // Compute max id seen so the worker advances the cursor.
    let maxId: number | undefined;
    for (const r of buf.rows) {
      const id = Number((r as any).id);
      if (Number.isFinite(id)) maxId = maxId === undefined ? id : Math.max(maxId, id);
    }

    // Resolve any waiters that are now satisfied.
    buf.pending = buf.pending.filter((p) => {
      if (buf.rows.length >= p.n) {
        clearTimeout(p.timer);
        p.resolve(buf.rows.slice(0, p.n));
        return false;
      }
      return true;
    });

    return { ack: true, commit_cursor: maxId !== undefined ? String(maxId) : undefined };
  };

  private async callOnce(functionId: string, payload: unknown): Promise<any> {
    return await this.opts.iii.trigger<unknown, any>({ function_id: functionId, payload });
  }

  private async callWithRetry(functionId: string, payload: unknown, attempts = 10): Promise<any> {
    let lastErr: unknown;
    for (let i = 0; i < attempts; i++) {
      try {
        return await this.callOnce(functionId, payload);
      } catch (e) {
        lastErr = e;
        await new Promise((r) => setTimeout(r, 200));
      }
    }
    throw lastErr;
  }

  private waitForRows(driver: DriverKey, n: number, timeoutMs: number): Promise<Array<Record<string, unknown>>> {
    const buf = this.buffers[driver];
    return new Promise((resolveP, rejectP) => {
      if (buf.rows.length >= n) {
        resolveP(buf.rows.slice(0, n));
        return;
      }
      const timer = setTimeout(() => {
        buf.pending = buf.pending.filter((p) => p.resolve !== resolveP);
        rejectP(new Error(`timeout waiting for ${n} rows on ${driver} (got ${buf.rows.length})`));
      }, timeoutMs);
      buf.pending.push({ n, resolve: resolveP, reject: rejectP, timer });
    });
  }

  private resetReceived(driver: DriverKey): void {
    const buf = this.buffers[driver];
    for (const p of buf.pending) clearTimeout(p.timer);
    buf.pending = [];
    buf.rows = [];
  }

  private async runCase(driver: DriverKey, c: TestCase): Promise<CaseResult> {
    const start = Date.now();
    const ctx: CaseContext = {
      driver,
      dialect: dialects[driver],
      call: (id, payload) => this.callOnce(id, payload),
      waitForRows: (n, t) => this.waitForRows(driver, n, t),
      resetReceived: () => this.resetReceived(driver),
      iii: this.opts.iii,
      expectError: async (fn, expectedCode) => {
        try {
          await fn();
        } catch (e: any) {
          const msg = e?.message ?? String(e);
          if (!msg.includes(expectedCode)) {
            throw new Error(`expected error code "${expectedCode}", got: ${msg}`);
          }
          return;
        }
        throw new Error(`expected throw with code "${expectedCode}", but call resolved`);
      },
      expectSilence: async (timeoutMs) => {
        // Reset, wait the window, then assert the per-driver buffer is still empty.
        // Used by trigger validation tests to prove a broken trigger never dispatches.
        const buf = this.buffers[driver];
        const startLen = buf.rows.length;
        await new Promise((r) => setTimeout(r, timeoutMs));
        const drift = buf.rows.length - startLen;
        if (drift > 0) {
          throw new Error(
            `expected silence for ${timeoutMs}ms but received ${drift} rows; latest=${JSON.stringify(buf.rows.slice(-Math.min(drift, 3)))}`,
          );
        }
      },
    };
    try {
      await c.run(ctx);
      return { driver, case: c.name, status: 'PASS', duration_ms: Date.now() - start };
    } catch (e: any) {
      return {
        driver,
        case: c.name,
        status: 'FAIL',
        error: e?.message ?? String(e),
        duration_ms: Date.now() - start,
      };
    }
  }

  private async waitForDatabaseWorker(driver: DriverKey): Promise<void> {
    // Probe with a no-op query until it succeeds; tolerates worker-startup race.
    await this.callWithRetry('iii-database::query', { db: driver, sql: 'SELECT 1' });
  }

  private async registerPollingTrigger(driver: DriverKey): Promise<void> {
    const ph1 = dialects[driver].placeholder(1);
    const handle = this.opts.iii.registerTrigger({
      type: 'iii-database::query-poll',
      function_id: 'harness::on_outbox_row',
      config: {
        trigger_id: `harness-poll-${driver}`,
        db: driver,
        sql: `SELECT id, body FROM outbox WHERE id > COALESCE(${ph1}, 0) ORDER BY id LIMIT 50`,
        interval_ms: 500,
        cursor_column: 'id',
      },
    });
    this.triggers[driver] = handle;
  }

  /** Unregister all active poll triggers. Idempotent. */
  private async unregisterAllTriggers(): Promise<void> {
    for (const driver of DRIVER_KEYS) {
      const t = this.triggers[driver];
      if (t) {
        try {
          t.unregister();
        } catch (e) {
          console.error(`[harness] unregister ${driver}: ${e}`);
        }
        delete this.triggers[driver];
      }
    }
  }

  async runAll(): Promise<{ pass: number; total: number; results: CaseResult[] }> {
    const drivers: DriverKey[] = this.opts.filterDriver ? [this.opts.filterDriver] : [...DRIVER_KEYS];
    // Wait for the database worker to be reachable on the first driver before kicking off.
    await this.waitForDatabaseWorker(drivers[0]);

    const results: CaseResult[] = [];
    const matchesDriver = (driver: DriverKey, c: TestCase) =>
      !c.applies || c.applies.includes(driver);

    // Stream each case result to stdout as it completes, instead of buffering
    // until runAll returns. Slow tests (TTL expiry, pool exhaustion) take 5+
    // seconds individually — the user wants to see progress, not wait blind.
    //
    // Color the PASS/FAIL tag green/red, but only when stdout is a TTY. When
    // run-tests.sh redirects stdout to a log file, isTTY is false and we
    // emit plain text (otherwise ANSI escapes show up as garbage in the log).
    const useColor = process.stdout.isTTY === true;
    const GREEN = useColor ? '\x1b[32m' : '';
    const RED = useColor ? '\x1b[31m' : '';
    const RESET = useColor ? '\x1b[0m' : '';
    const record = (r: CaseResult): CaseResult => {
      const color = r.status === 'PASS' ? GREEN : RED;
      const err = r.error ? ' — ' + r.error : '';
      console.log(`[harness] ${color}${r.status}${RESET} ${r.driver} :: ${r.case} (${r.duration_ms}ms)${err}`);
      results.push(r);
      return r;
    };

    for (const driver of drivers) {
      // Always run the schema reset; not a counted case but failures abort this driver.
      const reset = record(await this.runCase(driver, SCHEMA_RESET));
      if (reset.status === 'FAIL') continue;

      // Function suite (6 cases).
      for (const c of FUNCTION_CASES) {
        record(await this.runCase(driver, c));
      }

      // Boundary, protocol, transaction-edge, concurrency, row-change cases.
      // Each test is self-contained (creates and drops its own scratch tables
      // / replication slots) so order doesn't matter.
      for (const c of [
        ...BOUNDARY_CASES,
        ...PROTOCOL_CASES,
        ...TRANSACTION_EDGE_CASES,
        ...CONCURRENCY_CASES,
        ...ROW_CHANGE_CASES,
      ]) {
        if (!matchesDriver(driver, c)) continue;
        record(await this.runCase(driver, c));
      }

      // Register the per-driver query-poll trigger before the polling case.
      try {
        await this.registerPollingTrigger(driver);
      } catch (e: any) {
        record({
          driver,
          case: 'register-poll-trigger',
          status: 'FAIL',
          error: e?.message ?? String(e),
          duration_ms: 0,
        });
        continue;
      }

      record(await this.runCase(driver, POLLING_CASE));

      // Trigger validation cases (sqlite_db only). Run after the polling case
      // so the long-lived polling trigger has already been registered and the
      // per-driver buffer state is well-defined. Each trigger case unregisters
      // its own ad-hoc trigger; the long-lived polling trigger keeps running.
      // We pause the long-lived polling trigger's effect by relying on a
      // unique scratch table per case (so the polling trigger's outbox SELECT
      // is unaffected by trigger-test inserts), and reset the buffer at the
      // start of each case via resetReceived().
      for (const c of TRIGGER_CASES) {
        if (!matchesDriver(driver, c)) continue;
        record(await this.runCase(driver, c));
      }
    }

    // Cleanup: unregister all active triggers so re-running the harness
    // against the same worker process doesn't leave zombie pollers running.
    await this.unregisterAllTriggers();

    const counted = results.filter((r) => r.case !== 'schema-reset' && r.case !== 'register-poll-trigger');
    const pass = counted.filter((r) => r.status === 'PASS').length;

    mkdirSync(resolve(this.opts.reportPath, '..'), { recursive: true });
    writeFileSync(
      this.opts.reportPath,
      JSON.stringify({ pass, total: counted.length, results }, null, 2),
    );

    return { pass, total: counted.length, results };
  }
}
