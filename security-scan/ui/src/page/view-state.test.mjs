import assert from 'node:assert/strict'
import test from 'node:test'
import {
  automaticFocusTarget,
  beginRetry,
  isRepositoryScopeCurrent,
  isStreamLive,
  nextVisibleFindingCount,
  settleRetry,
  shouldReloadReconciliation,
} from './view-state.js'

test('keeps concurrent retry state isolated by run id', () => {
  let states = beginRetry({}, 'run-a')
  states = beginRetry(states, 'run-b')
  states = settleRetry(states, 'run-a', 'retry failed')

  assert.deepEqual(states['run-a'], { pending: false, error: 'retry failed' })
  assert.deepEqual(states['run-b'], { pending: true, error: null })
})

test('reports live only for a registered binding on a connected host', () => {
  assert.equal(isStreamLive(true, 'connected'), true)
  assert.equal(isStreamLive(true, 'reconnecting'), false)
  assert.equal(isStreamLive(false, 'connected'), false)
})

test('withholds repository-scoped state until the active repository resolves', () => {
  assert.equal(isRepositoryScopeCurrent('iii-hq/iii', null), false)
  assert.equal(isRepositoryScopeCurrent('iii-hq/workers', 'iii-hq/iii'), false)
  assert.equal(
    isRepositoryScopeCurrent('iii-hq/workers', 'iii-hq/workers'),
    true,
  )
  assert.equal(isRepositoryScopeCurrent('', ''), true)
})

test('chooses a stable automatic narrow-pane focus target', () => {
  assert.deepEqual(automaticFocusTarget(true, true, 'run-next'), {
    kind: 'run',
    runId: 'run-next',
  })
  assert.deepEqual(automaticFocusTarget(true, true, null), { kind: 'filter' })
  assert.equal(automaticFocusTarget(false, true, 'run-next'), null)
  assert.equal(automaticFocusTarget(true, false, 'run-next'), null)
})

test('progressively reveals findings without exceeding the report size', () => {
  assert.equal(nextVisibleFindingCount(20, 55, 20), 40)
  assert.equal(nextVisibleFindingCount(40, 55, 20), 55)
})

test('reloads reconciliation only for a selected run after a newer refresh signal', () => {
  assert.equal(shouldReloadReconciliation(4, 5, 'run-a'), true)
  assert.equal(shouldReloadReconciliation(5, 5, 'run-a'), false)
  assert.equal(shouldReloadReconciliation(4, 5, null), false)
})
