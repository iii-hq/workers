import { readFile, readdir, writeFile } from 'node:fs/promises'
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
const observations = []
for (const file of (await readdir(directory)).sort()) {
  if (!/^Test[A-Za-z0-9_-]+-observations\.json$/.test(file)) continue
  const entries = JSON.parse(await readFile(resolve(directory, file), 'utf8'))
  for (const { connection, request, result } of entries) {
    if (!['invokefunction', 'observe'].includes(request?.type)) continue
    const outcome = typeof result === 'string'
      ? ['success', 'no acknowledgement observed', 'configuration event delivery observed'].includes(result) ? result : 'unknown'
      : ['FORBIDDEN', 'invocation_failed'].includes(result?.code) ? result.code : 'error'
    observations.push({
      scenario: file.slice(0, -'-observations.json'.length),
      connection,
      type: request.type,
      function_id: request.source?.function_id ?? request.function_id,
      namespace: request.source?.namespace ?? request.namespace,
      target_function_id: request.target?.function_id,
      target_namespace: request.target?.namespace,
      outcome,
    })
  }
}
const counts = { passed: 0, failed: 0, skipped: 0, observations: observations.length }
for (const { outcome } of tests) counts[{ PASS: 'passed', FAIL: 'failed', SKIP: 'skipped' }[outcome]]++
await writeFile(resolve(directory, 'public-report.json'), JSON.stringify({ counts, tests, observations }, null, 2) + '\n', { mode: 0o600 })
