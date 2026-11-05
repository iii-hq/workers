import { describe, expect, it } from 'vitest'
import {
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

describe('parseDiscoverResponse', () => {
  it('parses a flat response and an enveloped one identically', () => {
    const parsed = { ...response, installable: [], skills: [], observers: [] }
    expect(parseDiscoverResponse(response)).toEqual(parsed)
    expect(
      parseDiscoverResponse({ content: [{ type: 'text', text: 'x' }], details: response }),
    ).toEqual(parsed)
  })

  it('keeps empty worker lists (the refine-guidance card)', () => {
    const empty = { guidance: 'No functions matched…', workers: [], latency_ms: 3 }
    expect(parseDiscoverResponse(empty)).toEqual({ ...empty, installable: [], skills: [], observers: [] })
  })

  it('parses the search mode when present and ignores an unknown one', () => {
    const parsed = parseDiscoverResponse({ ...response, search_mode: 'judge' })
    expect(parsed?.searchMode).toBe('judge')
    // Older transcripts carry `jev`: the card still renders, without a badge.
    for (const mode of ['jev', 'quantum', 7]) {
      const legacy = parseDiscoverResponse({ ...response, search_mode: mode })
      expect(legacy).not.toBeNull()
      expect('searchMode' in (legacy as object)).toBe(false)
    }
    // Absent on legacy rows: the field is simply omitted.
    expect('searchMode' in (parseDiscoverResponse(response) as object)).toBe(false)
  })

  it('parses the installed skills section when present', () => {
    const parsed = parseDiscoverResponse({
      guidance: 'The `skills` entries are installed how-to documents…',
      workers: [],
      skills: [
        { id: 'cron', title: 'cron', description: 'Schedule any registered function on a cron expression.' },
      ],
      latency_ms: 7,
    })
    expect(parsed?.skills).toEqual([
      { id: 'cron', title: 'cron', description: 'Schedule any registered function on a cron expression.' },
    ])
    expect(parsed?.workers).toEqual([])
  })

  it('parses the observers section when present', () => {
    const parsed = parseDiscoverResponse({
      guidance: 'The `observers` entries are observers that already run a function…',
      workers: [],
      observers: [
        {
          id: 'o-1',
          function_id: 'cron::on-tick',
          target: 'harness::sweep-pending',
          worker_name: 'harness',
          args: { expression: '0 0 0 * * *' },
          filters: {},
        },
        {
          id: 'o-2',
          function_id: 'configuration::on-updated',
          target: 'queue::on-config-change',
          filters: { 'input.id': { $eq: 'queue' } },
        },
      ],
      latency_ms: 7,
    })
    expect(parsed?.observers).toEqual([
      {
        id: 'o-1',
        functionId: 'cron::on-tick',
        target: 'harness::sweep-pending',
        workerName: 'harness',
        args: { expression: '0 0 0 * * *' },
        filters: {},
      },
      {
        id: 'o-2',
        functionId: 'configuration::on-updated',
        target: 'queue::on-config-change',
        args: {},
        filters: { 'input.id': { $eq: 'queue' } },
      },
    ])
  })

  it('rejects a malformed observers section', () => {
    for (const observers of [
      {},
      [{ function_id: 'cron::on-tick', target: 'x' }], // no id
      [{ id: 'o', target: 'x' }], // no function_id
      [{ id: 'o', function_id: 'cron::on-tick' }], // no target
      [{ id: 'o', function_id: 'cron::on-tick', target: 'x', worker_name: 7 }],
    ]) {
      expect(parseDiscoverResponse({ guidance: 'g', workers: [], observers, latency_ms: 1 })).toBeNull()
    }
  })

  it('rejects a malformed skills section', () => {
    for (const skills of [
      {},
      [{ title: 'x', description: '' }], // no id
      [{ id: 'cron', title: 'cron' }], // no description
    ]) {
      expect(parseDiscoverResponse({ guidance: 'g', workers: [], skills, latency_ms: 1 })).toBeNull()
    }
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
            function: 'compose::add',
            payload: { worker: 'image-resize' },
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
        skills: [],
        observers: [],
        workers: [
          { namespace: 'a', functions: [candidate, candidate] },
          { namespace: 'b', functions: [candidate] },
        ],
      }),
    ).toBe(3)
  })
})
