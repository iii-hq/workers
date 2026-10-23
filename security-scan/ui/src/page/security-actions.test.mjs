import assert from 'node:assert/strict'
import test from 'node:test'
import { ACTION_CHANGED_TRIGGER } from './live-triggers.js'
import { actionUpdateFromEvent, createSecurityActionsStore, securityActionKey } from './security-actions.js'

const KEY = securityActionKey('run-1', 2, 'issue')

const request = {
  action_id: 'action-1',
  run_id: 'run-1',
  finding_index: 2,
  action: 'issue',
  status: 'queued',
  deduplicated: false,
}

function action(status, updatedAt = status === 'completed' ? 3 : 2) {
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
    updated_at: updatedAt,
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

/** A host whose binding and connection state the test drives. */
function liveHost() {
  let handler = null
  let connection = null
  const bindings = []
  return {
    bindings,
    host: {
      iii: {
        browserId: 'browser-1',
        on(id, next) {
          handler = next
          bindings.push({ on: id })
          return () => {
            handler = null
          }
        },
        registerTrigger(input) {
          bindings.push(input)
          return () => {
            bindings.push({ unregistered: input.type })
          }
        },
        addConnectionStateListener(next) {
          connection = next
          next('connected')
          return () => {
            connection = null
          }
        },
      },
    },
    emit(payload) {
      handler?.(payload)
    },
    connection(state) {
      connection?.(state)
    },
  }
}

/** The flat `security-scan::action-changed` payload, with the field the engine adds. */
function changed(status, updatedAt = 3, actionId = 'action-1') {
  return {
    action_id: actionId,
    run_id: 'run-1',
    repository: 'iii-hq/iii',
    status,
    updated_at: updatedAt,
    _caller_worker_id: 'worker-1',
  }
}

function deferred() {
  let resolve
  const promise = new Promise((done) => {
    resolve = done
  })
  return { promise, resolve }
}

async function settle() {
  for (let index = 0; index < 10; index += 1) await Promise.resolve()
}

test('binds the worker-owned action trigger type to a tab-scoped handler', () => {
  const harness = liveHost()
  const store = createSecurityActionsStore({
    host: harness.host,
    bindingId: 'bind',
    requestAction: async () => request,
    readAction: async () => null,
    errorText: String,
  })
  store.start()
  assert.deepEqual(harness.bindings, [
    { on: 'iii::security-scan-ui::actions::bind' },
    {
      type: ACTION_CHANGED_TRIGGER,
      function_id: 'iii::security-scan-ui::actions::bind::browser-1',
      config: {},
    },
  ])
  store.dispose()
  assert.deepEqual(harness.bindings.at(-1), { unregistered: ACTION_CHANGED_TRIGGER })
})

test('extracts action updates from action-changed payloads only', () => {
  assert.deepEqual(actionUpdateFromEvent(changed('completed')), {
    actionId: 'action-1',
    status: 'completed',
    updatedAt: 3,
  })
  // The old stream envelope and malformed payloads are not action changes.
  assert.equal(
    actionUpdateFromEvent({
      event: { type: 'event', event: { type: 'security-scan:action-updated', data: changed('completed') } },
    }),
    null,
  )
  assert.equal(actionUpdateFromEvent({ action_id: 'a', status: 'queued' }), null)
  assert.equal(actionUpdateFromEvent({ action_id: '', status: 'queued', updated_at: 1 }), null)
  assert.equal(actionUpdateFromEvent(null), null)
  assert.equal(actionUpdateFromEvent([changed('queued')]), null)
})

test('initial read after the request, then a live update re-reads the record', async () => {
  const harness = liveHost()
  const reads = [action('queued'), action('completed')]
  let readCount = 0
  const store = createSecurityActionsStore({
    host: harness.host,
    bindingId: 'one',
    requestAction: async () => request,
    readAction: async () => {
      readCount += 1
      return reads.shift() ?? null
    },
    errorText: String,
  })
  store.start()

  await store.request('run-1', 2, 'issue')
  assert.equal(readCount, 1)
  assert.equal(store.getSnapshot()[KEY].action.status, 'queued')

  harness.emit(changed('completed'))
  await settle()
  assert.equal(readCount, 2)
  assert.equal(store.getSnapshot()[KEY].action.status, 'completed')
  assert.equal(store.getSnapshot()[KEY].request, null)
  store.dispose()
})

test('ignores notifications for actions this tab is not tracking', async () => {
  const harness = liveHost()
  let readCount = 0
  const store = createSecurityActionsStore({
    host: harness.host,
    bindingId: 'untracked',
    requestAction: async () => request,
    readAction: async () => {
      readCount += 1
      return action('queued')
    },
    errorText: String,
  })
  store.start()
  harness.emit(changed('completed', 3, 'someone-elses-action'))
  await settle()
  assert.equal(readCount, 0)
  assert.deepEqual(store.getSnapshot(), {})
  store.dispose()
})

test('duplicate notifications coalesce into one in-flight read plus one re-read', async () => {
  const harness = liveHost()
  const pending = []
  const store = createSecurityActionsStore({
    host: harness.host,
    bindingId: 'dup',
    requestAction: async () => request,
    readAction: () => {
      const read = deferred()
      pending.push(read)
      return read.promise
    },
    errorText: String,
  })
  store.start()
  const requested = store.request('run-1', 2, 'issue')
  await settle()
  pending.shift().resolve(action('queued'))
  await requested
  assert.equal(pending.length, 0)

  for (let index = 0; index < 5; index += 1) harness.emit(changed('completed'))
  await settle()
  // One read in flight; the four duplicates only marked it dirty.
  assert.equal(pending.length, 1)
  pending.shift().resolve(action('preparing', 2))
  await settle()
  assert.equal(pending.length, 1, 'exactly one coalesced re-read')
  pending.shift().resolve(action('completed'))
  await settle()
  assert.equal(pending.length, 0)
  assert.equal(store.getSnapshot()[KEY].action.status, 'completed')
  store.dispose()
})

test('reads apply in issue order, so a slow earlier read cannot overwrite a later one', async () => {
  const harness = liveHost()
  const pending = []
  const store = createSecurityActionsStore({
    host: harness.host,
    bindingId: 'order',
    requestAction: async () => request,
    readAction: () => {
      const read = deferred()
      pending.push(read)
      return read.promise
    },
    errorText: String,
  })
  store.start()
  const requested = store.request('run-1', 2, 'issue')
  await settle()
  // A notification arrives while the initial read is still in flight.
  harness.emit(changed('completed'))
  await settle()
  assert.equal(pending.length, 1, 'the second read waits for the first')
  pending.shift().resolve(action('queued'))
  await settle()
  assert.equal(store.getSnapshot()[KEY].action.status, 'queued')
  assert.equal(pending.length, 1, 'the coalesced re-read starts after the first read')
  pending.shift().resolve(action('completed'))
  // The request's initial read settles once the coalesced chain has.
  await requested
  assert.equal(store.getSnapshot()[KEY].action.status, 'completed')
  store.dispose()
})

test('an out-of-order older notification does not roll the optimistic status back', async () => {
  const harness = liveHost()
  const store = createSecurityActionsStore({
    host: harness.host,
    bindingId: 'stale',
    requestAction: async () => request,
    // Authoritative reads fail, so the optimistic request status stays visible.
    readAction: async () => {
      throw new Error('temporarily unavailable')
    },
    errorText: String,
  })
  store.start()
  await store.request('run-1', 2, 'issue')

  harness.emit(changed('completed', 5))
  harness.emit(changed('queued', 3))
  await settle()
  assert.equal(store.getSnapshot()[KEY].request.status, 'completed')
  // An equal revision is not older: it still applies.
  harness.emit(changed('failed', 5))
  await settle()
  assert.equal(store.getSnapshot()[KEY].request.status, 'failed')
  store.dispose()
})

test('reconnect re-reads every tracked action once', async () => {
  const harness = liveHost()
  let readCount = 0
  const store = createSecurityActionsStore({
    host: harness.host,
    bindingId: 'reconnect',
    requestAction: async () => request,
    readAction: async () => {
      readCount += 1
      return readCount === 1 ? action('queued') : action('completed')
    },
    errorText: String,
  })
  store.start()
  await store.request('run-1', 2, 'issue')
  assert.equal(readCount, 1)

  harness.connection('reconnecting')
  await settle()
  assert.equal(readCount, 1)
  harness.connection('connected')
  await settle()
  assert.equal(readCount, 2)
  assert.equal(store.getSnapshot()[KEY].action.status, 'completed')
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
  const state = store.getSnapshot()[KEY]
  assert.deepEqual(state.request, request)
  assert.equal(state.action, null)
  assert.equal('repository' in state.request, false)
  assert.equal('target_sha' in state.request, false)
  store.dispose()
})

test('reads once through the request path when live updates are unavailable', async () => {
  let reads = 0
  const store = createSecurityActionsStore({
    host: {
      iii: {
        browserId: 'browser-1',
        on() {
          return () => {}
        },
        registerTrigger() {
          throw new Error('trigger type unavailable')
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
  // authoritative record arrives on the next notification or reconnect.
  assert.equal(reads, 1)
  const state = store.getSnapshot()[KEY]
  assert.deepEqual(state.request, request)
  assert.equal(state.action, null)
  store.dispose()
})
