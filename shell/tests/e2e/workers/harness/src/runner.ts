import { writeFileSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import type { ISdk } from 'iii-sdk';
import { FUNCTION_CASES, type CaseContext, type TestCase } from './cases.ts';
import { SAFETY_CASES } from './cases-safety.ts';
import { JOB_CASES } from './cases-jobs.ts';
import { EDGE_CASES } from './cases-edge.ts';
import { FS_HOST_CASES } from './cases-fs-host.ts';
import { FS_HOST_JAIL_CASES } from './cases-fs-host-jail.ts';
import { FS_WRITE_INLINE_CASES } from './cases-fs-write-inline.ts';
import { EXEC_STDIN_CASES } from './cases-exec-stdin.ts';
import { JOBS_BG_TIMEOUT_CASES } from './cases-jobs-bg-timeout.ts';
import { FS_SANDBOX_CASES } from './cases-fs-sandbox.ts';
import { FS_PROTOCOL_BREAK_CASES } from './cases-fs-protocol-break.ts';
import { STREAMING_BREAK_CASES } from './cases-streaming-break.ts';
import { EXEC_BREAK_CASES } from './cases-exec-break.ts';
import { EXEC_SANDBOX_CASES } from './cases-exec-sandbox.ts';
import { JOB_BREAK_CASES } from './cases-jobs-break.ts';
import { FS_ENCODING_CASES } from './cases-fs-encoding.ts';
import { FS_ERROR_CASES } from './cases-fs-errors.ts';
import { CONCURRENCY_CASES } from './cases-concurrency.ts';
import { SANDBOX_BREAK_CASES } from './cases-sandbox-break.ts';
import { VULN_REPRO_CASES } from './cases-vuln-repro.ts';
import { VULN_REPRO_JAILED_CASES } from './cases-vuln-repro-jailed.ts';

interface CaseResult {
  case: string;
  status: 'PASS' | 'FAIL';
  error?: string;
  duration_ms: number;
}

export interface RunnerOptions {
  iii: ISdk;
  reportPath: string;
}

export class Runner {
  constructor(private opts: RunnerOptions) {}

  private async call(functionId: string, payload: unknown): Promise<any> {
    return await this.opts.iii.trigger<unknown, any>({ function_id: functionId, payload });
  }

  private sleep(ms: number): Promise<void> {
    return new Promise((r) => setTimeout(r, ms));
  }

  private async expectError(
    fn: () => Promise<unknown>,
    pattern: string | RegExp,
  ): Promise<void> {
    const matches = (s: string): boolean =>
      typeof pattern === 'string' ? s.includes(pattern) : pattern.test(s);
    const display = typeof pattern === 'string' ? `"${pattern}"` : pattern.toString();
    try {
      await fn();
    } catch (e: any) {
      const msg = e?.message ?? String(e);
      // The SDK rejects with the raw wire error body { code, message, stacktrace }.
      // v0.4.0 surfaces S-codes as the structured `code` so agents branch on
      // error.code rather than parsing the message — so match the pattern
      // against the code AND the human message.
      const code = e && typeof e === 'object' && 'code' in e ? String(e.code) : '';
      if (!matches(msg) && !matches(code)) {
        throw new Error(`expected error matching ${display}, got: ${code ? code + ': ' : ''}${msg}`);
      }
      return;
    }
    throw new Error(`expected throw matching ${display}, but call resolved`);
  }

  private async runCase(c: TestCase): Promise<CaseResult> {
    const start = Date.now();
    const ctx: CaseContext = {
      call: (id, payload) => this.call(id, payload),
      sleep: (ms) => this.sleep(ms),
      expectError: (fn, substring) => this.expectError(fn, substring),
      iii: this.opts.iii,
    };
    try {
      await c.run(ctx);
      return { case: c.name, status: 'PASS', duration_ms: Date.now() - start };
    } catch (e: any) {
      return {
        case: c.name,
        status: 'FAIL',
        error: e?.message ?? String(e),
        duration_ms: Date.now() - start,
      };
    }
  }

  private async waitForWorker(): Promise<void> {
    const deadline = Date.now() + 30_000;
    let lastErr: unknown;
    while (Date.now() < deadline) {
      try {
        await this.call('shell::exec', { command: 'echo', args: ['ready'] });
        return;
      } catch (e) {
        lastErr = e;
        await this.sleep(200);
      }
    }
    throw new Error(`shell worker not reachable within 30s: ${lastErr}`);
  }

  async runAll(): Promise<{ pass: number; total: number; results: CaseResult[] }> {
    await this.waitForWorker();

    const useColor = process.stdout.isTTY === true;
    const GREEN = useColor ? '\x1b[32m' : '';
    const RED = useColor ? '\x1b[31m' : '';
    const RESET = useColor ? '\x1b[0m' : '';

    const results: CaseResult[] = [];
    const record = (r: CaseResult): CaseResult => {
      const color = r.status === 'PASS' ? GREEN : RED;
      const err = r.error ? ' — ' + r.error : '';
      console.log(`[harness] ${color}${r.status}${RESET} :: ${r.case} (${r.duration_ms}ms)${err}`);
      results.push(r);
      return r;
    };

    // HARNESS_SUITE selects which case set runs. `default` (no var
    // set) is the full unjailed suite + the unjailed vuln repros.
    // `jailed` runs ONLY the symlink-jail-escape repro against an
    // engine started with config-jailed.yaml — the rest of the suite
    // assumes no host_roots and would mis-fail there.
    const suite = process.env.HARNESS_SUITE ?? 'default';
    const allCases: TestCase[] =
      suite === 'jailed'
        ? [...VULN_REPRO_JAILED_CASES]
        : [
            ...FUNCTION_CASES,
            ...SAFETY_CASES,
            ...JOB_CASES,
            ...EDGE_CASES,
            ...FS_HOST_CASES,
            ...FS_HOST_JAIL_CASES,
            ...FS_WRITE_INLINE_CASES,
            ...EXEC_STDIN_CASES,
            ...JOBS_BG_TIMEOUT_CASES,
            ...FS_SANDBOX_CASES,
            ...FS_PROTOCOL_BREAK_CASES,
            ...STREAMING_BREAK_CASES,
            ...EXEC_BREAK_CASES,
            ...EXEC_SANDBOX_CASES,
            ...JOB_BREAK_CASES,
            ...FS_ENCODING_CASES,
            ...FS_ERROR_CASES,
            ...CONCURRENCY_CASES,
            ...SANDBOX_BREAK_CASES,
            ...VULN_REPRO_CASES,
          ];
    for (const c of allCases) {
      record(await this.runCase(c));
    }

    const pass = results.filter((r) => r.status === 'PASS').length;

    mkdirSync(resolve(this.opts.reportPath, '..'), { recursive: true });
    writeFileSync(
      this.opts.reportPath,
      JSON.stringify({ pass, total: results.length, results }, null, 2),
    );

    return { pass, total: results.length, results };
  }
}
