import assert from 'node:assert/strict'
import test from 'node:test'
import {
  isLiveUpdates,
  ON_ACTION_CHANGED_FN,
  ON_RECONCILIATION_CHANGED_FN,
  ON_RUN_CHANGED_FN,
  RUN_LIVE_SOURCES,
  RUNS_HANDLER_ID,
} from './live-sources.js'

test('the runs page observes only the worker-owned change functions', () => {
  assert.deepEqual([...RUN_LIVE_SOURCES], ['security-scan::on-run-changed', 'security-scan::on-reconciliation-changed'])
  assert.equal(ON_RUN_CHANGED_FN, 'security-scan::on-run-changed')
  assert.equal(ON_RECONCILIATION_CHANGED_FN, 'security-scan::on-reconciliation-changed')
  assert.equal(ON_ACTION_CHANGED_FN, 'security-scan::on-action-changed')
  // Plain ids (no filters), so the observer sees every run; the list can
  // never be frozen into a stale shape.
  for (const source of RUN_LIVE_SOURCES) {
    assert.equal(typeof source, 'string')
    assert.ok(source.startsWith('security-scan::on-'), source)
  }
  assert.ok(Object.isFrozen(RUN_LIVE_SOURCES))
  // `iii::` keeps per-event handler invocations out of the trace feed.
  assert.equal(RUNS_HANDLER_ID, 'iii::security-scan-ui::runs')
})

test('reports live only for an active observer on a connected host', () => {
  assert.equal(isLiveUpdates(true, 'connected'), true)
  assert.equal(isLiveUpdates(true, 'reconnecting'), false)
  assert.equal(isLiveUpdates(false, 'connected'), false)
})
