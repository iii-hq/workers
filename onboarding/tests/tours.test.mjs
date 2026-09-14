import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { test } from 'node:test'
import { TOURS, getTour, listTours } from '../src/tours.mjs'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')

/**
 * The console's web source, when a sibling checkout has one. It ships as
 * `ade` and is still aliased `console`, so both names are tried — finding
 * neither is a packaged worker, and the anchor checks below skip.
 */
function consoleWebSrc() {
  for (const name of ['ade', 'console']) {
    const candidate = join(root, '..', name, 'web', 'src')
    if (existsSync(candidate)) return candidate
  }
  return null
}

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
  const consoleSrc = consoleWebSrc()
  for (const tour of TOURS) {
    for (const step of tour.steps) {
      if (!step.anchors) continue
      assert.ok(step.anchors.length > 0, `${step.id}: empty anchors`)
      assert.match(step.anchors[0], /^\.onboarding-[a-z-]+$/, `${step.id}: odd first anchor`)
      if (!consoleSrc) continue // packaged worker: no sibling checkout
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

/**
 * A step's button either sends a prompt, performs a console move, or opens a
 * screen. Whichever it is, the page needs the fields to render it.
 */
test('every ask carries a prompt and a button label', () => {
  for (const tour of TOURS) {
    for (const step of tour.steps) {
      if (!step.ask) continue
      assert.ok(step.ask.text?.trim(), `${step.id}: ask has no text`)
      assert.ok(step.ask.label?.trim(), `${step.id}: ask has no label`)
    }
  }
})

/** The page carries out actions by name, so an unknown one would do nothing. */
/**
 * An `on_closed` anchor cannot use the `onboarding-*` rule above: it points at
 * a console control the console publishes no tour class for, so it rides that
 * control's own aria-label. That label is the fragile part — this is the check
 * that fails when someone renames it.
 */
test('every on_closed step carries a body and a label the console still uses', () => {
  const consoleSrc = consoleWebSrc()
  for (const tour of TOURS) {
    for (const step of tour.steps) {
      if (!step.on_closed) continue
      const { screen, body, anchors } = step.on_closed
      assert.ok(screen, `${step.id}: on_closed has no screen to watch`)
      assert.ok(body, `${step.id}: on_closed has no body to show`)
      assert.ok(anchors?.length, `${step.id}: on_closed has no anchor`)
      if (!consoleSrc) continue // packaged worker: no sibling checkout
      for (const anchor of anchors) {
        const label = /\[aria-label="([^"]+)"\]/.exec(anchor)?.[1]
        if (!label) continue
        const hits = execFileSync('grep', ['-rl', label, consoleSrc], {
          encoding: 'utf8',
          stdio: ['ignore', 'pipe', 'ignore'],
        })
        assert.ok(hits.trim().length > 0, `aria-label "${label}" is in no console file`)
      }
    }
  }
})
