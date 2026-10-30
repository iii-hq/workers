import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const read = (...path) => readFileSync(join(root, ...path), 'utf8')

/** The `## Events` section of the README, up to the next `##` heading. */
const eventsSection = () => {
  const readme = read('README.md')
  const start = readme.indexOf('\n## Events\n')
  assert.notEqual(start, -1, 'the README has an Events section')
  const end = readme.indexOf('\n## ', start + 1)
  return readme.slice(start, end === -1 ? undefined : end)
}

/** The doc comment directly above `const publishStep`. */
const publishStepComment = () => {
  const source = read('src', 'index.mjs')
  const at = source.indexOf('const publishStep')
  assert.notEqual(at, -1, 'index.mjs defines publishStep')
  const open = source.lastIndexOf('/**', at)
  const close = source.indexOf('*/', open)
  assert.ok(open !== -1 && close < at, 'publishStep has a doc comment')
  return source.slice(open, close)
}

const stepTopic = () => {
  const match = read('src', 'index.mjs').match(/const STEP_TOPIC = '([^']+)'/)
  assert.ok(match, 'index.mjs defines STEP_TOPIC')
  return match[1]
}

// The queue worker keys each subscriber's durable queue by its observer id.
// A subscriber that observes without a stable id comes back from a restart as
// a new observer with a new queue, and the steps published while it was away
// stay behind in the old one. The docs a subscriber copies from therefore name
// the id.
test('the README tells a step subscriber to observe under a stable observer id', () => {
  const events = eventsSection()
  const topic = stepTopic()

  const example = events.match(/```typescript\n([\s\S]*?)```/)
  assert.ok(example, 'the Events section shows how to subscribe')
  const code = example[1]
  assert.match(code, /iii\.observe\(/)
  assert.match(code, /function_id: 'queue::on-message'/)
  assert.ok(
    code.includes(`args: { topic: '${topic}' }`),
    `the example subscribes to ${topic}, the topic publishStep uses`,
  )
  assert.match(code, /\{ id: '[^']+' \}/, 'the example passes a stable observer id')

  const prose = events.replace(/\s+/g, ' ')
  for (const claim of [
    'under a stable observer id',
    'one durable queue per subscriber, keyed by its observer id',
    "restarts under the same worker name and observer id within the engine's grace period takes its queue back, with every step published while it was away",
    'without an `id` gets a new observer id, and with it a new, empty queue',
  ]) {
    assert.ok(prose.includes(claim), `the Events section says: ${claim}`)
  }
})

test('the publishStep comment names the stable observer id a subscriber needs', () => {
  const comment = publishStepComment()
  assert.match(comment, /`queue::on-message`/)
  assert.match(comment, /args `\{ topic: STEP_TOPIC \}`/)
  const prose = comment.replace(/\s*\n\s*\*\s*/g, ' ')
  assert.ok(prose.includes("under a stable observer id (`{ id: '<subscriber>' }`)"))
  assert.ok(
    prose.includes(
      'restarts under the same worker name and id within the engine\'s grace period takes its queue back',
    ),
  )
})
