/**
 * Smoke test for the injected UI's pure wire surface: the harness envelope
 * unwrap. The chat renderer's claim set and fallthrough behavior live with
 * the renderer in function-trigger-message/index.test.tsx.
 */

import { describe, expect, it } from 'vitest'

import { unwrapEnvelope } from './types'

describe('unwrapEnvelope', () => {
  it('unwraps the harness content/details envelope', () => {
    const details = { id: 'abc12345' }
    expect(unwrapEnvelope({ content: [], details })).toBe(details)
  })

  it('passes every other shape through', () => {
    const plain = { id: 'abc12345' }
    expect(unwrapEnvelope(plain)).toBe(plain)
    expect(unwrapEnvelope(null)).toBeNull()
    expect(unwrapEnvelope([1])).toEqual([1])
    expect(unwrapEnvelope('x')).toBe('x')
  })
})
