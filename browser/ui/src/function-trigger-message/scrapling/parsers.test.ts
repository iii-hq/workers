import { describe, expect, it } from 'vitest'
import {
  fetchEngineLabel,
  isScraplingFunction,
  safeParseResponse,
  screenshotResponseSchema,
  SCRAPLING_FUNCTION_IDS,
} from './parsers'

describe('root browser scraping ids', () => {
  it('claims all 19 scraping functions without stealing session screenshot', () => {
    expect(SCRAPLING_FUNCTION_IDS).toEqual([
      'browser::fetch',
      'browser::stealthy-fetch',
      'browser::dynamic-fetch',
      'browser::screenshot-url',
      'browser::extract',
      'browser::css',
      'browser::xpath',
      'browser::regex',
      'browser::find-similar',
      'browser::find',
      'browser::find-by-text',
      'browser::find-by-regex',
      'browser::describe',
      'browser::to-markdown',
      'browser::session-open',
      'browser::session-fetch',
      'browser::session-close',
      'browser::session-list',
      'browser::crawl',
    ])
    expect(isScraplingFunction('browser::screenshot-url')).toBe(true)
    expect(isScraplingFunction('browser::screenshot')).toBe(false)
    expect(isScraplingFunction('browser::scrapling::fetch')).toBe(false)
    expect(fetchEngineLabel('browser::stealthy-fetch')).toBe('stealth')
  })

  it('decodes native screenshot tiles from a harness envelope', () => {
    const parsed = safeParseResponse(screenshotResponseSchema, {
      content: [{ type: 'text', text: 'caption' }],
      details: {
        content: [{ type: 'image', mime: 'image/png', data: 'aGk=' }],
        mime: 'image/png',
        url: 'https://example.com',
      },
    })
    expect(parsed?.content[0]?.data).toBe('aGk=')
  })
})
