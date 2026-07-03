import { describe, expect, it } from 'vitest'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import {
  fetchResponseSchema,
  findSimilarResponseSchema,
  isScraplingFunction,
  queryResponseSchema,
  SCRAPLING_FUNCTION_IDS,
  safeParseResponse,
  screenshotResponseSchema,
} from '../parsers'

/** Harness `{ content, details, terminate }` envelope, as agent-trigger wraps it. */
function wrapHarness(details: unknown) {
  return {
    content: [{ type: 'text', text: JSON.stringify(details) }],
    details,
    terminate: false,
  }
}

const page = {
  status: 200,
  url: 'https://example.com',
  headers: { 'content-type': 'text/html' },
  cookies: { sid: '1' },
  encoding: 'utf-8',
  extracted: { title: 'Example Domain' },
}

describe('scrapling function ids', () => {
  it('covers the full 9-function worker surface', () => {
    expect(SCRAPLING_FUNCTION_IDS).toHaveLength(9)
    for (const id of SCRAPLING_FUNCTION_IDS) {
      expect(isScraplingFunction(id)).toBe(true)
    }
    expect(isScraplingFunction('scrapling::nope')).toBe(false)
    expect(isScraplingFunction('web::fetch')).toBe(false)
  })
})

describe('fetch response', () => {
  it('parses a single page, harness-wrapped', () => {
    const parsed = safeParseResponse(fetchResponseSchema, wrapHarness(page))
    expect(parsed).not.toBeNull()
    expect(parsed && 'results' in parsed).toBe(false)
    expect(parsed && 'status' in parsed ? parsed.status : null).toBe(200)
  })

  it('parses a bulk response and keeps per-url error rows', () => {
    const bulk = {
      results: [page, { url: 'https://bad.example.com', error: 'boom' }],
    }
    const parsed = safeParseResponse(fetchResponseSchema, bulk)
    expect(parsed && 'results' in parsed ? parsed.results : []).toHaveLength(2)
    expect(
      parsed && 'results' in parsed ? parsed.results[1]?.error : null,
    ).toBe('boom')
  })

  it('never misreads a successful fetch as an infra error', () => {
    // `status` is a number here (unlike the state:: denial-shape hazard), so
    // the shared error parser must return null for every success shape.
    expect(parseSandboxErrorDisplay(wrapHarness(page))).toBeNull()
    expect(
      parseSandboxErrorDisplay(
        wrapHarness({
          results: [page, { url: 'https://bad.example.com', error: 'boom' }],
        }),
      ),
    ).toBeNull()
  })

  it('still surfaces a real function_error envelope', () => {
    const display = parseSandboxErrorDisplay({
      error: {
        kind: 'function_error',
        message: 'trigger_failed: provide `url` or `urls`',
        content: [{ type: 'text', text: 'provide `url` or `urls`' }],
      },
    })
    expect(display?.variant).toBe('invocation')
  })
})

describe('query response', () => {
  it('accepts a first-match string', () => {
    expect(safeParseResponse(queryResponseSchema, { result: 'Apple' })).toEqual(
      { result: 'Apple' },
    )
  })

  it('accepts an all-matches list with null attr misses', () => {
    const parsed = safeParseResponse(queryResponseSchema, {
      result: ['/a', null, '/b'],
    })
    expect(parsed?.result).toEqual(['/a', null, '/b'])
  })

  it('accepts a null no-match result', () => {
    expect(safeParseResponse(queryResponseSchema, { result: null })).toEqual({
      result: null,
    })
  })
})

describe('screenshot response', () => {
  it('parses the base64 payload', () => {
    const parsed = safeParseResponse(
      screenshotResponseSchema,
      wrapHarness({
        image_base64: 'aGk=',
        mime: 'image/png',
        url: 'https://example.com',
      }),
    )
    expect(parsed?.image_base64).toBe('aGk=')
  })
})

describe('find-similar response', () => {
  it('parses anchor+similar items', () => {
    const parsed = safeParseResponse(findSimilarResponseSchema, {
      count: 2,
      items: [{ text: 'Apple' }, { text: 'Banana' }],
    })
    expect(parsed?.count).toBe(2)
    expect(parsed?.items).toHaveLength(2)
  })
})
