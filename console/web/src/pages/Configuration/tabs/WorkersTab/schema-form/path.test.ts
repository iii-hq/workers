import { describe, expect, it } from 'vitest'
import {
  pathGet,
  pathSet,
  pathToDomId,
  pathToPointer,
  pathUnset,
  pointerToPath,
} from './path'

describe('pathGet', () => {
  it('returns the root when path is empty', () => {
    expect(pathGet({ a: 1 }, [])).toEqual({ a: 1 })
  })

  it('walks nested object keys', () => {
    expect(pathGet({ a: { b: { c: 'x' } } }, ['a', 'b', 'c'])).toBe('x')
  })

  it('walks array indices', () => {
    expect(pathGet({ items: ['a', 'b', 'c'] }, ['items', 1])).toBe('b')
  })

  it('returns undefined for missing keys', () => {
    expect(pathGet({ a: 1 }, ['b'])).toBeUndefined()
    expect(pathGet({ a: { b: 1 } }, ['a', 'x', 'y'])).toBeUndefined()
  })

  it('returns undefined when walking into a primitive', () => {
    expect(pathGet({ a: 'x' }, ['a', 'b'])).toBeUndefined()
  })

  it('returns undefined when indexing into a non-array', () => {
    expect(pathGet({ a: 1 }, ['a', 0])).toBeUndefined()
  })

  it('distinguishes explicit null from missing', () => {
    expect(pathGet({ a: null }, ['a'])).toBeNull()
    expect(pathGet({ a: null }, ['a', 'b'])).toBeUndefined()
  })
})

describe('pathSet', () => {
  it('returns the replacement when path is empty', () => {
    expect(pathSet({ a: 1 }, [], 'new')).toBe('new')
  })

  it('sets a top-level key', () => {
    expect(pathSet({ a: 1 }, ['b'], 2)).toEqual({ a: 1, b: 2 })
  })

  it('sets a deeply nested key', () => {
    expect(pathSet({ a: { b: 1 } }, ['a', 'c'], 2)).toEqual({
      a: { b: 1, c: 2 },
    })
  })

  it('creates intermediate objects on the fly', () => {
    expect(pathSet(null, ['a', 'b', 'c'], 1)).toEqual({
      a: { b: { c: 1 } },
    })
  })

  it('replaces an array element', () => {
    expect(pathSet({ items: ['a', 'b', 'c'] }, ['items', 1], 'X')).toEqual({
      items: ['a', 'X', 'c'],
    })
  })

  it('pads arrays with null when writing past the end', () => {
    expect(pathSet({ items: ['a'] }, ['items', 3], 'd')).toEqual({
      items: ['a', null, null, 'd'],
    })
  })

  it('creates a new array when missing', () => {
    expect(pathSet({}, ['items', 0], 'a')).toEqual({ items: ['a'] })
  })

  it('does not mutate the input', () => {
    const original = { a: { b: 1 } }
    const next = pathSet(original, ['a', 'c'], 2)
    expect(original).toEqual({ a: { b: 1 } })
    expect(next).not.toBe(original)
  })

  it('overwrites a primitive with an object when descending into it', () => {
    expect(pathSet({ a: 'x' }, ['a', 'b'], 1)).toEqual({ a: { b: 1 } })
  })
})

describe('pathUnset', () => {
  it('deletes an object key', () => {
    expect(pathUnset({ a: 1, b: 2 }, ['b'])).toEqual({ a: 1 })
  })

  it('splices an array element', () => {
    expect(pathUnset({ items: ['a', 'b', 'c'] }, ['items', 1])).toEqual({
      items: ['a', 'c'],
    })
  })

  it('returns input unchanged when the path is missing', () => {
    expect(pathUnset({ a: 1 }, ['b', 'c'])).toEqual({ a: 1 })
  })

  it('does not mutate the input', () => {
    const original = { a: 1, b: 2 }
    const next = pathUnset(original, ['b'])
    expect(original).toEqual({ a: 1, b: 2 })
    expect(next).not.toBe(original)
  })

  it('deletes a deeply nested key', () => {
    expect(pathUnset({ a: { b: 1, c: 2 } }, ['a', 'c'])).toEqual({
      a: { b: 1 },
    })
  })
})

describe('pathToPointer / pointerToPath', () => {
  it('encodes an empty path to an empty pointer', () => {
    expect(pathToPointer([])).toBe('')
    expect(pointerToPath('')).toEqual([])
  })

  it('round-trips object key paths', () => {
    expect(pathToPointer(['a', 'b', 'c'])).toBe('/a/b/c')
    expect(pointerToPath('/a/b/c')).toEqual(['a', 'b', 'c'])
  })

  it('encodes array indices and decodes them as numbers', () => {
    expect(pathToPointer(['items', 0])).toBe('/items/0')
    expect(pointerToPath('/items/0')).toEqual(['items', 0])
  })

  it('escapes `~` and `/` per RFC 6901', () => {
    expect(pathToPointer(['weird/key', 'has~tilde'])).toBe(
      '/weird~1key/has~0tilde',
    )
    expect(pointerToPath('/weird~1key/has~0tilde')).toEqual([
      'weird/key',
      'has~tilde',
    ])
  })

  it('throws on a pointer that does not start with `/`', () => {
    expect(() => pointerToPath('foo')).toThrow(/invalid json pointer/)
  })

  it('keeps tokens that look like padded numbers as strings', () => {
    expect(pointerToPath('/01')).toEqual(['01'])
  })
})

describe('pathToDomId', () => {
  it('builds deterministic field anchors from schema paths', () => {
    expect(pathToDomId(['fs', 'host_roots'])).toBe(
      'configuration-field-fs-host_roots',
    )
    expect(pathToDomId(['items', 0, 'weird/key'])).toBe(
      'configuration-field-items-0-weird-key',
    )
  })
})
