import assert from 'node:assert/strict'
import test from 'node:test'
import { withRpcTimeout } from './rpc-timeout.js'

test('returns an RPC result before the deadline', async () => {
  assert.equal(
    await withRpcTimeout(Promise.resolve('ok'), 'security-scan::list', 50),
    'ok',
  )
})

test('rejects a stuck RPC after the deadline', async () => {
  await assert.rejects(
    withRpcTimeout(new Promise(() => {}), 'security-scan::read', 5),
    /security-scan::read timed out after 0.005s/,
  )
})
