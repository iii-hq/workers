import { writeFileSync, mkdirSync } from 'node:fs'
import { createServer, type Server } from 'node:http'
import { resolve } from 'node:path'
import { once } from 'node:events'
import type { ISdk } from 'iii-sdk'
import { CASES, ORIGIN_PAGE_HTML, type CaseContext, type TestCase } from './cases.ts'

interface CaseResult {
  case: string
  status: 'PASS' | 'FAIL'
  error?: string
  duration_ms: number
}

export interface RunnerOptions {
  iii: ISdk
  reportPath: string
  /** Harness-side substring filter on case name (run-tests.sh's --filter=). */
  filter?: string
}

export class Runner {
  private origin = ''

  constructor(private opts: RunnerOptions) {}

  private async call(functionId: string, payload: unknown): Promise<any> {
    return await this.opts.iii.trigger<unknown, any>({ function_id: functionId, payload })
  }

  private async callWithRetry(functionId: string, payload: unknown, attempts = 10): Promise<any> {
    let lastErr: unknown
    for (let i = 0; i < attempts; i++) {
      try {
        return await this.call(functionId, payload)
      } catch (e) {
        lastErr = e
        await new Promise((r) => setTimeout(r, 200))
      }
    }
    throw lastErr
  }

  private async runCase(c: TestCase): Promise<CaseResult> {
    const start = Date.now()
    const ctx: CaseContext = { call: (id, payload) => this.call(id, payload), origin: this.origin }
    try {
      await c.run(ctx)
      return { case: c.name, status: 'PASS', duration_ms: Date.now() - start }
    } catch (e: any) {
      return { case: c.name, status: 'FAIL', error: e?.message ?? String(e), duration_ms: Date.now() - start }
    }
  }

  async runAll(): Promise<{ pass: number; total: number; results: CaseResult[] }> {
    // Probe with a no-op call until it succeeds; tolerates the worker-startup
    // race (run-tests.sh already retries `iii trigger` before launching the
    // harness at all, but the harness is also runnable standalone).
    await this.callWithRetry('browser::css', { html: '<p>x</p>', query: 'p' })

    const server = await this.startOrigin()
    const cases = this.opts.filter ? CASES.filter((c) => c.name.includes(this.opts.filter!)) : CASES

    // Stream each case result to stdout as it completes, colored green/red
    // only when stdout is a TTY — run-tests.sh redirects stdout to a log
    // file, and bash's grep for the HARNESS_DONE sentinel must see plain text.
    const useColor = process.stdout.isTTY === true
    const GREEN = useColor ? '\x1b[32m' : ''
    const RED = useColor ? '\x1b[31m' : ''
    const RESET = useColor ? '\x1b[0m' : ''

    const results: CaseResult[] = []
    try {
      for (const c of cases) {
        const r = await this.runCase(c)
        const color = r.status === 'PASS' ? GREEN : RED
        const err = r.error ? ' — ' + r.error : ''
        console.log(`[harness] ${color}${r.status}${RESET} ${r.case} (${r.duration_ms}ms)${err}`)
        results.push(r)
      }
    } finally {
      server.close()
    }

    const pass = results.filter((r) => r.status === 'PASS').length

    mkdirSync(resolve(this.opts.reportPath, '..'), { recursive: true })
    writeFileSync(this.opts.reportPath, JSON.stringify({ pass, total: results.length, results }, null, 2))

    return { pass, total: results.length, results }
  }

  private async startOrigin(): Promise<Server> {
    const server = createServer((req, res) => {
      const path = new URL(req.url ?? '/', 'http://127.0.0.1').pathname
      const body =
        path === '/plain'
          ? Buffer.from([0x63, 0x61, 0x66, 0xe9])
          : Buffer.from(
              path === '/leaf'
                ? '<html><body><p>leaf</p></body></html>'
                : path === '/cookie'
                  ? `<html><body><p>${req.headers.cookie ?? 'none'}</p></body></html>`
                  : ORIGIN_PAGE_HTML,
            )
      res.sendDate = false
      res.statusCode = path === '/plain' ? 206 : 200
      res.setHeader('Content-Type', path === '/plain' ? 'text/plain; charset=iso-8859-1' : 'text/html; charset=utf-8')
      res.setHeader('Date', 'Wed, 12 Aug 2026 16:00:00 GMT')
      res.setHeader('X-Test', ['one', 'two'])
      res.setHeader('Set-Cookie', 'sid=abc; Path=/')
      res.setHeader('Content-Length', body.length)
      res.setHeader('Connection', 'close')
      res.end(body)
    })
    server.listen(0, '127.0.0.1')
    await once(server, 'listening')
    const address = server.address()
    if (!address || typeof address === 'string') throw new Error('local origin did not bind TCP')
    this.origin = `http://127.0.0.1:${address.port}`
    return server
  }
}
