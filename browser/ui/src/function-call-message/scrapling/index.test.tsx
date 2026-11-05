import type {
  FunctionCallMessage,
  FunctionCallRenderer,
  Host,
} from '@iii-dev/console-ui'
import { describe, expect, it, vi } from 'vitest'
import setup from '../../../page'
import { createScraplingRenderer } from './index'

// The shared components are the running console's at runtime; here the
// structural ones pass children through so the rendered text is inspectable.
vi.mock('@iii-dev/console-ui', () => {
  const passthrough = ({
    children,
    className,
  }: {
    children?: React.ReactNode
    className?: string
  }) => <div className={className}>{children}</div>
  return {
    ActionLine: passthrough,
    Badge: passthrough,
    Button: passthrough,
    Card: passthrough,
    Chip: passthrough,
    EmptyState: () => null,
    IconButton: passthrough,
    Input: () => null,
    JsonHighlight: () => null,
    MetaRow: passthrough,
    Skeleton: () => null,
    StatusDot: () => null,
    StatusPanel: () => null,
    Table: passthrough,
    TableBody: passthrough,
    TableCell: passthrough,
    TableRow: passthrough,
    TableViewport: passthrough,
    Toolbar: passthrough,
  }
})

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
      iii: { call: () => Promise.resolve({}) },
    } as unknown as Host
    const renderer = createScraplingRenderer(withHost)
    const done = {
      functionId: 'browser::fetch',
      input: { url: 'https://example.com' },
      output: { status: 200, url: 'https://example.com', content: 'hi' },
      running: false,
    } as unknown as FunctionCallMessage
    const html = renderToStaticMarkup(<>{renderer.tryRender?.(done)}</>)
    expect(html).toContain('Open in browser')
    expect(html).toContain('br-ui-open-in-browser')
  })

  it('links the fetched url and colours links in markdown content', async () => {
    const { renderToStaticMarkup } = await import('react-dom/server')
    const renderer = createScraplingRenderer(host)
    const done = {
      functionId: 'browser::fetch',
      input: { url: 'https://example.com', format: 'markdown' },
      output: {
        status: 200,
        url: 'https://example.com/',
        format: 'markdown',
        content: '### Title\n[Docs](/docs) see https://x.dev/a.',
      },
      running: false,
    } as unknown as FunctionCallMessage
    const html = renderToStaticMarkup(<>{renderer.tryRender?.(done)}</>)
    expect(html).toContain('href="https://example.com/"')
    expect(html).toContain('target="_blank"')
    expect(html).toContain('<span class="br-ui-md-mark">###</span>')
    expect(html).toContain('<span class="br-ui-md-link">Docs</span>')
    expect(html).toContain('<span class="br-ui-md-url">/docs</span>')
    // a sentence-ending period stays outside the url
    expect(html).toContain('<span class="br-ui-md-link">https://x.dev/a</span>.')
  })

  it('renders approval previews and parse results', () => {
    const renderer = createScraplingRenderer(host)
    const preview = {
      functionId: 'browser::fetch',
      input: { url: 'https://example.com' },
      pendingApproval: true,
    } as FunctionCallMessage
    const result = {
      functionId: 'browser::css',
      input: { html: '<p>x</p>', query: 'p' },
      output: { result: ['x'] },
    } as FunctionCallMessage
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
    } as FunctionCallMessage)
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
      } as FunctionCallMessage),
    ).not.toBeNull()
    expect(
      renderer.tryRender({
        functionId: 'browser::fetch',
        input: { urls: [] },
        output: { status: 200, url: 'https://example.com' },
      } as FunctionCallMessage),
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
      } as FunctionCallMessage)
      const running = renderer.tryRenderRunning?.({
        functionId,
        input,
        running: true,
      } as FunctionCallMessage)
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
        } as FunctionCallMessage),
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
        } as unknown as FunctionCallMessage),
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
      } as FunctionCallMessage),
    ).toBeNull()
  })

  it('falls through empty fetch payloads', () => {
    const renderer = createScraplingRenderer(host)
    expect(
      renderer.tryRender({
        functionId: 'browser::fetch',
        input: {},
        output: {},
      } as FunctionCallMessage),
    ).toBeNull()
  })

  it('falls through CSS requests missing required input', () => {
    const renderer = createScraplingRenderer(host)
    expect(
      renderer.tryRender({
        functionId: 'browser::css',
        input: { html: '<p>x</p>' },
        output: { result: ['x'] },
      } as FunctionCallMessage),
    ).toBeNull()
    expect(
      renderer.tryRender({
        functionId: 'browser::css',
        input: { query: 'p' },
        output: { result: ['x'] },
      } as FunctionCallMessage),
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
      } as unknown as FunctionCallMessage),
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
      } as FunctionCallMessage),
      renderer.tryRenderRunning?.({
        functionId: 'browser::screenshot-url',
        input,
        running: true,
      } as FunctionCallMessage),
      renderer.tryRender({
        functionId: 'browser::screenshot-url',
        input,
        output: {
          content: [{ type: 'image', mime: 'image/png', data: 'aGk=' }],
          url: 'https://example.com',
        },
      } as FunctionCallMessage),
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
        } as FunctionCallMessage),
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
      } as unknown as FunctionCallMessage),
    ).toBeNull()
  })

  it('registers the exact scraping renderer before the broad browser renderer', () => {
    const renderers: FunctionCallRenderer[] = []
    setup({
      pages: { register: () => () => {} },
      configForms: { register: () => () => {} },
      functionCalls: {
        register: (renderer: FunctionCallRenderer) => {
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
