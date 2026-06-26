/**
 * E2E harness entry point. In one process it:
 *   1. registers the support functions on the engine (auth/middleware/hooks/targets),
 *   2. waits for the engine to index them,
 *   3. connects a downstream worker THROUGH the proxy port (bearer token),
 *   4. runs the RBAC suite and emits a `HARNESS_DONE: PASS|FAIL n/m` sentinel.
 *
 * run-tests.sh greps stdout for the sentinel and reads the JSON report.
 */

import { registerWorker } from 'iii-sdk'
import { resolve } from 'node:path'
import { registerSupport } from './support.ts'
import { Runner } from './runner.ts'

const ENGINE_URL = process.env.III_URL ?? 'ws://127.0.0.1:49134'
const PROXY_URL = process.env.PROXY_URL ?? 'ws://127.0.0.1:49271'
const REPORT_PATH = resolve(process.env.HARNESS_REPORT_PATH ?? './reports/report.json')
const FILTER = process.env.HARNESS_FILTER

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

async function waitForAuthFn(support: ReturnType<typeof registerWorker>): Promise<boolean> {
  // The proxy invokes support::auth on the downstream's upgrade, so it must be
  // indexed before `down` connects.
  for (let i = 0; i < 40; i++) {
    try {
      const d: Json = await support.trigger({
        function_id: 'engine::functions::info',
        payload: { function_id: 'support::auth' },
      })
      if (d?.function_id === 'support::auth') return true
    } catch {
      /* not indexed yet */
    }
    await sleep(200)
  }
  return false
}

;(async () => {
  const useColor = process.stdout.isTTY === true
  const GREEN = useColor ? '\x1b[32m' : ''
  const RED = useColor ? '\x1b[31m' : ''
  const RESET = useColor ? '\x1b[0m' : ''

  let support: ReturnType<typeof registerWorker> | undefined
  let down: ReturnType<typeof registerWorker> | undefined
  let exitCode = 1

  try {
    support = registerSupport(ENGINE_URL)
    if (!(await waitForAuthFn(support))) {
      throw new Error('support::auth was not indexed by the engine in time')
    }

    down = registerWorker(PROXY_URL, { headers: { authorization: 'Bearer test-token' } })
    // Let the upgrade authenticate + the upstream open before the first call.
    await sleep(800)

    const runner = new Runner({ support, down, reportPath: REPORT_PATH, filter: FILTER })
    const { pass, total } = await runner.runAll()
    const status = total > 0 && pass === total ? 'PASS' : 'FAIL'
    const color = status === 'PASS' ? GREEN : RED
    console.log(`HARNESS_DONE: ${color}${status}${RESET} ${pass}/${total}`)
    exitCode = status === 'PASS' ? 0 : 1
  } catch (e: unknown) {
    console.error('[harness] fatal:', (e as { stack?: string })?.stack ?? e)
    console.log(`HARNESS_DONE: ${RED}FAIL${RESET} 0/0`)
    exitCode = 1
  }

  // Grace period to flush ws bytes, then close both connections.
  await sleep(200)
  try {
    await down?.shutdown()
  } catch {
    /* ignore */
  }
  try {
    await support?.shutdown()
  } catch {
    /* ignore */
  }
  process.exit(exitCode)
})()
