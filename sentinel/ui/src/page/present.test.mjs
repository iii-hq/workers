import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  availableActions,
  bulkActions,
  bulkOutcome,
  transitionSentence,
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
  assert.deepEqual(availableActions('resolved'), ['reopen'])
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
      { count: 0, height: 0, hot: false },
      { count: 0, height: 0, hot: false },
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

test('the first row of a history is the group being born', () => {
  assert.equal(transitionSentence({ to_status: 'new', actor: 'ingest' }), 'first seen')
})

test('a move reads as the reason the worker recorded, not as a pair of states', () => {
  assert.equal(
    transitionSentence({
      from_status: 'resolved',
      to_status: 'regressed',
      reason: 'regression',
      actor: 'ingest',
    }),
    'came back after a fix',
  )
})

test('a move with no reason still says what happened', () => {
  assert.equal(
    transitionSentence({ from_status: 'investigating', to_status: 'new', actor: 'investigation' }),
    'investigating → new',
  )
})

test('a selection can only be asked to do what every group in it can', () => {
  assert.deepEqual(bulkActions(['new', 'diagnosed']), ['resolve', 'ignore'])
  assert.deepEqual(bulkActions(['new', 'ignored']), [], 'an ignored group cannot be resolved')
  assert.deepEqual(bulkActions(['ignored', 'ignored']), ['unignore'])
  assert.deepEqual(bulkActions([]), [])
})

test('investigating is never a batch action', () => {
  assert.ok(!bulkActions(['new', 'new']).includes('investigate'))
  assert.deepEqual(bulkActions(['investigating', 'investigating']), [])
})

test('the outcome names what was refused rather than just counting', () => {
  assert.equal(bulkOutcome('resolve', 3, []), '3 resolved.')
  assert.equal(
    bulkOutcome('resolve', 2, ['a group cannot move from ignored to resolved']),
    '2 resolved, 1 refused: a group cannot move from ignored to resolved',
  )
  assert.equal(bulkOutcome('ignore', 0, ['x', 'y']), '0 ignored, 2 refused: x')
})

import {
  ago,
  hotFrom,
  ignoreNote,
  regressedNote,
  scopeOf,
  spaced,
  splitTitle,
  statusLook,
  transitionLook,
  versionRange,
} from './present.js'

test('a resolved group is only offered the way back', () => {
  assert.deepEqual(availableActions('resolved'), ['reopen'])
})

test('counts set their thousands apart with a space', () => {
  assert.equal(spaced(1284), '1 284')
  assert.equal(spaced(8), '8')
})

test('ago is the console\'s relative time, read as a phrase', () => {
  const now = 1_790_000_000_000
  assert.equal(ago(now - 2_000, now), 'just now')
  assert.equal(ago(now - 12_000, now), '12s ago')
  assert.equal(ago(now - 6 * 60_000, now), '6m ago')
  assert.equal(ago(now - 3 * 3_600_000, now), '3h ago')
  assert.equal(ago(now - 14 * 86_400_000, now), '14d ago')
})

test('a version range collapses when nothing changed', () => {
  assert.equal(versionRange('0.22.0', '0.23.0'), '0.22.0 → 0.23.0')
  assert.equal(versionRange('0.23.0', '0.23.0'), '0.23.0')
  assert.equal(versionRange(undefined, undefined), 'unknown version')
})

test('the exception type leads a title only when it prefixes it', () => {
  assert.deepEqual(splitTitle('CasMismatch: expected <n>', 'CasMismatch'), {
    type: 'CasMismatch',
    rest: 'expected <n>',
  })
  assert.deepEqual(splitTitle('Function not found', undefined), { type: null, rest: 'Function not found' })
})

test('only a regression paints its hours hot, from the hour it came back', () => {
  const now = 10 * 3_600_000 + 1
  assert.equal(hotFrom({ status: 'new' }, 24, now), 24)
  assert.equal(hotFrom({ status: 'regressed', regressed_at_ms: now - 2 * 3_600_000 }, 24, now), 21)
  const bars = sparklineBars([1, 0, 2], 1)
  assert.deepEqual(bars.map((bar) => bar.hot), [false, false, true], 'an empty hour is never hot')
})

test('the list scope round-trips through its statuses', () => {
  assert.equal(scopeOf(['new', 'investigating', 'diagnosed', 'regressed']), 'open')
  assert.equal(scopeOf(['regressed']), 'regressed')
  assert.equal(scopeOf(['resolved']), 'resolved')
})

test('each state has its own look, and only regressions are alarming', () => {
  assert.equal(statusLook('regressed').badge, 'alert')
  assert.equal(statusLook('new').badge, 'default')
  assert.equal(statusLook('investigating').glyph, 'live')
})

test('a regression says what was resolved and what brought it back', () => {
  const now = 1_790_000_000_000
  assert.equal(
    regressedNote(
      {
        service_name: 'state',
        last_version: '0.23.0',
        regressed_at_ms: now - 12_000,
        resolved_at_ms: now - 3 * 86_400_000,
        resolved_version: '0.22.1',
        resolve_until_version_change: true,
      },
      now,
    ),
    'Resolved 3d ago in state 0.22.1 with until version change; the first occurrence on 0.23.0 reopened it 12s ago. Occurrences on 0.22.1 kept counting without reopening.',
  )
  assert.equal(
    regressedNote({ service_name: 'state' }, now),
    'It was resolved, and an occurrence reopened it.',
  )
})

test('an ignore rule becomes the note under the group', () => {
  assert.match(ignoreNote({ service_name: 'x', ignore_rule: { kind: 'forever' } }), /^Ignored forever/)
  assert.match(
    ignoreNote({ service_name: 'x', last_version: '1.0', ignore_rule: { kind: 'version_change' } }),
    /until the x version changes \(baseline 1\.0\)/,
  )
})

test('a history row names the state and who moved it', () => {
  assert.deepEqual(transitionLook({ to_status: 'new', actor: 'ingest' }), {
    what: 'First seen',
    note: '',
    dot: 'ghost',
  })
  const resolved = transitionLook({ from_status: 'new', to_status: 'resolved', reason: 'resolved', actor: 'console' })
  assert.equal(resolved.what, 'Resolved')
  assert.equal(resolved.note, 'marked resolved · by a person')
})
