import { describe, expect, it } from 'vitest'
import { isDirty, jsonEqual } from './dirty'

describe('jsonEqual', () => {
  it('treats primitive equals as equal', () => {
    expect(jsonEqual(1, 1)).toBe(true)
    expect(jsonEqual('a', 'a')).toBe(true)
    expect(jsonEqual(true, true)).toBe(true)
    expect(jsonEqual(null, null)).toBe(true)
  })

  it('treats different primitives as not equal', () => {
    expect(jsonEqual(1, 2)).toBe(false)
    expect(jsonEqual('a', 'b')).toBe(false)
    expect(jsonEqual(0, '0')).toBe(false)
    expect(jsonEqual(false, 0)).toBe(false)
  })

  it('treats null, undefined, and missing keys interchangeably', () => {
    expect(jsonEqual(null, undefined)).toBe(true)
    expect(jsonEqual({ a: null }, {})).toBe(true)
    expect(jsonEqual({}, { a: null })).toBe(true)
    expect(jsonEqual({ a: undefined as unknown as null }, {})).toBe(true)
  })

  it('ignores object key order', () => {
    expect(jsonEqual({ a: 1, b: 2 }, { b: 2, a: 1 })).toBe(true)
  })

  it('recurses through nested objects', () => {
    expect(jsonEqual({ a: { b: { c: 1 } } }, { a: { b: { c: 1 } } })).toBe(true)
    expect(jsonEqual({ a: { b: { c: 1 } } }, { a: { b: { c: 2 } } })).toBe(
      false,
    )
  })

  it('compares arrays positionally', () => {
    expect(jsonEqual([1, 2, 3], [1, 2, 3])).toBe(true)
    expect(jsonEqual([1, 2, 3], [3, 2, 1])).toBe(false)
    expect(jsonEqual([1, 2], [1, 2, 3])).toBe(false)
  })

  it('does not confuse arrays with objects', () => {
    expect(jsonEqual([], {})).toBe(false)
  })

  it('returns true for dictionary key renames followed by reverts', () => {
    const loaded = { databases: { primary: { url: 'sqlite:./db' } } }
    const draft = { databases: { primary: { url: 'sqlite:./db' } } }
    expect(jsonEqual(loaded, draft)).toBe(true)
  })

  it('returns false when a deeply nested value diverges', () => {
    const loaded = {
      databases: { primary: { pool: { max: 10 }, url: 'sqlite:./db' } },
    }
    const draft = {
      databases: { primary: { pool: { max: 20 }, url: 'sqlite:./db' } },
    }
    expect(jsonEqual(loaded, draft)).toBe(false)
  })

  it('treats reorderings inside dictionaries as equal', () => {
    const a = { primary: { x: 1 }, secondary: { x: 2 } }
    const b = { secondary: { x: 2 }, primary: { x: 1 } }
    expect(jsonEqual(a, b)).toBe(true)
  })
})

describe('isDirty', () => {
  it('is false when loaded and draft match', () => {
    expect(isDirty({ a: 1 }, { a: 1 })).toBe(false)
  })

  it('is true when draft adds a new key', () => {
    expect(isDirty({ a: 1 }, { a: 1, b: 2 })).toBe(true)
  })

  it('is true when draft removes a key (and value was not null)', () => {
    expect(isDirty({ a: 1, b: 2 }, { a: 1 })).toBe(true)
  })

  it('is false when the removed key was null in the loaded value', () => {
    expect(isDirty({ a: 1, b: null }, { a: 1 })).toBe(false)
  })

  it('is true when an array element changes', () => {
    expect(isDirty({ items: [1, 2, 3] }, { items: [1, 9, 3] })).toBe(true)
  })

  it('is false when neither side has a meaningful value (both null-ish)', () => {
    expect(isDirty(null, undefined)).toBe(false)
  })
})
