import test from 'node:test'
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

for (const [runner, report] of [['smoke.mjs', 'cr-regression-failure.json'], ['sp-selector-after.mjs', 'cr-after-failure.json']]) {
  test(`${runner} preserves an early failure before any screenshot creates its output`, { timeout: 20000 }, async () => {
    const ownedRoot = await mkdtemp(join(tmpdir(), 'sentinel-ui-runner-'))
    const output = join(ownedRoot, 'fresh-output')
    try {
      const result = await new Promise((resolve, reject) => {
        const child = spawn(process.execPath, [fileURLToPath(new URL(`../../fixtures/${runner}`, import.meta.url))], {
          env: { ...process.env, SENTINEL_UI_EVIDENCE_DIR: output, SENTINEL_UI_FORCE_EARLY_FAILURE: '1' },
          stdio: ['ignore', 'pipe', 'pipe'],
        })
        let stdout = '', stderr = ''
        child.stdout.on('data', chunk => { stdout += chunk })
        child.stderr.on('data', chunk => { stderr += chunk })
        child.once('error', reject)
        child.once('close', (code, signal) => resolve({ code, signal, stdout, stderr }))
      })
      assert.equal(result.code, 1)
      assert.equal(result.signal, null)
      assert.match(result.stderr, /Synthetic early fixture failure/)
      assert.doesNotMatch(result.stderr, /ENOENT/)
      const saved = JSON.parse(await readFile(join(output, report), 'utf8'))
      assert.equal(saved.error, 'Error: Synthetic early fixture failure')
      assert.deepEqual(saved.observations, [])
    } finally {
      // This unique directory was created above by this test, never preexisting evidence.
      await rm(ownedRoot, { recursive: true, force: true })
    }
  })
}
