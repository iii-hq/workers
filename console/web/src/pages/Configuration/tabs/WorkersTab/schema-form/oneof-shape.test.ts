import { describe, expect, it } from 'vitest'
import type { JsonSchema } from '../api'
import {
  discriminator,
  hasRenderableProps,
  isNullSchema,
  isPlainObject,
  mergeNullable,
  nullableUnionInner,
  singleEnumString,
  stripProperty,
} from './oneof-shape'

// Resolved branches of the iii-state `adapter` schema (after $ref resolution),
// the shape schemars emits for a Rust adjacently-tagged enum.
const adapterVariants: JsonSchema[] = [
  {
    title: 'kv',
    type: 'object',
    required: ['name'],
    additionalProperties: false,
    properties: {
      name: { type: 'string', enum: ['kv'] },
      config: { type: 'object', properties: { store_method: {} } },
    },
  },
  {
    title: 'redis',
    type: 'object',
    required: ['name'],
    additionalProperties: false,
    properties: {
      name: { type: 'string', enum: ['redis'] },
      config: { type: 'object', properties: { redis_url: {} } },
    },
  },
  {
    title: 'bridge',
    type: 'object',
    required: ['name'],
    additionalProperties: false,
    properties: {
      name: { type: 'string', enum: ['bridge'] },
      config: { type: 'object', properties: { bridge_url: {} } },
    },
  },
]

describe('discriminator', () => {
  it('detects the discriminator key and per-branch values', () => {
    expect(discriminator(adapterVariants)).toEqual({
      key: 'name',
      values: ['kv', 'redis', 'bridge'],
    })
  })

  it('supports the `const` discriminator form', () => {
    const variants: JsonSchema[] = [
      { type: 'object', properties: { kind: { const: 'a' } } },
      { type: 'object', properties: { kind: { const: 'b' } } },
    ]
    expect(discriminator(variants)).toEqual({ key: 'kind', values: ['a', 'b'] })
  })

  it('returns null when a variant is missing the pinned key', () => {
    const variants: JsonSchema[] = [
      { type: 'object', properties: { name: { enum: ['kv'] } } },
      { type: 'object', properties: { other: { enum: ['redis'] } } },
    ]
    expect(discriminator(variants)).toBeNull()
  })

  it('returns null when discriminator values are not distinct', () => {
    const variants: JsonSchema[] = [
      { type: 'object', properties: { name: { enum: ['dup'] } } },
      { type: 'object', properties: { name: { enum: ['dup'] } } },
    ]
    expect(discriminator(variants)).toBeNull()
  })

  it('returns null when a variant is not an object', () => {
    const variants: JsonSchema[] = [
      { type: 'object', properties: { name: { enum: ['kv'] } } },
      { type: 'string', enum: ['redis'] },
    ]
    expect(discriminator(variants)).toBeNull()
  })

  it('returns null when a non-object variant still carries a properties map', () => {
    const variants: JsonSchema[] = [
      { type: 'string', properties: { kind: { enum: ['a'] } } },
      { type: 'number', properties: { kind: { enum: ['b'] } } },
    ]
    expect(discriminator(variants)).toBeNull()
  })

  it('returns null for a single-variant union', () => {
    expect(discriminator([adapterVariants[0]])).toBeNull()
  })
})

describe('nullableUnionInner', () => {
  it('returns the non-null branch for [enum, null]', () => {
    const inner = { type: 'string', enum: ['in_memory', 'file_based'] }
    expect(nullableUnionInner([inner, { type: 'null' }])).toBe(inner)
  })

  it('returns the non-null branch for [null, object]', () => {
    const inner = { type: 'object', properties: {} }
    expect(nullableUnionInner([{ type: 'null' }, inner])).toBe(inner)
  })

  it('returns null when there is no null branch', () => {
    expect(
      nullableUnionInner([{ type: 'string' }, { type: 'number' }]),
    ).toBeNull()
  })

  it('returns null for unions that are not exactly two variants', () => {
    expect(nullableUnionInner([{ type: 'string' }])).toBeNull()
    expect(
      nullableUnionInner([
        { type: 'string' },
        { type: 'number' },
        { type: 'null' },
      ]),
    ).toBeNull()
  })

  it('returns null when both branches are null', () => {
    expect(nullableUnionInner([{ type: 'null' }, { type: 'null' }])).toBeNull()
  })
})

describe('mergeNullable', () => {
  it('keeps an enum branch intact (EnumField handles the unset)', () => {
    const inner = { type: 'string', enum: ['in_memory', 'file_based'] }
    const merged = mergeNullable({ description: 'outer' }, inner)
    expect(merged.enum).toEqual(['in_memory', 'file_based'])
    expect(merged.type).toEqual(['string', 'null'])
    expect(merged.description).toBe('outer')
  })

  it('adds null to a non-enum branch type so it routes to NullableField', () => {
    const merged = mergeNullable(
      {},
      { type: 'object', properties: { x: {} } },
    )
    expect(merged.type).toEqual(['object', 'null'])
    expect(merged.properties).toEqual({ x: {} })
  })

  it('carries the outer default and prefers the outer description', () => {
    const merged = mergeNullable(
      { description: 'outer', default: 'd' },
      { type: 'string', description: 'inner' },
    )
    expect(merged.description).toBe('outer')
    expect(merged.default).toBe('d')
  })
})

describe('stripProperty', () => {
  it('removes the discriminator from properties, required, and title', () => {
    const stripped = stripProperty(adapterVariants[0], 'name')
    expect(Object.keys(stripped.properties as object)).toEqual(['config'])
    expect(stripped.required).toEqual([])
    expect(stripped.title).toBeUndefined()
    // Original schema is untouched.
    expect(
      Object.keys(adapterVariants[0].properties as object),
    ).toContain('name')
  })
})

describe('singleEnumString', () => {
  it('reads a single-value enum', () => {
    expect(singleEnumString({ enum: ['kv'] })).toBe('kv')
  })
  it('reads a const', () => {
    expect(singleEnumString({ const: 'redis' })).toBe('redis')
  })
  it('returns null for multi-value enums and missing schemas', () => {
    expect(singleEnumString({ enum: ['a', 'b'] })).toBeNull()
    expect(singleEnumString(undefined)).toBeNull()
    expect(singleEnumString({ type: 'string' })).toBeNull()
  })
})

describe('misc helpers', () => {
  it('isNullSchema only matches a pure null schema', () => {
    expect(isNullSchema({ type: 'null' })).toBe(true)
    expect(isNullSchema({ type: ['string', 'null'] })).toBe(false)
    expect(isNullSchema({ type: 'null', enum: [null] })).toBe(false)
  })
  it('isPlainObject distinguishes objects from arrays/null', () => {
    expect(isPlainObject({ a: 1 })).toBe(true)
    expect(isPlainObject([])).toBe(false)
    expect(isPlainObject(null)).toBe(false)
  })
  it('hasRenderableProps requires a non-empty properties map', () => {
    expect(hasRenderableProps({ properties: { x: {} } })).toBe(true)
    expect(hasRenderableProps({ properties: {} })).toBe(false)
    expect(hasRenderableProps({ type: 'object' })).toBe(false)
  })
})
