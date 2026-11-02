import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { transformSync } from 'esbuild'
import { TOURS } from '../src/tours.mjs'

// The page's condition binding, loaded from its TypeScript source: it imports
// only types, so stripping them is all it takes to run it here.
const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const { code } = transformSync(readFileSync(join(root, 'ui', 'src', 'conditions.ts'), 'utf8'), {
  loader: 'ts',
  format: 'esm',
})
const { bindCondition, reconcileCondition, firesStep } = await import(
  `data:text/javascript;base64,${Buffer.from(code).toString('base64')}`
)

const tick = () => new Promise((resolve) => setImmediate(resolve))

/**
 * A console host as the page sees it. `handle` decides what `observe` returns
 * besides its disposer: nothing (an older host), or the handle's `ready()` and
 * `onStateChange`.
 */
const fakeHost = ({ handle, observeThrows, state = {} } = {}) => {
  const log = []
  const handlers = new Map()
  const listeners = new Set()
  let settle
  const host = {
    iii: {
      browserId: 'console-tab',
      on(id, handler) {
        handlers.set(id, handler)
        log.push(['on', id])
        return () => {
          handlers.delete(id)
          log.push(['off', id])
        }
      },
      observe(source, target) {
        if (observeThrows) throw new Error(observeThrows)
        log.push(['observe', source, target])
        const dispose = () => log.push(['unobserve', target])
        if (!handle) return dispose
        const ready = new Promise((resolve, reject) => {
          settle = { resolve, reject }
        })
        ready.catch(() => {})
        return Object.assign(dispose, {
          ready: () => ready,
          onStateChange(listener) {
            listeners.add(listener)
            return () => listeners.delete(listener)
          },
        })
      },
      async call(id, payload) {
        log.push(['call', id, payload])
        return state[`${payload.scope}/${payload.key}`] ?? null
      },
    },
  }
  return {
    host,
    log,
    handlers,
    emit: (s, reason) => [...listeners].forEach((listener) => listener(s, reason)),
    settle: () => settle,
  }
}

const agentStep = TOURS.flatMap((tour) => tour.steps).find(
  (step) => step.condition?.function_id === 'state::on-changed',
)
const turnStep = TOURS.flatMap((tour) => tour.steps).find(
  (step) => step.condition?.function_id === 'harness::on-turn-completed',
)

test('bindCondition observes the condition with a per-tab handler and fires through it', () => {
  const fake = fakeHost()
  const fired = []
  const unbind = bindCondition(fake.host, 'learn-iii::x', agentStep.condition, (event) => fired.push(event))
  const target = 'learn-iii::condition::learn-iii::x::console-tab'
  assert.deepEqual(fake.log, [
    ['on', 'learn-iii::condition::learn-iii::x'],
    ['observe', { function_id: 'state::on-changed', filters: agentStep.condition.filters }, target],
  ])

  const event = { event_type: 'state:created', scope: 'learn-iii', key: 'k', new_value: true }
  fake.handlers.get('learn-iii::condition::learn-iii::x')(event)
  assert.equal(fired.length, 1)
  assert.equal(fired[0].function_id, 'state::on-changed')
  assert.deepEqual(fired[0].payload, event)

  unbind()
  // The observer goes before its handler, so nothing is delivered to a gone handler.
  assert.deepEqual(fake.log.slice(-2), [['unobserve', target], ['off', 'learn-iii::condition::learn-iii::x']])
})

test('bindCondition passes args through when the condition has them', () => {
  const fake = fakeHost()
  bindCondition(fake.host, 'k', { function_id: 'cron::on-tick', args: { expression: '* * * * * *' }, label: '' }, () => {})
  assert.deepEqual(fake.log[1][1], { function_id: 'cron::on-tick', args: { expression: '* * * * * *' } })
})

test('a refused observer is reported once, whether by ready() or by its state', async () => {
  const fake = fakeHost({ handle: true })
  const failures = []
  bindCondition(fake.host, 'k', turnStep.condition, () => {}, (failure) => failures.push(failure))
  fake.emit('error', { code: 'FORBIDDEN', message: 'no' })
  fake.settle().reject(new Error('FORBIDDEN: no'))
  await tick()
  assert.equal(failures.length, 1)
  assert.match(failures[0], /harness::on-turn-completed observer error: FORBIDDEN/)
})

test('a rejected ready() alone is reported', async () => {
  const fake = fakeHost({ handle: true })
  const failures = []
  bindCondition(fake.host, 'k', turnStep.condition, () => {}, (failure) => failures.push(failure))
  fake.settle().reject(new Error('NOT_OBSERVABLE'))
  await tick()
  assert.deepEqual(failures, ['harness::on-turn-completed observer refused: NOT_OBSERVABLE'])
})

test('the engine dropping the observer is reported, our own dispose is not', async () => {
  const dropped = fakeHost({ handle: true })
  const failures = []
  bindCondition(dropped.host, 'k', turnStep.condition, () => {}, (failure) => failures.push(failure))
  dropped.emit('suspended')
  dropped.emit('active')
  assert.deepEqual(failures, [], 'a suspension is not a failure')
  dropped.emit('disposed', { code: 'FORBIDDEN' })
  assert.equal(failures.length, 1)

  const ours = fakeHost({ handle: true })
  const none = []
  const unbind = bindCondition(ours.host, 'k', turnStep.condition, () => {}, (failure) => none.push(failure))
  unbind()
  ours.emit('disposed', { code: 'UNOBSERVED' })
  ours.settle().reject(new Error('UNOBSERVED'))
  await tick()
  assert.deepEqual(none, [])
})

test('a host that returns a bare disposer reports nothing and still binds', async () => {
  const fake = fakeHost()
  const failures = []
  const unbind = bindCondition(fake.host, 'k', turnStep.condition, () => {}, (failure) => failures.push(failure))
  await tick()
  unbind()
  assert.deepEqual(failures, [])
})

test('an observe that throws takes its handler down and reports the failure', () => {
  const fake = fakeHost({ observeThrows: 'observe is not a function' })
  const failures = []
  const unbind = bindCondition(fake.host, 'k', turnStep.condition, () => {}, (failure) => failures.push(failure))
  assert.deepEqual(fake.log, [['on', 'learn-iii::condition::k'], ['off', 'learn-iii::condition::k']])
  assert.equal(failures.length, 1)
  assert.match(failures[0], /cannot be observed: observe is not a function/)
  unbind()
})

test('reconcileCondition reads an agent step key back, and nothing else', async () => {
  const scope = agentStep.condition.filters['input.scope'].$eq
  const key = agentStep.condition.filters['input.key'].$eq

  const written = fakeHost({ state: { [`${scope}/${key}`]: true } })
  const fired = await reconcileCondition(written.host, agentStep.condition)
  assert.deepEqual(written.log, [['call', 'state::get', { scope, key }]])
  assert.equal(fired.function_id, 'state::on-changed')
  assert.deepEqual(fired.payload, { scope, key, value: true, read_back: true })

  assert.equal(await reconcileCondition(fakeHost().host, agentStep.condition), null, 'an unwritten key')
  const turn = fakeHost()
  assert.equal(await reconcileCondition(turn.host, turnStep.condition), null, 'a turn leaves no trace')
  assert.deepEqual(turn.log, [])
})

test('an agent step waits on writes of its key, never on a delete', () => {
  const filter = agentStep.condition.filters['input.event_type']
  assert.ok(filter?.$in, 'the step filters on the event type')
  assert.ok(filter.$in.includes('state:created') && filter.$in.includes('state:updated'))
  assert.ok(!filter.$in.includes('state:deleted'))
})

test('a finished turn closes the step only in the tour chat', async () => {
  const fired = (session_id) => ({ function_id: turnStep.condition.function_id, payload: { session_id }, at: 0 })
  const chat = async () => 'tour-chat'
  assert.equal(await firesStep(turnStep.condition, fired('tour-chat'), chat), true)
  assert.equal(await firesStep(turnStep.condition, fired('other-chat'), chat), false)
  assert.equal(await firesStep(turnStep.condition, fired(undefined), async () => null), false, 'no tour chat yet')
  assert.equal(
    await firesStep(turnStep.condition, { ...fired(), payload: null }, chat),
    false,
    'an event with no session',
  )
  assert.equal(await firesStep(agentStep.condition, fired('other-chat'), chat), true, 'other conditions always close')
})
