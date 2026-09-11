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
 * The first anchor of every step is a class the console carries for this tour
 * alone. This is the check that fails when one is renamed or dropped.
 */
test('every step anchors on a console class that still exists', () => {
  const consoleSrc = join(root, '..', 'console', 'web', 'src')
  for (const tour of TOURS) {
    for (const step of tour.steps) {
      if (!step.anchors) continue
      assert.ok(step.anchors.length > 0, `${step.id}: empty anchors`)
      assert.match(step.anchors[0], /^\.onboarding-[a-z-]+$/, `${step.id}: odd first anchor`)
      if (!existsSync(consoleSrc)) continue // packaged worker: no sibling checkout
      const hits = execFileSync('grep', ['-rl', step.anchors[0].slice(1), consoleSrc], {
        encoding: 'utf8',
      })
      assert.ok(hits.trim().length > 0, `${step.anchors[0]} is in no console file`)
    }
  }
})

/** A condition has to be bindable: a trigger type, a config, and a label. */
test('every condition is a complete trigger binding', () => {
  for (const tour of TOURS) {
    for (const step of tour.steps) {
      if (!step.condition) continue
      const { type, config, label } = step.condition
      assert.ok(type?.length, `${step.id}: condition has no trigger type`)
      assert.equal(typeof config, 'object', `${step.id}: condition config is not an object`)
      assert.ok(label?.length, `${step.id}: condition has no label`)
    }
  }
})

test('the injected page ships the box it draws', () => {
  const css = readFileSync(join(root, 'ui', 'styles.css'), 'utf8')
  assert.match(css, /\.onboarding-spotlight/)
})
