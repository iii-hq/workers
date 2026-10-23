import assert from 'node:assert/strict'
import test from 'node:test'
import {
  ACTION_CHANGED_TRIGGER,
  isLiveUpdates,
  RECONCILIATION_CHANGED_TRIGGER,
  RUN_CHANGED_TRIGGER,
  RUN_LIVE_TRIGGERS,
  RUNS_HANDLER_ID,
} from './live-triggers.js'

test('the runs page binds only the worker-owned change trigger types', () => {
  assert.deepEqual([...RUN_LIVE_TRIGGERS], ['security-scan::run-changed', 'security-scan::reconciliation-changed'])
  assert.equal(RUN_CHANGED_TRIGGER, 'security-scan::run-changed')
  assert.equal(RECONCILIATION_CHANGED_TRIGGER, 'security-scan::reconciliation-changed')
  assert.equal(ACTION_CHANGED_TRIGGER, 'security-scan::action-changed')
  // Plain ids (no filter config), so the binding sees every run like the
  // feed it replaces; the list can never be frozen into a stale shape.
  for (const trigger of RUN_LIVE_TRIGGERS) {
    assert.equal(typeof trigger, 'string')
    assert.ok(trigger.startsWith('security-scan::'), trigger)
  }
  assert.ok(Object.isFrozen(RUN_LIVE_TRIGGERS))
  // `iii::` keeps per-event handler invocations out of the trace feed.
  assert.equal(RUNS_HANDLER_ID, 'iii::security-scan-ui::runs')
})

test('reports live only for a registered binding on a connected host', () => {
  assert.equal(isLiveUpdates(true, 'connected'), true)
  assert.equal(isLiveUpdates(true, 'reconnecting'), false)
  assert.equal(isLiveUpdates(false, 'connected'), false)
})
