import { writeFileSync, mkdirSync } from 'node:fs'
import { resolve } from 'node:path'
import type { IIIClient } from 'iii-sdk'
import { CASES, type CaseContext, type TestCase } from './cases.ts'

interface CaseResult {
  case: string
  status: 'PASS' | 'FAIL'
  error?: string
  duration_ms: number
}

export interface RunnerOptions {
  support: IIIClient
  down: IIIClient
  reportPath: string
  filter?: string
}

export class Runner {
  constructor(private opts: RunnerOptions) {}

  private async runCase(c: TestCase): Promise<CaseResult> {
    const start = Date.now()
    const ctx: CaseContext = {
      support: this.opts.support,
      down: this.opts.down,
      expectError: async (fn, code) => {
        try {
          await fn()
        } catch (e: unknown) {
          // The Node SDK throws an InvocationError carrying a typed `.code`;
          // fall back to a message substring match for robustness.
          const ecode = (e as { code?: string })?.code
          const msg = (e as { message?: string })?.message ?? String(e)
          if (ecode === code || msg.includes(code)) return
          throw new Error(`expected error code "${code}", got code="${ecode}" message="${msg}"`)
        }
        throw new Error(`expected throw with code "${code}", but the call resolved`)
      },
    }
    try {
      await c.run(ctx)
      return { case: c.name, status: 'PASS', duration_ms: Date.now() - start }
    } catch (e: unknown) {
      const msg = (e as { message?: string })?.message ?? String(e)
      return { case: c.name, status: 'FAIL', error: msg, duration_ms: Date.now() - start }
    }
  }

  async runAll(): Promise<{ pass: number; total: number; results: CaseResult[] }> {
    const cases = this.opts.filter ? CASES.filter((c) => c.name.includes(this.opts.filter!)) : CASES

    // Color the PASS/FAIL tag only when stdout is a TTY — run-tests.sh
    // redirects to a log file and greps for the plain-text sentinel.
    const useColor = process.stdout.isTTY === true
    const GREEN = useColor ? '\x1b[32m' : ''
    const RED = useColor ? '\x1b[31m' : ''
    const RESET = useColor ? '\x1b[0m' : ''

    const results: CaseResult[] = []
    for (const c of cases) {
      // Cases share `down`/`support` and some depend on earlier ones (the
      // prefix self-invoke uses the function the prefix-apply case registered),
      // so they run sequentially in declaration order.
      const r = await this.runCase(c)
      const color = r.status === 'PASS' ? GREEN : RED
      const err = r.error ? ' — ' + r.error : ''
      console.log(`[harness] ${color}${r.status}${RESET} ${r.case} (${r.duration_ms}ms)${err}`)
      results.push(r)
    }

    const pass = results.filter((r) => r.status === 'PASS').length
    mkdirSync(resolve(this.opts.reportPath, '..'), { recursive: true })
    writeFileSync(this.opts.reportPath, JSON.stringify({ pass, total: results.length, results }, null, 2))
    return { pass, total: results.length, results }
  }
}
