import type {
  FunctionTriggerMessage,
  FunctionTriggerRenderer,
  Host,
} from '@iii-dev/console-ui'
import { describe, expect, it, vi } from 'vitest'
import setup from '../../../page'
import { createScraplingRenderer } from './index'

vi.mock('@iii-dev/console-ui', () => ({
  Badge: () => null,
  Button: () => null,
  Input: () => null,
  JsonHighlight: () => null,
  StatusDot: () => null,
}))

const host = {} as Host

describe('scraping renderer', () => {
  it('owns URL screenshot but not interactive session screenshot', () => {
    const renderer = createScraplingRenderer(host)
    expect(renderer.isMatch('browser::screenshot-url')).toBe(true)
    expect(renderer.isMatch('browser::screenshot')).toBe(false)
  })

  it('offers open-in-browser on a fetched url', async () => {
    const { renderToStaticMarkup } = await import('react-dom/server')
    const withHost = {
      iii: { trigger: () => Promise.resolve({}) },
    } as unknown as Host
    const renderer = createScraplingRenderer(withHost)
    const done = {
      functionId: 'browser::fetch',
      input: { url: 'https://example.com' },
      output: { status: 200, url: 'https://example.com', content: 'hi' },
      running: false,
    } as unknown as FunctionTriggerMessage
    const html = renderToStaticMarkup(<>{renderer.tryRender?.(done)}</>)
    expect(html).toContain('Open in browser')
    expect(html).toContain('br-ui-open-in-browser')
  })

  it('renders approval previews and parse results', () => {
    const renderer = createScraplingRenderer(host)
    const preview = {
      functionId: 'browser::fetch',
      input: { url: 'https://example.com' },
      pendingApproval: true,
    } as FunctionTriggerMessage
    const result = {
      functionId: 'browser::css',
      input: { html: '<p>x</p>', query: 'p' },
      output: { result: ['x'] },
    } as FunctionTriggerMessage
    expect(renderer.tryRenderPreview?.(preview)).not.toBeNull()
    expect(renderer.tryRender(result)).not.toBeNull()
  })

  it('redacts proxy-bearing values throughout raw payloads', () => {
    const renderer = createScraplingRenderer(host)
    const input = {
      proxy: 'http://user:pass@example.com',
      nested: [
        { proxies: { https: 'http://token@example.com' } },
        { proxy_auth: { username: 'user', password: 'pass' } },
        { proxying: 'unchanged' },
      ],
    }
    expect(renderer.redactRaw?.(input)).toEqual({
      proxy: '[redacted]',
      nested: [
        { proxies: '[redacted]' },
        { proxy_auth: '[redacted]' },
        { proxying: 'unchanged' },
      ],
    })
    expect(input.proxy).toBe('http://user:pass@example.com')
  })

  it('marks circular raw payloads without mutating them', () => {
    const renderer = createScraplingRenderer(host)
    const input: Record<string, unknown> = { proxy: 'secret' }
    input.self = input
    expect(renderer.redactRaw?.(input)).toEqual({
      proxy: '[redacted]',
      self: '[circular]',
    })
    expect(input.self).toBe(input)
  })

  it('redacts proxy URL userinfo embedded in raw strings', () => {
    const renderer = createScraplingRenderer(host)
    const error =
      'proxy is not usable: http://user:pass@proxy.example: invalid endpoint'
    const redacted =
      'proxy is not usable: http://[redacted]@proxy.example: invalid endpoint'
    expect(renderer.redactRaw?.(error)).toBe(redacted)
    expect(renderer.redactRaw?.({ nested: { error } })).toEqual({
      nested: { error: redacted },
    })
    expect(renderer.redactRaw?.('request failed for https://example.com/a')).toBe(
      'request failed for https://example.com/a',
    )
  })

  it('redacts proxy credentials in rich error cards', () => {
    const renderer = createScraplingRenderer(host)
    const rendered = renderer.tryRender({
      functionId: 'browser::fetch',
      input: { url: 'https://example.com' },
      output: {
        error: {
          kind: 'function_error',
          message:
            'proxy failed: http://user:pass@proxy.example is unavailable',
        },
      },
    } as FunctionTriggerMessage)
    const serialized = JSON.stringify(rendered)
    expect(serialized).toContain('[redacted]')
    expect(serialized).not.toContain('user:pass')
  })

  it('falls back from empty urls to the single fetch url', () => {
    const renderer = createScraplingRenderer(host)
    expect(
      renderer.tryRender({
        functionId: 'browser::fetch',
        input: { url: 'https://example.com', urls: [] },
        output: { status: 200, url: 'https://example.com' },
      } as FunctionTriggerMessage),
    ).not.toBeNull()
    expect(
      renderer.tryRender({
        functionId: 'browser::fetch',
        input: { urls: [] },
        output: { status: 200, url: 'https://example.com' },
      } as FunctionTriggerMessage),
    ).toBeNull()
  })

  it('renders approval and running states for close and list sessions', () => {
    const renderer = createScraplingRenderer(host)
    const sessionId = '1234567890abcdef'
    for (const [functionId, input] of [
      ['browser::session-close', { session_id: sessionId }],
      ['browser::session-list', {}],
    ] as const) {
      const preview = renderer.tryRenderPreview?.({
        functionId,
        input,
        pendingApproval: true,
      } as FunctionTriggerMessage)
      const running = renderer.tryRenderRunning?.({
        functionId,
        input,
        running: true,
      } as FunctionTriggerMessage)
      expect(preview).not.toBeNull()
      expect(running).not.toBeNull()
      if (functionId === 'browser::session-close') {
        for (const rendered of [preview, running]) {
          const serialized = JSON.stringify(rendered)
          expect(serialized).toContain('1234567890…')
          expect(serialized).not.toContain(sessionId)
        }
      }
    }
  })

  it('validates terminal close and list session requests', () => {
    const renderer = createScraplingRenderer(host)
    for (const [functionId, input, output] of [
      ['browser::session-close', { session_id: 'session-1' }, { closed: true }],
      ['browser::session-list', { type: 'http' }, { sessions: [] }],
    ] as const) {
      expect(
        renderer.tryRender({
          functionId,
          input,
          output,
        } as FunctionTriggerMessage),
      ).not.toBeNull()
    }

    for (const [functionId, input, output] of [
      ['browser::session-close', {}, { closed: true }],
      ['browser::session-list', { type: 42 }, { sessions: [] }],
    ] as const) {
      expect(
        renderer.tryRender({
          functionId,
          input,
          output,
        } as unknown as FunctionTriggerMessage),
      ).toBeNull()
    }
  })

  it.each([
    ['browser::extract', { html: '<p>x</p>' }, { extracted: {} }],
    [
      'browser::find-similar',
      { html: '<p>x</p>' },
      { count: 0, items: [] },
    ],
    ['browser::find', {}, { count: 0, items: [] }],
    [
      'browser::find-by-text',
      { html: '<p>x</p>' },
      { count: 0, items: [] },
    ],
    [
      'browser::find-by-regex',
      { html: '<p>x</p>' },
      { count: 0, items: [] },
    ],
    ['browser::describe', { html: '<p>x</p>' }, { found: false }],
    ['browser::to-markdown', {}, { format: 'markdown', content: '' }],
    [
      'browser::session-fetch',
      { session_id: 'session-1' },
      { status: 200, url: 'https://example.com' },
    ],
    [
      'browser::crawl',
      {},
      { stats: { crawled: 0, items: 0, errors: 0 } },
    ],
  ])('falls through %s requests missing required fields or targets', (functionId, input, output) => {
    const renderer = createScraplingRenderer(host)
    expect(
      renderer.tryRender({
        functionId,
        input,
        output,
      } as FunctionTriggerMessage),
    ).toBeNull()
  })

  it('falls through empty fetch payloads', () => {
    const renderer = createScraplingRenderer(host)
    expect(
      renderer.tryRender({
        functionId: 'browser::fetch',
        input: {},
        output: {},
      } as FunctionTriggerMessage),
    ).toBeNull()
  })

  it('falls through CSS requests missing required input', () => {
    const renderer = createScraplingRenderer(host)
    expect(
      renderer.tryRender({
        functionId: 'browser::css',
        input: { html: '<p>x</p>' },
        output: { result: ['x'] },
      } as FunctionTriggerMessage),
    ).toBeNull()
    expect(
      renderer.tryRender({
        functionId: 'browser::css',
        input: { query: 'p' },
        output: { result: ['x'] },
      } as FunctionTriggerMessage),
    ).toBeNull()
  })

  it('falls through screenshots with an invalid request', () => {
    const renderer = createScraplingRenderer(host)
    expect(
      renderer.tryRender({
        functionId: 'browser::screenshot-url',
        input: { url: 42 },
        output: {
          content: [{ type: 'image', mime: 'image/png', data: 'aGk=' }],
          url: 'https://example.com',
        },
      } as unknown as FunctionTriggerMessage),
    ).toBeNull()
  })

  it('shows only proxy presence in screenshot states', () => {
    const renderer = createScraplingRenderer(host)
    const input = {
      url: 'https://example.com',
      proxy: 'http://user:pass@proxy.example',
    }
    const states = [
      renderer.tryRenderPreview?.({
        functionId: 'browser::screenshot-url',
        input,
        pendingApproval: true,
      } as FunctionTriggerMessage),
      renderer.tryRenderRunning?.({
        functionId: 'browser::screenshot-url',
        input,
        running: true,
      } as FunctionTriggerMessage),
      renderer.tryRender({
        functionId: 'browser::screenshot-url',
        input,
        output: {
          content: [{ type: 'image', mime: 'image/png', data: 'aGk=' }],
          url: 'https://example.com',
        },
      } as FunctionTriggerMessage),
    ]
    for (const state of states) {
      const serialized = JSON.stringify(state)
      expect(serialized).toContain('"proxy"')
      expect(serialized).not.toContain('user:pass')
      expect(serialized).not.toContain('proxy.example')
    }
  })

  it('falls through screenshot successes without image data', () => {
    const renderer = createScraplingRenderer(host)
    for (const content of [
      [],
      [{ type: 'text', text: 'Screenshot captured' }],
      [{ type: 'image', mime: 'image/png', data: '' }],
    ]) {
      expect(
        renderer.tryRender({
          functionId: 'browser::screenshot-url',
          input: { url: 'https://example.com' },
          output: { content, url: 'https://example.com' },
        } as FunctionTriggerMessage),
      ).toBeNull()
    }
  })

  it('falls through malformed result payloads', () => {
    const renderer = createScraplingRenderer(host)
    expect(
      renderer.tryRender({
        functionId: 'browser::css',
        input: { html: '<p>x</p>', query: 'p' },
        output: { result: 42 },
      } as unknown as FunctionTriggerMessage),
    ).toBeNull()
  })

  it('registers the exact scraping renderer before the broad browser renderer', () => {
    const renderers: FunctionTriggerRenderer[] = []
    setup({
      pages: { register: () => () => {} },
      configForms: { register: () => () => {} },
      functionTriggers: {
        register: (renderer: FunctionTriggerRenderer) => {
          renderers.push(renderer)
          return () => {}
        },
      },
    } as unknown as Host)
    expect(renderers.map((renderer) => renderer.id)).toEqual([
      'browser/page.js#screenshot-display',
      'browser/page.js#scraping-calls',
      'browser/page.js#calls',
    ])
  })
})
