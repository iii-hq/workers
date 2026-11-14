import { readFile, writeFile } from 'node:fs/promises'
import { resolve } from 'node:path'

const directory = resolve(process.argv[2] ?? 'mesh/tests/e2e/reports')
const optionalRead = async (path) => {
  try { return await readFile(path, 'utf8') } catch (error) {
    if (error.code === 'ENOENT') return ''
    throw error
  }
}
const log = await optionalRead(resolve(directory, 'results.log'))
// Only test outcome lines are public; diagnostic output can contain raw frames.
const tests = [...log.matchAll(/^--- (PASS|FAIL|SKIP): (Test[A-Za-z0-9_]+) \(([\d.]+)s\)$/gm)]
  .map(([, outcome, name, seconds]) => ({ name, outcome, seconds: Number(seconds) }))
const counts = { passed: 0, failed: 0, skipped: 0 }
for (const { outcome } of tests) counts[{ PASS: 'passed', FAIL: 'failed', SKIP: 'skipped' }[outcome]]++
await writeFile(resolve(directory, 'public-report.json'), JSON.stringify({ counts, tests }, null, 2) + '\n', { mode: 0o600 })
