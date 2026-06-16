import { describe, expect, it } from 'vitest'
import type { JsonSchema, JsonValue } from '../api'
import {
  isSingleStringEnumVariant,
  matchVariantIndex,
  variantDefault,
  variantLabel,
} from './variant-match'

/**
 * Mirrors the schemars output for an adjacently tagged enum
 * (`#[serde(tag = "name", content = "config")]`), as registered by
 * session-manager's `StorageAdapter`. `config` is a `$ref` so the tests
 * also exercise root-relative resolution.
 */
const ROOT: JsonSchema = {
  type: 'object',
  definitions: {
    FsBackendConfig: {
      type: 'object',
      properties: {
        data_dir: { type: 'string', default: '~/.iii/data/session-manager' },
      },
    },
    BridgeBackendConfig: {
      type: 'object',
      properties: {
        url: { type: 'string' },
        timeout_ms: { type: 'integer', default: 5000 },
      },
      required: ['url'],
    },
  },
}

const ADAPTER_VARIANTS: JsonSchema[] = [
  {
    type: 'object',
    properties: {
      name: { type: 'string', enum: ['fs'] },
      config: { $ref: '#/definitions/FsBackendConfig' },
    },
    required: ['config', 'name'],
  },
  {
    type: 'object',
    properties: {
      name: { type: 'string', enum: ['bridge'] },
      config: { $ref: '#/definitions/BridgeBackendConfig' },
    },
    required: ['config', 'name'],
  },
]

describe('matchVariantIndex — discriminated (tagged) variants', () => {
  it('selects the variant whose tag matches the value', () => {
    const fs: JsonValue = { name: 'fs', config: { data_dir: '/tmp' } }
    const bridge: JsonValue = { name: 'bridge', config: { url: 'ws://m' } }
    expect(matchVariantIndex(ADAPTER_VARIANTS, fs)).toBe(0)
    expect(matchVariantIndex(ADAPTER_VARIANTS, bridge)).toBe(1)
  })

  it('does not collapse two object variants onto the first', () => {
    // Both variants are `type: object`; without tag matching a bridge value
    // would wrongly resolve to the fs variant (index 0).
    const bridge: JsonValue = { name: 'bridge', config: {} }
    expect(matchVariantIndex(ADAPTER_VARIANTS, bridge)).toBe(1)
  })

  it('falls back to the first variant when no tag matches', () => {
    expect(matchVariantIndex(ADAPTER_VARIANTS, { name: 'unknown' })).toBe(0)
    expect(matchVariantIndex(ADAPTER_VARIANTS, undefined)).toBe(0)
  })
})

describe('matchVariantIndex — heterogeneous (structural) variants', () => {
  const variants: JsonSchema[] = [
    { type: 'integer' },
    { type: 'string' },
    { type: 'object', properties: { value: { type: 'integer' } } },
  ]

  it('matches by JSON type when there is no tag', () => {
    expect(matchVariantIndex(variants, 30)).toBe(0)
    expect(matchVariantIndex(variants, 'PT30S')).toBe(1)
    expect(matchVariantIndex(variants, { value: 1 })).toBe(2)
  })
})

describe('variantDefault', () => {
  it('builds a discriminated default with nested config defaults', () => {
    expect(variantDefault(ADAPTER_VARIANTS[0], ROOT)).toEqual({
      name: 'fs',
      config: { data_dir: '~/.iii/data/session-manager' },
    })
    expect(variantDefault(ADAPTER_VARIANTS[1], ROOT)).toEqual({
      name: 'bridge',
      config: { url: '', timeout_ms: 5000 },
    })
  })

  it('round-trips: a switched-to variant re-selects itself', () => {
    const bridgeDefault = variantDefault(ADAPTER_VARIANTS[1], ROOT)
    expect(matchVariantIndex(ADAPTER_VARIANTS, bridgeDefault)).toBe(1)
  })

  it('defers to the shallow default for primitives', () => {
    expect(variantDefault({ type: 'string' }, ROOT)).toBe('')
    expect(variantDefault({ type: 'integer', default: 7 }, ROOT)).toBe(7)
  })
})

describe('variantLabel', () => {
  it('labels tagged object variants by their tag value', () => {
    expect(variantLabel(ADAPTER_VARIANTS[0], 0)).toBe('fs')
    expect(variantLabel(ADAPTER_VARIANTS[1], 1)).toBe('bridge')
  })

  it('prefers an explicit title, then enum, then type', () => {
    expect(variantLabel({ title: 'seconds', type: 'integer' }, 0)).toBe(
      'seconds',
    )
    expect(variantLabel({ type: 'string', enum: ['otlp'] }, 0)).toBe('otlp')
    expect(variantLabel({ type: 'integer' }, 0)).toBe('integer')
  })
})

describe('isSingleStringEnumVariant', () => {
  it('accepts single string enums and rejects others', () => {
    expect(isSingleStringEnumVariant({ type: 'string', enum: ['a'] })).toBe(
      true,
    )
    expect(isSingleStringEnumVariant({ enum: ['a'] })).toBe(true)
    expect(
      isSingleStringEnumVariant({ type: 'string', enum: ['a', 'b'] }),
    ).toBe(false)
    expect(isSingleStringEnumVariant({ type: 'integer', enum: [1] })).toBe(
      false,
    )
    expect(isSingleStringEnumVariant({ type: 'object' })).toBe(false)
  })
})
