import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { test } from 'node:test'
import { TOURS, findStep, getTour, listTours } from '../src/tours.mjs'

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

/**
 * A step the agent finishes, rather than the operator, reports itself done by
 * writing one key into the onboarding state scope. Two halves have to agree
 * or the step never closes: the sentence the prompt ends with, and the
 * `state` trigger the page binds. This is the check that fails when one of
 * them moves.
 */
test('every agent-finished step names the state key its trigger waits on', () => {
  for (const tour of TOURS) {
    for (const step of tour.steps) {
      if (step.condition?.type !== 'state') continue
      const key = `step_${step.id}_completed`
      assert.equal(step.condition.config.scope, 'onboarding', `${step.id}: wrong state scope`)
      assert.equal(step.condition.config.key, key, `${step.id}: trigger waits on the wrong key`)
      assert.ok(step.ask?.text.includes(key), `${step.id}: the prompt never asks for ${key}`)
      assert.ok(
        step.ask.text.includes('onboarding'),
        `${step.id}: the prompt never names the state scope`,
      )
    }
  }
})

/** The four steps the agent does the work for all wait on the agent. */
test('every step that asks the agent to build waits for it to report done', () => {
  const agentSteps = ['composability', 'discoverability', 'extensibility', 'reactivity']
  for (const id of agentSteps) {
    const step = getTour('console-basics').steps.find((entry) => entry.id === id)
    assert.ok(step, `${id} is no longer a step`)
    assert.equal(step.condition?.type, 'state', `${id}: does not wait on a state write`)
  }
})

/** The tour names the five CODER properties; `Discover` is not one of them. */
test('the discoverability step is titled Discoverability', () => {
  const step = getTour('console-basics').steps.find((entry) => entry.id === 'discoverability')
  assert.equal(step.title, 'Discoverability')
})

test('findStep numbers steps from 1 in tour order', () => {
  for (const tour of TOURS) {
    tour.steps.forEach((step, index) => {
      const found = findStep(tour.id, step.id)
      assert.equal(found?.step, step)
      assert.equal(found?.tour, tour)
      assert.equal(found?.number, index + 1, `${tour.id}/${step.id} numbered wrong`)
    })
  }
  assert.equal(findStep('no-such-tour', 'message'), undefined)
  assert.equal(findStep(TOURS[0].id, 'no-such-step'), undefined)
})

/**
 * The tour hands over to the Tech Lead at the build, and never hands back:
 * every step from there on goes to the same chat, and every step before it
 * stays in the one the tour started in.
 */
test('the build half of console-basics runs under one agent profile', () => {
  const steps = getTour('console-basics').steps
  const handover = steps.findIndex((step) => step.id === 'extensibility')
  assert.ok(handover > 0, 'extensibility is no longer a step')

  for (const step of steps.slice(0, handover)) {
    assert.equal(step.agent, undefined, `${step.id}: sends before the handover`)
  }
  for (const step of steps.slice(handover)) {
    // A step with nothing to ask has nowhere to send it.
    if (!step.ask) continue
    assert.deepEqual(step.agent, { id: 'tech-lead', name: 'Tech Lead' }, `${step.id}: not sent to the Tech Lead`)
  }
})
