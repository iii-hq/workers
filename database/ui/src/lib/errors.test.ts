import { describe, expect, it } from 'vitest'
import { errText } from './errors'

describe('errText', () => {
  it('unwraps the real driver error out of the transport envelope', () => {
    // Exactly what `database::query` rejects with for a missing table.
    const wire = {
      code: 'invocation_failed',
      message:
        'handler error: {"code":"DRIVER_ERROR","driver":"mysql","inner_code":"1146","message":"Server error: `ERROR 42S02 (1146): Table \'iii.no_such_table\' doesn\'t exist\'"}',
      stacktrace: 'a very long rust backtrace',
    }
    expect(errText(wire)).toBe(
      "DRIVER_ERROR (1146): Server error: `ERROR 42S02 (1146): Table 'iii.no_such_table' doesn't exist'",
    )
  })

  it('never renders [object Object]', () => {
    for (const value of [{}, { a: 1 }, [], null, undefined, 42]) {
      expect(errText(value)).not.toContain('object Object')
    }
  })

  it('passes Errors and strings straight through', () => {
    expect(errText(new Error('boom'))).toBe('boom')
    expect(errText('boom')).toBe('boom')
  })

  it('drops the transport code when there is nothing better', () => {
    expect(errText({ code: 'invocation_failed', message: 'worker not connected' })).toBe('worker not connected')
  })

  it('keeps a handler code that carries meaning', () => {
    expect(errText({ code: 'POOL_TIMEOUT', message: 'pool acquire exceeded 5000ms' })).toBe(
      'POOL_TIMEOUT: pool acquire exceeded 5000ms',
    )
  })

  it('stops rather than spins on a self-referential wrapper', () => {
    const nested = { code: 'A', message: 'handler error: {"code":"B","message":"handler error: {"}' }
    expect(errText(nested)).toContain('B')
  })
})
