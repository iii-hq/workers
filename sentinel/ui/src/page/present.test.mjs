import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  availableActions,
  codeLocation,
  ignoreSummary,
  sessionAffordance,
  sinceMs,
  sparklineBars,
} from './present.js'

test('a running first pass offers stopping, not relabelling', () => {
  assert.deepEqual(availableActions('investigating'), ['stop'])
})

test('only a human decision is offered on an open group', () => {
  for (const status of ['new', 'diagnosed', 'regressed']) {
    assert.deepEqual(availableActions(status), ['investigate', 'resolve', 'ignore'])
  }
})

test('a closed group offers only the way back', () => {
  assert.deepEqual(availableActions('resolved'), ['investigate', 'reopen'])
  assert.deepEqual(availableActions('ignored'), ['unignore'])
})

test('the window filter counts back from now, and all time has no floor', () => {
  const now = 1_790_000_000_000
  assert.equal(sinceMs('24h', now), now - 86_400_000)
  assert.equal(sinceMs('all', now), null)
})

test('an hour with occurrences never draws as nothing', () => {
  const bars = sparklineBars([0, 1, 50])
  assert.equal(bars[0].height, 0)
  assert.ok(bars[1].height >= 8, `a single occurrence must be visible: ${bars[1].height}`)
  assert.equal(bars[2].height, 100)
})

test('an empty window draws no bars rather than dividing by zero', () => {
  assert.deepEqual(
    sparklineBars([0, 0]),
    [
      { count: 0, height: 0 },
      { count: 0, height: 0 },
    ],
  )
})

test('the session button says open unless the session is already in front of the user', () => {
  assert.equal(sessionAffordance('s1', 's1')?.variant, 'live')
  assert.equal(sessionAffordance('s1', 's2')?.variant, 'open')
  assert.equal(sessionAffordance(null, 's2'), null)
})

test('code evidence resolves against the mapped checkout, and nothing else does', () => {
  assert.deepEqual(codeLocation({ kind: 'code', path: 'src/a.rs', line: 12 }, '/repo/'), {
    path: '/repo/src/a.rs',
    line: 12,
  })
  assert.deepEqual(codeLocation({ kind: 'code', path: './src/a.rs' }, '/repo'), {
    path: '/repo/src/a.rs',
    line: 1,
  })
  assert.equal(codeLocation({ kind: 'trace', span_id: 's' }, '/repo'), null)
  assert.equal(codeLocation({ kind: 'code', path: 'a.rs' }, null), null)
})

test('an ignore rule reads as a sentence', () => {
  assert.equal(ignoreSummary({ kind: 'forever' }), 'ignored')
  assert.equal(ignoreSummary({ kind: 'occurrences', count: 1 }), 'ignored for 1 more occurrence')
  assert.equal(ignoreSummary({ kind: 'occurrences', count: 50 }), 'ignored for 50 more occurrences')
  assert.equal(ignoreSummary({ kind: 'version_change' }), 'ignored until the version changes')
  assert.equal(ignoreSummary(null), '')
})
