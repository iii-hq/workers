import { describe, expect, it } from 'vitest'
import { errorAt, pointer } from './pointers'

describe('pointer', () => {
  it('encodes RFC 6901 tokens', () => {
    expect(pointer('providers', 'anthropic', 'api_key')).toBe('/providers/anthropic/api_key')
    expect(pointer('routing_heuristics', 0, 'pattern')).toBe('/routing_heuristics/0/pattern')
    expect(pointer('a/b', 'c~d')).toBe('/a~1b/c~0d')
  })
})

describe('errorAt', () => {
  it('returns the message for a matching pointer', () => {
    const errors = new Map([['/providers/openai/api_key', 'required']])
    expect(errorAt(errors, 'providers', 'openai', 'api_key')).toBe('required')
    expect(errorAt(errors, 'providers', 'anthropic', 'api_key')).toBe(null)
    expect(errorAt(undefined, 'providers', 'openai', 'api_key')).toBe(null)
  })
})
