import { describe, expect, it } from 'vitest'
import { isObjectSchema } from './guard'

describe('isObjectSchema', () => {
  it('accepts a plain schema object', () => {
    expect(isObjectSchema({ type: 'object', title: 'x' })).toBe(true)
    expect(isObjectSchema({})).toBe(true)
  })

  it('rejects null/undefined — a worker that exposed no config schema', () => {
    // This is the crash case: SchemaForm read `schema.title` on null.
    expect(isObjectSchema(null)).toBe(false)
    expect(isObjectSchema(undefined)).toBe(false)
  })

  it('rejects non-object schemas (array, boolean, string)', () => {
    expect(isObjectSchema([])).toBe(false)
    expect(isObjectSchema(true)).toBe(false)
    expect(isObjectSchema('object')).toBe(false)
  })
})
