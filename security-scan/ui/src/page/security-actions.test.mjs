import assert from 'node:assert/strict'
import test from 'node:test'
import {
  actionUpdateFromFrame,
  createSecurityActionsStore,
  securityActionKey,
} from './security-actions.js'

const request = {
  action_id: 'action-1',
  run_id: 'run-1',
  finding_index: 2,
  action: 'issue',
  status: 'queued',
  deduplicated: false,
}

function action(status) {
  return {
    schema_version: 'security-scan.action.v1',
    action_id: 'action-1',
    run_id: 'run-1',
    finding_index: 2,
    action: 'issue',
    repository: 'iii-hq/iii',
    target_sha: '0123456789abcdef0123456789abcdef01234567',
    status,
    attempt: 1,
    created_at: 1,
    updated_at: status === 'completed' ? 3 : 2,
    ...(status === 'completed'
      ? {
          completed_at: 3,
          result: {
            url: 'https://github.com/iii-hq/iii/issues/1',
            kind: 'issue',
          },
        }
      : {}),
  }
}

function liveHost() {
  let handler = null
  return {
    host: {
      iii: {
        browserId: 'browser-1',
        on(_id, next) {
          handler = next
          return () => {
            handler = null
          }
        },
        registerTrigger() {
          return () => {}
        },
        addConnectionStateListener(next) {
          next('connected')
          return () => {}
        },
      },
    },
    emit(frame) {
      handler?.(frame)
    },
  }
}

function actionFrame(status, updatedAt = 3) {
  return {
    event: {
      type: 'event',
      event: {
        type: 'security-scan:action-updated',
        data: {
          action_id: 'action-1',
          run_id: 'run-1',
          status,
          updated_at: updatedAt,
        },
      },
    },
  }
}

test('extracts action updates from stream event frames', () => {
  assert.deepEqual(actionUpdateFromFrame(actionFrame('completed')), {
    actionId: 'action-1',
    status: 'completed',
    updatedAt: 3,
  })
  assert.equal(actionUpdateFromFrame({ event: { type: 'other' } }), null)
})

test('refreshes one shared action store from live update events', async () => {
  const harness = liveHost()
  const reads = [action('queued'), action('completed')]
  const store = createSecurityActionsStore({
    host: harness.host,
    bindingId: 'one',
    requestAction: async () => request,
    readAction: async () => reads.shift() ?? null,
    errorText: String,
  })
  store.start()

  await store.request('run-1', 2, 'issue')
  const key = securityActionKey('run-1', 2, 'issue')
  assert.equal(store.getSnapshot()[key].action.status, 'queued')

  harness.emit(actionFrame('completed'))
  await Promise.resolve()
  await Promise.resolve()
  assert.equal(store.getSnapshot()[key].action.status, 'completed')
  assert.equal(store.getSnapshot()[key].request, null)
  store.dispose()
})

test('keeps a pending response separate when authoritative reads fail', async () => {
  const harness = liveHost()
  const store = createSecurityActionsStore({
    host: harness.host,
    bindingId: 'two',
    requestAction: async () => request,
    readAction: async () => {
      throw new Error('temporarily unavailable')
    },
    errorText: String,
  })
  store.start()

  await store.request('run-1', 2, 'issue')
  const state =
    store.getSnapshot()[securityActionKey('run-1', 2, 'issue')]
  assert.deepEqual(state.request, request)
  assert.equal(state.action, null)
  assert.equal('repository' in state.request, false)
  assert.equal('target_sha' in state.request, false)
  store.dispose()
})

test('reads once through the request path when the stream is unavailable', async () => {
  let reads = 0
  const store = createSecurityActionsStore({
    host: {
      iii: {
        browserId: 'browser-1',
        on() {
          return () => {}
        },
        registerTrigger() {
          throw new Error('stream unavailable')
        },
        addConnectionStateListener() {
          throw new Error('connection unavailable')
        },
      },
    },
    bindingId: 'three',
    requestAction: async () => request,
    readAction: async () => {
      reads += 1
      return null
    },
    errorText: String,
  })
  store.start()
  await store.request('run-1', 2, 'issue')

  // No timer rearms the read: the accepted request stays visible and the
  // authoritative record arrives on the next stream frame or reconnect.
  assert.equal(reads, 1)
  const state = store.getSnapshot()[securityActionKey('run-1', 2, 'issue')]
  assert.deepEqual(state.request, request)
  assert.equal(state.action, null)
  store.dispose()
})
