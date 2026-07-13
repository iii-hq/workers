import assert from 'node:assert/strict'
import { activate } from './extension.js'

const contributions = []
const disposed = []
const extension = activate({
  apiVersion: 1,
  browserId: 'test-browser',
  extension: { id: 'approval-gate', workerVersion: 'test' },
  registerSlot(contribution) {
    contributions.push(contribution)
    return () => disposed.push(contribution.id)
  },
  async trigger() {
    throw new Error('not called during activation')
  },
  on() {
    throw new Error('not called during activation')
  },
  registerTrigger() {
    throw new Error('not called during activation')
  },
})

assert.deepEqual(
  contributions.map((contribution) => contribution.slot),
  [
    'chat.composer.controls',
    'chat.banner',
    'function-call.pending-actions',
    'settings.sections',
    'chat.workspace-access',
  ],
)
assert.ok(contributions.every((contribution) => typeof contribution.mount === 'function'))

extension.dispose()
assert.deepEqual(disposed.sort(), contributions.map((contribution) => contribution.id).sort())
