#!/usr/bin/env node
/**
 * The design-lint baseline only shrinks. ESLint accepts whatever counts
 * `eslint-suppressions.json` holds, so a change that adds a violation and
 * bumps its count would pass `lint:design`; this compares every count with
 * the base revision's file and fails on any growth (a new file or rule in
 * the baseline counts as growth from zero).
 *
 *   node scripts/check-suppression-growth.mjs <base.json> <head.json>
 *
 * No base file (the gate's first baseline) passes.
 */
import { existsSync, readFileSync } from 'node:fs'

const [basePath, headPath] = process.argv.slice(2)
if (!basePath || !headPath) {
  console.error('usage: check-suppression-growth.mjs <base.json> <head.json>')
  process.exit(2)
}
if (!existsSync(basePath)) {
  console.log(
    'No suppression baseline at the base revision: nothing to compare.',
  )
  process.exit(0)
}

const read = (path) => JSON.parse(readFileSync(path, 'utf8'))
const base = read(basePath)
const head = read(headPath)

const grown = []
for (const [file, rules] of Object.entries(head)) {
  for (const [rule, { count }] of Object.entries(rules)) {
    const before = base[file]?.[rule]?.count ?? 0
    if (count > before) grown.push(`${file}  ${rule}: ${before} → ${count}`)
  }
}

if (grown.length > 0) {
  console.error(
    [
      'eslint-suppressions.json grew. Fix the new findings instead of counting them:',
      ...grown.map((line) => `  ${line}`),
      'See ade/web/AGENTS.md.',
    ].join('\n'),
  )
  process.exit(1)
}
console.log('Suppression counts did not grow.')
