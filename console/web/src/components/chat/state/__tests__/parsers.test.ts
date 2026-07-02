import { describe, expect, it } from 'vitest'
import {
  isStateFunction,
  safeParseRequest,
  stateRequestSchema,
  unwrapEnvelope,
} from '../parsers'

function wrap<T>(details: T) {
  return {
    content: [{ type: 'text', text: JSON.stringify(details) }],
    details,
    terminate: false,
  }
}

describe('isStateFunction', () => {
  it('matches every state:: op via prefix', () => {
    for (const id of [
      'state::get',
      'state::set',
      'state::delete',
      'state::update',
      'state::list',
      'state::list_groups',
    ]) {
      expect(isStateFunction(id)).toBe(true)
    }
  })

  it('rejects non-state ids', () => {
    expect(isStateFunction('stateful::x')).toBe(false)
    expect(isStateFunction('engine::register_trigger')).toBe(false)
    expect(isStateFunction('state')).toBe(false)
  })
})

describe('stateRequestSchema', () => {
  it('parses scope + key', () => {
    expect(
      safeParseRequest(stateRequestSchema, { scope: 'ops', key: 'build' }),
    ).toEqual({ scope: 'ops', key: 'build' })
  })

  it('tolerates missing fields (list_groups)', () => {
    expect(safeParseRequest(stateRequestSchema, {})).toEqual({})
    expect(safeParseRequest(stateRequestSchema, undefined)).toEqual({})
  })

  it('ignores extra request fields (value / ops)', () => {
    expect(
      safeParseRequest(stateRequestSchema, {
        scope: 'ops',
        key: 'build',
        value: { status: 'green' },
      }),
    ).toEqual({ scope: 'ops', key: 'build' })
  })
})

describe('unwrapEnvelope re-export', () => {
  it('peels the harness envelope to the stored value', () => {
    const value = { commit: 'abc123', status: 'green' }
    expect(unwrapEnvelope(wrap(value))).toEqual(value)
  })

  it('returns primitives unchanged', () => {
    expect(unwrapEnvelope(null)).toBeNull()
    expect(unwrapEnvelope(42)).toBe(42)
  })
})
