import { writeFileSync, mkdirSync } from 'node:fs'
import { createServer, type Server } from 'node:http'
import { resolve } from 'node:path'
import { once } from 'node:events'
import type { ISdk } from 'iii-sdk'
import { CASES, ORIGIN_PAGE_HTML, type CaseContext, type TestCase } from './cases.ts'
import { HARDENING_CASES } from './cases-hardening.ts'

const ALL_CASES: TestCase[] = [...CASES, ...HARDENING_CASES]

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

    // Claim the pre-generate hook trigger type, as a real agent-harness stack
    // would. Without an owner the browser worker's guidance binding stays
    // parked in the engine's pending map, which registered-triggers::list
    // does not expose — with one, the binding goes live and the
    // inject_guidance hot-apply case can observe it bind and unbind. The
    // handler never fires: nothing in this suite emits pre-generate events.
    this.opts.iii.registerTriggerType(
      { id: 'harness::hook::pre-generate', description: 'E2E stand-in for the agent harness pre-generate hook.' },
      { registerTrigger: async () => {}, unregisterTrigger: async () => {} },
    )

    const server = await this.startOrigin()
    const cases = this.opts.filter ? ALL_CASES.filter((c) => c.name.includes(this.opts.filter!)) : ALL_CASES

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
      const address = server.address()
      const port = address && typeof address !== 'string' ? address.port : 0

      // Hardening-case endpoints own their complete response (status, headers,
      // body); everything below them keeps the legacy uniform header block so
      // the frozen-envelope assertions of the original cases stay byte-stable.
      const html = (content: string, headers: Record<string, string | string[]> = {}, contentType = 'text/html; charset=utf-8') => {
        const buf = Buffer.from(content)
        res.statusCode = 200
        res.setHeader('Content-Type', contentType)
        for (const [k, v] of Object.entries(headers)) res.setHeader(k, v)
        res.setHeader('Content-Length', buf.length)
        res.setHeader('Connection', 'close')
        res.end(buf)
      }
      const redirect = (location: string, headers: Record<string, string | string[]> = {}) => {
        res.statusCode = 302
        res.setHeader('Location', location)
        for (const [k, v] of Object.entries(headers)) res.setHeader(k, v)
        res.setHeader('Content-Length', 0)
        res.setHeader('Connection', 'close')
        res.end()
      }
      res.sendDate = false
      switch (path) {
        case '/loop':
          return redirect(`http://127.0.0.1:${port}/loop`)
        case '/hop-a':
          return redirect(`http://127.0.0.1:${port}/hop-b`, { 'Set-Cookie': 'hop=1; Path=/' })
        case '/hop-x':
          // Same server, different hostname: exercises the hostname-only
          // scoping of redirect cookie replay (127.0.0.1 vs localhost).
          return redirect(`http://localhost:${port}/hop-b`, { 'Set-Cookie': 'hopx=1; Path=/' })
        case '/hop-b':
          return html(`<html><body><p>${req.headers.cookie ?? 'none'}</p></body></html>`)
        case '/multi-cookie':
          return html('<html><body><p>mc</p></body></html>', {
            'Set-Cookie': ['a=1; Path=/', 'b=2; Path=/'],
            'X-Test': ['one', 'two'],
          })
        case '/echo-headers':
          // rawHeaders preserves duplicates and original casing — the egress
          // gate case counts Connection headers, which req.headers would fold.
          return html(
            `<html><body><pre>${JSON.stringify({ url: req.url, rawHeaders: req.rawHeaders })}</pre></body></html>`,
          )
        case '/cf-managed':
          // Fake Cloudflare managed challenge: the cType marker routes
          // solve_cloudflare into its managed branch, and the "Verifying"
          // text keeps it spinning until the solve deadline expires.
          return html(
            "<html><head><title>E2E CF</title></head><body><script>/* cType: 'managed' */</script><p>Verifying you are human.</p></body></html>",
          )
        case '/cf-clean':
          // Same marker but no spin text and no challenge iframe: the solve
          // loop must fall through and return the page normally.
          return html(
            "<html><head><title>fine</title></head><body><script>/* cType: 'managed' */</script><p>done</p></body></html>",
          )
        case '/worker-page':
          return html(
            '<html><body><div id="out">pending</div><script>const w=new Worker("/w.js");w.onmessage=(e)=>{document.getElementById("out").textContent=e.data}</script></body></html>',
          )
        case '/w.js':
          return html("postMessage('worker-ran')", {}, 'application/javascript')
        case '/idn':
          return html('<html><body><a href="http://xn--mnchen-3ya.example/">m</a></body></html>')
        default:
          break
      }
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
