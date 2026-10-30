import { strict as assert } from 'node:assert'
import { randomBytes } from 'node:crypto'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import { execFileSync } from 'node:child_process'
import { test } from 'node:test'

const script = new URL('./public-report.mjs', import.meta.url)
test('CI selects only outcomes and curated observations, excluding raw credentials and diagnostics', async () => {
  const directory = await mkdtemp(resolve(tmpdir(), 'mesh-public-report-'))
  try {
    const token = randomBytes(32).toString('hex')
    const raw = JSON.stringify({ type: 'workerregistered', reattach_token: token })
    await writeFile(resolve(directory, 'ide-default-frames.json'), raw)
    await writeFile(resolve(directory, 'engine.log'), raw)
    await writeFile(resolve(directory, 'TestFixture-ide-info.json'), raw)
    await writeFile(resolve(directory, 'results.log'), `=== RUN   TestFixture\n${raw}\n--- PASS: TestFixture (1.25s)\n--- FAIL: TestFailure (0.10s)\n--- SKIP: TestSkipped (0.00s)\n`)
    await writeFile(resolve(directory, 'TestFixture-observations.json'), JSON.stringify([
      { connection: 1, request: { type: 'invokefunction', function_id: 'configuration::get', namespace: 'default', data: { reattach_token: token } }, result: 'success' },
      { connection: 1, request: { type: 'registertrigger', function_id: 'ide::ui-content', trigger_type: 'console:assets', config: { reattach_token: token } }, result: { code: 'REGISTRATION_DENIED', message: token } },
      { connection: 1, request: { type: 'workerregistered', reattach_token: token }, result: 'success' },
    ]))
    execFileSync(process.execPath, [script.pathname, directory])
    const workflow = await readFile(new URL('../../../.github/workflows/mesh-e2e.yml', import.meta.url), 'utf8')
    const selection = workflow.match(/name: mesh-e2e-report\s+path: (.+)/)[1]
    const selected = resolve(directory, selection.replace(/^mesh\/tests\/e2e\/reports\//, ''))
    const publicText = await readFile(selected, 'utf8')
    assert(!publicText.includes(token), 'selected artifact contains a fixture credential')
    assert(!publicText.includes('reattach_token'), 'selected artifact contains raw credential metadata')
    const report = JSON.parse(publicText)
    assert.deepEqual(report.counts, { passed: 1, failed: 1, skipped: 1, observations: 2 })
    assert.deepEqual(report.observations, [
      { scenario: 'TestFixture', connection: 1, type: 'invokefunction', function_id: 'configuration::get', namespace: 'default', outcome: 'success' },
      { scenario: 'TestFixture', connection: 1, type: 'registertrigger', function_id: 'ide::ui-content', trigger_type: 'console:assets', outcome: 'REGISTRATION_DENIED' },
    ])
    assert.equal(await readFile(resolve(directory, 'ide-default-frames.json'), 'utf8'), raw, 'private diagnostics preserved')
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
})
