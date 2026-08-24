import assert from 'node:assert/strict'
import test from 'node:test'
import { errText } from './errors.js'

test('renders a rejected list RPC as its message, not [object Object]', () => {
  const wire = {
    code: 'invocation_failed',
    message:
      'handler error: dependency failure: state::claim-namespace failed after 20 attempts: remote error (function_not_found): Function state::claim-namespace not found',
  }
  assert.equal(
    errText(wire),
    'dependency failure: state::claim-namespace failed after 20 attempts: remote error (function_not_found): Function state::claim-namespace not found',
  )
  assert.equal(String(wire), '[object Object]')
  assert.doesNotMatch(errText(wire), /\[object Object\]/)
})

test('unwraps a nested handler envelope and prefixes the handler code', () => {
  assert.equal(
    errText({
      code: 'invocation_failed',
      message:
        'handler error: {"code":"INVALID_REQUEST","message":"limit must be between 1 and 200"}',
    }),
    'INVALID_REQUEST: limit must be between 1 and 200',
  )
})

test('keeps a meaningful top-level code', () => {
  assert.equal(
    errText({
      code: 'function_not_found',
      message: 'Function security-scan::list not found',
    }),
    'function_not_found: Function security-scan::list not found',
  )
})

test('passes Errors and strings through', () => {
  assert.equal(errText(new Error('plain')), 'plain')
  assert.equal(errText('already text'), 'already text')
})

test('never yields [object Object] for an unrecognised object', () => {
  assert.equal(errText({ weird: true }), '{"weird":true}')
  const cyclic = {}
  cyclic.self = cyclic
  assert.equal(errText(cyclic), 'unknown error')
})
