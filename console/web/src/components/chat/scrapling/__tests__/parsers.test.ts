import { describe, expect, it } from 'vitest'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import {
  crawlResponseSchema,
  describeResponseSchema,
  elementsResponseSchema,
  fetchResponseSchema,
  findSimilarResponseSchema,
  isScraplingFunction,
  markdownResponseSchema,
  queryResponseSchema,
  SCRAPLING_FUNCTION_IDS,
  safeParseResponse,
  screenshotResponseSchema,
  sessionListResponseSchema,
  sessionOpenResponseSchema,
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
  it('covers the full worker surface', () => {
    expect(SCRAPLING_FUNCTION_IDS).toHaveLength(19)
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

  it('parses a page with a rendered markdown `content` body', () => {
    const parsed = safeParseResponse(
      fetchResponseSchema,
      wrapHarness({
        status: 200,
        url: 'https://example.com',
        headers: {},
        cookies: {},
        encoding: 'utf-8',
        format: 'markdown',
        content: '# Title\n\nbody',
      }),
    )
    expect(parsed && 'content' in parsed ? parsed.content : null).toContain(
      '# Title',
    )
    expect(parsed && 'format' in parsed ? parsed.format : null).toBe('markdown')
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

describe('element search response (find / find-by-text / find-by-regex)', () => {
  it('parses matched elements with generated selectors, harness-wrapped', () => {
    const parsed = safeParseResponse(
      elementsResponseSchema,
      wrapHarness({
        count: 1,
        items: [
          {
            tag: 'a',
            text: 'Apple',
            html: '<a href="/a">Apple</a>',
            attrs: { href: '/a' },
            css: 'body > ul > li > a',
            xpath: '//body/ul/li/a',
          },
        ],
      }),
    )
    expect(parsed?.count).toBe(1)
    expect(parsed?.items[0]?.css).toBe('body > ul > li > a')
  })

  it('parses an empty (no-match) result', () => {
    expect(
      safeParseResponse(elementsResponseSchema, { count: 0, items: [] })?.count,
    ).toBe(0)
  })
})

describe('describe response', () => {
  it('parses a found element with full selectors + DOM context', () => {
    const parsed = safeParseResponse(describeResponseSchema, {
      found: true,
      element: {
        tag: 'h1',
        css: 'body > h1',
        full_css: 'html > body > h1.t',
        xpath: '//body/h1',
        full_xpath: '/html/body/h1',
        classes: ['t'],
        parent_tag: 'body',
        children: 0,
        siblings: 1,
      },
    })
    expect(parsed?.found).toBe(true)
    expect(parsed?.element?.classes).toEqual(['t'])
  })

  it('parses a not-found result', () => {
    const parsed = safeParseResponse(describeResponseSchema, { found: false })
    expect(parsed?.found).toBe(false)
    expect(parsed?.element).toBeUndefined()
  })
})

describe('to-markdown response', () => {
  it('parses the converted content', () => {
    const parsed = safeParseResponse(
      markdownResponseSchema,
      wrapHarness({ format: 'markdown', content: '# Title\n\nbody' }),
    )
    expect(parsed?.format).toBe('markdown')
    expect(parsed?.content).toContain('# Title')
  })
})

describe('session responses', () => {
  it('parses session-open with the returned id', () => {
    const parsed = safeParseResponse(
      sessionOpenResponseSchema,
      wrapHarness({ session_id: 'abc123', type: 'stealthy' }),
    )
    expect(parsed?.session_id).toBe('abc123')
    expect(parsed?.type).toBe('stealthy')
  })

  it('parses session-list rows with idle time', () => {
    const parsed = safeParseResponse(sessionListResponseSchema, {
      sessions: [{ session_id: 'abc', type: 'http', idle_s: 12.3 }],
    })
    expect(parsed?.sessions).toHaveLength(1)
    expect(parsed?.sessions[0]?.idle_s).toBe(12.3)
  })
})

describe('crawl response', () => {
  it('parses stats + sample items + stream ref, keeping error rows', () => {
    const parsed = safeParseResponse(
      crawlResponseSchema,
      wrapHarness({
        stats: { crawled: 3, items: 2, errors: 1, stopped: 'done' },
        items: [
          { url: 'https://s/', status: 200, extracted: { t: 'Home' } },
          { url: 'https://s/gone', error: 'dns failed' },
        ],
        stream: { name: 'scrapling::crawl', group_id: 'g1' },
      }),
    )
    expect(parsed?.stats.crawled).toBe(3)
    expect(parsed?.stats.errors).toBe(1)
    expect(parsed?.items?.[1]?.error).toBe('dns failed')
    expect(parsed?.stream?.group_id).toBe('g1')
  })
})
