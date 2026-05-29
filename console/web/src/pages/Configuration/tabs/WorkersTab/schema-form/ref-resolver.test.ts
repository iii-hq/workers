import { describe, expect, it } from 'vitest'
import type { JsonSchema } from '../api'
import {
  isInlineLeaf,
  isNullable,
  resolveSchema,
  schemaDefault,
  schemaTypes,
  withoutNull,
} from './ref-resolver'

const ROOT: JsonSchema = {
  definitions: {
    Pool: {
      type: 'object',
      properties: {
        max: { type: 'integer', default: 10 },
      },
    },
    Mode: {
      oneOf: [
        { enum: ['disable'], type: 'string' },
        { enum: ['require'], type: 'string' },
      ],
    },
    Cycle: {
      properties: {
        nested: { $ref: '#/definitions/Cycle' },
      },
      type: 'object',
    },
    LoopA: { $ref: '#/definitions/LoopB' },
    LoopB: { $ref: '#/definitions/LoopA' },
  },
}

describe('resolveSchema', () => {
  it('returns the schema unchanged when no $ref is present', () => {
    const s: JsonSchema = { type: 'string' }
    expect(resolveSchema(s, { root: ROOT })).toBe(s)
  })

  it('resolves a direct $ref', () => {
    const s: JsonSchema = { $ref: '#/definitions/Pool' }
    const resolved = resolveSchema(s, { root: ROOT })
    expect(resolved.type).toBe('object')
    expect((resolved.properties as Record<string, JsonSchema>).max.type).toBe(
      'integer',
    )
  })

  it('resolves the schemars `allOf: [$ref]` wrapper and keeps siblings', () => {
    const s: JsonSchema = {
      allOf: [{ $ref: '#/definitions/Pool' }],
      default: { max: 20 },
      description: 'wrapped',
    }
    const resolved = resolveSchema(s, { root: ROOT })
    expect(resolved.type).toBe('object')
    expect(resolved.default).toEqual({ max: 20 })
    expect(resolved.description).toBe('wrapped')
    expect(resolved.allOf).toBeUndefined()
  })

  it('lets the sibling description override the referenced description', () => {
    const root: JsonSchema = {
      definitions: {
        Foo: { type: 'string', description: 'inner' },
      },
    }
    const wrapper: JsonSchema = {
      allOf: [{ $ref: '#/definitions/Foo' }],
      description: 'outer',
    }
    const resolved = resolveSchema(wrapper, { root })
    expect(resolved.description).toBe('outer')
  })

  it('returns {} for unsupported external $refs', () => {
    const s: JsonSchema = { $ref: 'https://example.com/schema.json' }
    expect(resolveSchema(s, { root: ROOT })).toEqual({})
  })

  it('returns {} for broken local $refs', () => {
    const s: JsonSchema = { $ref: '#/definitions/Missing' }
    expect(resolveSchema(s, { root: ROOT })).toEqual({})
  })

  it('does not eagerly recurse into nested properties (lazy by design)', () => {
    // Properties hold a $ref themselves; resolveSchema only resolves the
    // outer level on each call. The dispatcher re-invokes resolveSchema
    // on each child as it renders, which keeps cycles bounded to the
    // depth the operator actually drills into.
    const s: JsonSchema = { $ref: '#/definitions/Cycle' }
    const resolved = resolveSchema(s, { root: ROOT })
    expect(resolved.type).toBe('object')
    const nested = (resolved.properties as Record<string, JsonSchema>).nested
    expect(nested.$ref).toBe('#/definitions/Cycle')
  })

  it('guards $ref→$ref cycles within a single chain', () => {
    const s: JsonSchema = { $ref: '#/definitions/LoopA' }
    const resolved = resolveSchema(s, { root: ROOT })
    expect(typeof resolved.description).toBe('string')
    expect(resolved.description).toMatch(/cycle/)
  })

  it('keeps unrelated allOf intersections unchanged', () => {
    const s: JsonSchema = {
      allOf: [{ type: 'string' }, { minLength: 3 }],
    }
    expect(resolveSchema(s, { root: ROOT })).toBe(s)
  })

  it('decodes JSON pointer escape sequences in $ref segments', () => {
    const root: JsonSchema = {
      definitions: {
        'has/slash': { type: 'string' },
      },
    }
    const s: JsonSchema = { $ref: '#/definitions/has~1slash' }
    expect(resolveSchema(s, { root }).type).toBe('string')
  })
})

describe('schemaTypes', () => {
  it('normalizes a single type to a one-element array', () => {
    expect(schemaTypes({ type: 'string' })).toEqual(['string'])
  })

  it('returns array types verbatim', () => {
    expect(schemaTypes({ type: ['string', 'null'] })).toEqual([
      'string',
      'null',
    ])
  })

  it('returns an empty array when type is missing', () => {
    expect(schemaTypes({})).toEqual([])
  })
})

describe('isNullable / withoutNull', () => {
  it('detects nullable unions', () => {
    expect(isNullable({ type: ['string', 'null'] })).toBe(true)
    expect(isNullable({ type: 'string' })).toBe(false)
    expect(isNullable({ type: ['null'] })).toBe(false)
  })

  it('strips `null` from a nullable union', () => {
    expect(withoutNull({ type: ['string', 'null'] }).type).toBe('string')
    expect(withoutNull({ type: ['string', 'integer', 'null'] }).type).toEqual([
      'string',
      'integer',
    ])
  })

  it('returns the schema unchanged when there is no null in the type union', () => {
    const s: JsonSchema = { type: 'string' }
    expect(withoutNull(s)).toBe(s)
  })
})

describe('isInlineLeaf', () => {
  it('treats primitive types as inline leaves', () => {
    expect(isInlineLeaf({ type: 'string' }, ROOT)).toBe(true)
    expect(isInlineLeaf({ type: 'integer' }, ROOT)).toBe(true)
    expect(isInlineLeaf({ type: 'number' }, ROOT)).toBe(true)
    expect(isInlineLeaf({ type: 'boolean' }, ROOT)).toBe(true)
  })

  it('treats any enum as an inline leaf regardless of type', () => {
    expect(isInlineLeaf({ enum: ['a', 'b'] }, ROOT)).toBe(true)
    expect(isInlineLeaf({ type: 'integer', enum: [1, 2] }, ROOT)).toBe(true)
  })

  it('does not inline objects, arrays, oneOf, anyOf, or nullable unions', () => {
    expect(isInlineLeaf({ type: 'object' }, ROOT)).toBe(false)
    expect(isInlineLeaf({ type: 'array' }, ROOT)).toBe(false)
    expect(isInlineLeaf({ oneOf: [{ type: 'string' }] }, ROOT)).toBe(false)
    expect(isInlineLeaf({ anyOf: [{ type: 'string' }] }, ROOT)).toBe(false)
    expect(isInlineLeaf({ type: ['string', 'null'] }, ROOT)).toBe(false)
  })

  it('classifies through a $ref', () => {
    expect(isInlineLeaf({ $ref: '#/definitions/Pool' }, ROOT)).toBe(false)
    expect(isInlineLeaf({ $ref: '#/definitions/Mode' }, ROOT)).toBe(false)
  })
})

describe('schemaDefault', () => {
  it('honors the schema-provided default', () => {
    expect(schemaDefault({ type: 'integer', default: 42 })).toBe(42)
  })

  it('falls back to type-based zero values', () => {
    expect(schemaDefault({ type: 'string' })).toBe('')
    expect(schemaDefault({ type: 'integer' })).toBe(0)
    expect(schemaDefault({ type: 'number' })).toBe(0)
    expect(schemaDefault({ type: 'boolean' })).toBe(false)
    expect(schemaDefault({ type: 'array' })).toEqual([])
    expect(schemaDefault({ type: 'object' })).toEqual({})
    expect(schemaDefault({})).toBeNull()
  })
})
