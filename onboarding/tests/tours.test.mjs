import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { test } from 'node:test'
import { TOURS, getTour, listTours } from '../src/tours.mjs'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')

test('every tour and step is addressable', () => {
  const ids = TOURS.map((tour) => tour.id)
  assert.equal(new Set(ids).size, ids.length, 'duplicate tour id')
  for (const tour of TOURS) {
    assert.ok(tour.steps.length > 0, `${tour.id} has no steps`)
    const steps = tour.steps.map((step) => step.id)
    assert.equal(new Set(steps).size, steps.length, `${tour.id} has duplicate step ids`)
    assert.equal(getTour(tour.id), tour)
  }
  assert.deepEqual(
    listTours().map((tour) => tour.step_count),
    TOURS.map((tour) => tour.steps.length),
  )
})

/**
 * The anchors are classes the console carries for this tour alone, so nothing
 * in the console renders them useless by accident. This is the check that
 * fails when one is renamed or dropped.
 */
test('every anchor still exists in the console source', () => {
  const console_src = join(root, '..', 'console', 'web', 'src')
  if (!existsSync(console_src)) return // packaged worker: no sibling checkout
  for (const tour of TOURS) {
    for (const step of tour.steps) {
      if (!step.anchor) continue
      assert.match(step.anchor, /^\.onboarding-[a-z-]+$/, `${step.id}: odd anchor`)
      const hits = execFileSync(
        'grep',
        ['-rl', step.anchor.slice(1), console_src],
        { encoding: 'utf8' },
      )
      assert.ok(hits.trim().length > 0, `${step.anchor} is in no console file`)
    }
  }
})

test('the injected page ships the anchors it draws', () => {
  const css = readFileSync(join(root, 'ui', 'styles.css'), 'utf8')
  assert.match(css, /\.onboarding-spotlight/)
})
