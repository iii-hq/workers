// Tests for the defensive coercion of untrusted OTel attribute values
// (Record<string, unknown>) into renderable text. Span attributes are
// producer-controlled, so a non-string value (array/object/number) must
// never reach a String method like `.split` and crash the React tree.

import { describe, expect, it } from 'vitest'
import { attributeText } from './attributeText'

describe('attributeText', () => {
  it('passes a string through unchanged', () => {
    expect(attributeText('boom\n  at foo')).toBe('boom\n  at foo')
  })

  it('stringifies numbers and booleans', () => {
    expect(attributeText(42)).toBe('42')
    expect(attributeText(true)).toBe('true')
  })

  it('JSON-encodes arrays so a stacktrace array renders as text', () => {
    expect(attributeText(['frame a', 'frame b'])).toBe('["frame a","frame b"]')
  })

  it('JSON-encodes plain objects', () => {
    expect(attributeText({ message: 'x' })).toBe('{"message":"x"}')
  })

  it('returns undefined for null and undefined', () => {
    expect(attributeText(null)).toBeUndefined()
    expect(attributeText(undefined)).toBeUndefined()
  })

  it('never throws on a value JSON.stringify cannot serialize', () => {
    const circular: Record<string, unknown> = {}
    circular.self = circular
    expect(() => attributeText(circular)).not.toThrow()
    expect(typeof attributeText(circular)).toBe('string')
  })
})
