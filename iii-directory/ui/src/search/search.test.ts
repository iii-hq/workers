import { describe, expect, it } from 'vitest'
import {
  discoverQuery,
  functionCount,
  isErrorOutput,
  parseDiscoverResponse,
  unwrapEnvelope,
} from './search'

const candidate = {
  function_id: 'github::issue::view',
  description: 'Show one issue.',
}

const response = {
  guidance: 'Choose candidates, then fetch their contracts in one batch.',
  workers: [{ namespace: 'github', functions: [candidate] }],
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

  it('accepts legacy schema-bearing candidates but keeps only compact fields', () => {
    expect(
      parseDiscoverResponse({
        ...response,
        workers: [
          {
            namespace: 'github',
            functions: [{ ...candidate, request_schema: { type: 'object' } }],
          },
        ],
      })?.workers[0].functions[0],
    ).toEqual(candidate)
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
      'candidate without function_id',
      {
        guidance: 'g',
        workers: [{ namespace: 'github', functions: [{ description: '' }] }],
        latency_ms: 1,
      },
    ],
    [
      'candidate without description',
      {
        guidance: 'g',
        workers: [{ namespace: 'github', functions: [{ function_id: 'github::x' }] }],
        latency_ms: 1,
      },
    ],
    ['string payload', '{"guidance":"g"}'],
    ['null payload', null],
  ])('rejects %s', (_name, payload) => {
    expect(parseDiscoverResponse(payload)).toBeNull()
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
          { namespace: 'a', functions: [candidate, candidate] },
          { namespace: 'b', functions: [candidate] },
        ],
      }),
    ).toBe(3)
  })
})
