import { registerWorker } from 'iii-sdk'
import { resolve } from 'node:path'
import { Runner } from './runner.ts'

// No default. 49134 is the port a developer's own stack usually holds, so a
// silent fallback would point the suite at their engine and report nonsense
// about it — see the harness README. run-tests.sh always sets this.
const URL = process.env.III_URL
if (!URL) throw new Error('III_URL is required — run this through ./run-tests.sh, which picks a free port')
const REPORT_PATH = resolve(process.env.HARNESS_REPORT_PATH ?? './reports/report.json')
const FILTER = process.env.HARNESS_FILTER || undefined

const iii = registerWorker(URL)
const runner = new Runner({ iii, reportPath: REPORT_PATH, filter: FILTER })

console.log(`[harness] code-runner e2e harness registered url=${URL} filter=${FILTER ?? 'all'}`)

;(async () => {
  const useColor = process.stdout.isTTY === true
  const GREEN = useColor ? '\x1b[32m' : ''
  const RED = useColor ? '\x1b[31m' : ''
  const RESET = useColor ? '\x1b[0m' : ''
  let exitCode = 1
  try {
    // The worker may still be connecting when we register. Probe until it
    // answers rather than failing the whole suite on a startup race.
    const deadline = Date.now() + 30000
    for (;;) {
      try {
        await iii.trigger({ function_id: 'code-runner::run', payload: { lang: 'node', code: '1' }, timeoutMs: 5000 })
        break
      } catch (e) {
        if (Date.now() > deadline) throw new Error(`code-runner never became callable: ${e}`)
        await new Promise((r) => setTimeout(r, 250))
      }
    }
    const { pass, total } = await runner.runAll()
    const status = pass === total && total > 0 ? 'PASS' : 'FAIL'
    const color = status === 'PASS' ? GREEN : RED
    console.log(`HARNESS_DONE: ${color}${status}${RESET} ${pass}/${total}`)
    exitCode = status === 'PASS' ? 0 : 1
  } catch (e: any) {
    console.error('[harness] fatal:', e?.stack ?? e)
    console.log(`HARNESS_DONE: ${RED}FAIL${RESET} 0/0`)
    exitCode = 1
  }
  // Grace period lets the OS flush ws bytes; shutdown() does not await the
  // close handshake.
  await new Promise((r) => setTimeout(r, 200))
  await iii.shutdown()
  process.exit(exitCode)
})()
