import { describe, expect, it } from 'vitest'
import {
  discoverQuery,
  functionCount,
  isErrorOutput,
  parseDiscoverResponse,
  schemaIsAny,
  unwrapEnvelope,
} from './search'

const contract = {
  function_id: 'github::issue::view',
  description: 'Show one issue.',
  request_schema: {
    type: 'object',
    properties: { number: { type: 'number' } },
    required: ['number'],
  },
}

const response = {
  guidance: 'This API reference satisfies and OVERRIDES …',
  workers: [{ namespace: 'github', functions: [contract] }],
  latency_ms: 12.4,
}

describe('unwrapEnvelope', () => {
  it('unwraps the harness result envelope and passes flat payloads through', () => {
    expect(
      unwrapEnvelope({ content: [{ type: 'text', text: '{}' }], details: response }),
    ).toEqual(response)
    expect(unwrapEnvelope(response)).toEqual(response)
    expect(unwrapEnvelope('raw')).toBe('raw')
    expect(unwrapEnvelope(null)).toBeNull()
  })
})

describe('isErrorOutput', () => {
  it('flags only error-shaped objects', () => {
    expect(isErrorOutput({ error: { kind: 'function_error' } })).toBe(true)
    expect(isErrorOutput(response)).toBe(false)
    expect(isErrorOutput(null)).toBe(false)
    expect(isErrorOutput([])).toBe(false)
  })
})

describe('discoverQuery', () => {
  it('reads the query from objects and double-encoded strings', () => {
    expect(discoverQuery({ query: 'create a todo app' })).toBe('create a todo app')
    expect(discoverQuery(JSON.stringify({ query: 'read an issue' }))).toBe('read an issue')
  })

  it('returns null for blank, missing, or unreadable queries', () => {
    expect(discoverQuery({ query: '   ' })).toBeNull()
    expect(discoverQuery({})).toBeNull()
    expect(discoverQuery('not json')).toBeNull()
    expect(discoverQuery(null)).toBeNull()
  })
})

describe('parseDiscoverResponse', () => {
  it('parses a flat response and an enveloped one identically', () => {
    const parsed = { ...response, installable: [] }
    expect(parseDiscoverResponse(response)).toEqual(parsed)
    expect(
      parseDiscoverResponse({ content: [{ type: 'text', text: 'x' }], details: response }),
    ).toEqual(parsed)
  })

  it('keeps empty worker lists (the refine-guidance card)', () => {
    const empty = { guidance: 'No functions matched…', workers: [], latency_ms: 3 }
    expect(parseDiscoverResponse(empty)).toEqual({ ...empty, installable: [] })
  })

  it('parses the installable registry section when present', () => {
    const fn = { function_id: 'image_resize::resize', description: 'Resize an image.' }
    const parsed = parseDiscoverResponse({
      guidance: 'No INSTALLED function matched…',
      workers: [],
      installable: [
        {
          name: 'image-resize',
          version: '0.1.13',
          description: 'III engine image resize worker.',
          functions: [fn],
          install: {
            function: 'worker::add',
            payload: { source: { kind: 'registry', name: 'image-resize' }, wait: false },
          },
        },
      ],
      latency_ms: 5,
    })
    expect(parsed?.installable).toEqual([
      {
        name: 'image-resize',
        version: '0.1.13',
        description: 'III engine image resize worker.',
        functions: [fn],
      },
    ])
    expect(parsed?.workers).toEqual([])
  })

  it('rejects a malformed installable section', () => {
    for (const installable of [
      {},
      [{ version: '1', description: '', functions: [] }], // no name
      [{ name: 'x', version: '1', description: '', functions: [{}] }], // bad function row
    ]) {
      expect(
        parseDiscoverResponse({ guidance: 'g', workers: [], installable, latency_ms: 1 }),
      ).toBeNull()
    }
  })

  it.each([
    ['missing guidance', { workers: [], latency_ms: 1 }],
    ['non-finite latency', { guidance: 'g', workers: [], latency_ms: Number.NaN }],
    ['workers not an array', { guidance: 'g', workers: {}, latency_ms: 1 }],
    [
      'worker without namespace',
      { guidance: 'g', workers: [{ functions: [] }], latency_ms: 1 },
    ],
    [
      'contract without function_id',
      {
        guidance: 'g',
        workers: [{ namespace: 'github', functions: [{ description: '', request_schema: {} }] }],
        latency_ms: 1,
      },
    ],
    [
      'contract without request_schema',
      {
        guidance: 'g',
        workers: [
          { namespace: 'github', functions: [{ function_id: 'github::x', description: '' }] },
        ],
        latency_ms: 1,
      },
    ],
    ['string payload', '{"guidance":"g"}'],
    ['null payload', null],
  ])('rejects %s', (_name, payload) => {
    expect(parseDiscoverResponse(payload)).toBeNull()
  })
})

describe('schemaIsAny', () => {
  it('tags unconstraining schemas and keeps real ones expandable', () => {
    expect(schemaIsAny({})).toBe(true)
    expect(schemaIsAny({ type: 'object' })).toBe(true)
    expect(schemaIsAny({ $schema: 'x', title: 'T', type: 'object' })).toBe(true)
    expect(schemaIsAny(null)).toBe(true)
    expect(schemaIsAny(contract.request_schema)).toBe(false)
    expect(schemaIsAny({ type: 'string' })).toBe(false)
  })
})

describe('functionCount', () => {
  it('sums functions across workers', () => {
    expect(
      functionCount({
        guidance: 'g',
        latency_ms: 1,
        installable: [],
        workers: [
          { namespace: 'a', functions: [contract, contract] },
          { namespace: 'b', functions: [contract] },
        ],
      }),
    ).toBe(3)
  })
})
