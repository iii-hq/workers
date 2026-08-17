import { describe, expect, it } from 'vitest'
import { errText } from './errors'

describe('errText', () => {
  it('renders the wire error object a rejected trigger throws', () => {
    /* The exact payload `directory::prompts::create` returns on a stack whose
       iii-directory predates the function — reproduced with
       `iii trigger directory::prompts::create`. `String(err)` gives
       "[object Object]" here, which is the bug this exists to stop. */
    const wire = {
      code: 'function_not_found',
      message: 'Function directory::prompts::create not found',
      stacktrace: null,
    }
    expect(errText(wire)).toBe(
      'function_not_found: Function directory::prompts::create not found',
    )
    expect(errText(wire)).not.toContain('[object Object]')
  })

  it('unwraps a handler error nested inside the transport envelope', () => {
    expect(
      errText({
        code: 'invocation_failed',
        message:
          'handler error: {"code":"D214","message":"prompt \\"test\\" already exists."}',
      }),
    ).toBe('D214: prompt "test" already exists.')
  })

  it('drops transport-only codes that say nothing', () => {
    expect(errText({ code: 'invocation_failed', message: 'boom' })).toBe('boom')
  })

  it('strips the SDK "handler error: " prefix from a plain (non-JSON) handler string', () => {
    /* iii-directory's handler errors are prose, not JSON — `unwrap` finds no
       `{` to peel, so without the fix this renders
       "handler error: D214 invalid_input: …" with the prefix still attached. */
    expect(
      errText({
        code: 'invocation_failed',
        message:
          'handler error: D214 invalid_input: system prompt "x" already exists.',
      }),
    ).toBe('D214 invalid_input: system prompt "x" already exists.')
  })

  it('passes Errors and strings through untouched', () => {
    expect(errText(new Error('plain'))).toBe('plain')
    expect(errText('already text')).toBe('already text')
  })

  it('never yields [object Object] for an unrecognised object', () => {
    expect(errText({ weird: true })).toBe('{"weird":true}')
    const cyclic: Record<string, unknown> = {}
    cyclic.self = cyclic
    expect(errText(cyclic)).toBe('unknown error')
  })
})
