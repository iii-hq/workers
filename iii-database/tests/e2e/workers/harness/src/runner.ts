import { writeFileSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import type { ISdk } from 'iii-sdk';
import { DRIVER_KEYS, dialects, type DriverKey } from './dialect.ts';
import {
  SCHEMA_RESET,
  FUNCTION_CASES,
  type CaseContext,
  type TestCase,
} from './cases.ts';
import { BOUNDARY_CASES } from './cases-boundary.ts';
import { PROTOCOL_CASES } from './cases-protocol.ts';
import { TRANSACTION_EDGE_CASES } from './cases-transaction.ts';
import { INTERACTIVE_TX_CASES } from './cases-interactive-tx.ts';
import { CONCURRENCY_CASES } from './cases-concurrency.ts';
import { ROW_CHANGE_CASES } from './cases-row-change.ts';

interface CaseResult {
  driver: DriverKey;
  case: string;
  status: 'PASS' | 'FAIL';
  error?: string;
  duration_ms: number;
}

export interface RunnerOptions {
  iii: ISdk;
  reportPath: string;
  filterDriver?: DriverKey;
}

export class Runner {
  constructor(private opts: RunnerOptions) {}

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

  private async runCase(driver: DriverKey, c: TestCase): Promise<CaseResult> {
    const start = Date.now();
    const ctx: CaseContext = {
      driver,
      dialect: dialects[driver],
      call: (id, payload) => this.callOnce(id, payload),
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

      // Function suite.
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
        ...INTERACTIVE_TX_CASES,
        ...CONCURRENCY_CASES,
        ...ROW_CHANGE_CASES,
      ]) {
        if (!matchesDriver(driver, c)) continue;
        record(await this.runCase(driver, c));
      }
    }

    const counted = results.filter((r) => r.case !== 'schema-reset');
    const pass = counted.filter((r) => r.status === 'PASS').length;

    mkdirSync(resolve(this.opts.reportPath, '..'), { recursive: true });
    writeFileSync(
      this.opts.reportPath,
      JSON.stringify({ pass, total: counted.length, results }, null, 2),
    );

    return { pass, total: counted.length, results };
  }
}
