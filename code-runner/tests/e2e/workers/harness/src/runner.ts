import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'
import type { IIIClient, InvocationError } from 'iii-sdk'
import type { CaseContext, CaseGroup, TestCase } from './cases.ts'
import { ALL_GROUPS } from './groups.ts'

interface CaseResult {
  group: string
  case: string
  status: 'PASS' | 'FAIL'
  error?: string
  duration_ms: number
}

export interface RunnerOptions {
  iii: IIIClient
  reportPath: string
  /** Substring filter on `group` or `group :: case`. */
  filter?: string
  callTimeoutMs?: number
}

export class Runner {
  constructor(private opts: RunnerOptions) {}

  private async call(functionId: string, payload: unknown = {}): Promise<any> {
    return this.opts.iii.trigger({
      function_id: functionId,
      payload: payload ?? {},
      timeoutMs: this.opts.callTimeoutMs ?? 20000,
    })
  }

  private async runCase(group: string, c: TestCase): Promise<CaseResult> {
    const start = Date.now()

    const ctx: CaseContext = {
      call: (id, payload) => this.call(id, payload),
      iii: this.opts.iii,
      expectError: async (fn, kind) => {
        try {
          await fn()
        } catch (e: any) {
          const err: InvocationError = e
          // Matched against the MESSAGE, not `code`. The SDK reports its own
          // transport-level code — `invocation_failed` for every handler
          // error — and carries this worker's `code-runner::…` code inside
          // the message. Asserting on `err.code` therefore passes or fails
          // identically for every taxonomy entry, which is no assertion at
          // all. node-engine's harness learned the same thing.
          const msg = err?.message ?? String(e)
          if (!msg.includes(kind)) {
            throw new Error(`expected an error mentioning ${JSON.stringify(kind)}, got: ${msg}`)
          }
          return err
        }
        throw new Error(`expected a throw mentioning ${JSON.stringify(kind)}, but the call resolved`)
      },
    }

    try {
      await c.run(ctx)
      return { group, case: c.name, status: 'PASS', duration_ms: Date.now() - start }
    } catch (e: any) {
      return {
        group,
        case: c.name,
        status: 'FAIL',
        error: e?.message ?? String(e),
        duration_ms: Date.now() - start,
      }
    }
  }

  async runAll(): Promise<{ pass: number; total: number; results: CaseResult[] }> {
    const useColor = process.stdout.isTTY === true
    const GREEN = useColor ? '\x1b[32m' : ''
    const RED = useColor ? '\x1b[31m' : ''
    const DIM = useColor ? '\x1b[2m' : ''
    const RESET = useColor ? '\x1b[0m' : ''

    const groups: CaseGroup[] = ALL_GROUPS
    const results: CaseResult[] = []

    for (const g of groups) {
      const selected = g.cases.filter(
        (c) =>
          !this.opts.filter || `${g.name} :: ${c.name}`.includes(this.opts.filter) || g.name.includes(this.opts.filter),
      )
      if (!selected.length) continue
      console.log(`${DIM}--- ${g.name} (${selected.length})${RESET}`)
      for (const c of selected) {
        const r = await this.runCase(g.name, c)
        results.push(r)
        const color = r.status === 'PASS' ? GREEN : RED
        const err = r.error ? ` ${DIM}${r.error}${RESET}` : ''
        console.log(`[harness] ${color}${r.status}${RESET} ${r.case} (${r.duration_ms}ms)${err}`)
      }
    }

    const pass = results.filter((r) => r.status === 'PASS').length
    const report = {
      generated_at: new Date().toISOString(),
      pass,
      total: results.length,
      results,
    }
    mkdirSync(dirname(this.opts.reportPath), { recursive: true })
    writeFileSync(this.opts.reportPath, `${JSON.stringify(report, null, 2)}\n`)
    return { pass, total: results.length, results }
  }
}
