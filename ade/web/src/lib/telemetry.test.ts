import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const capture = vi.fn()
const init = vi.fn()
vi.mock('posthog-js', () => ({ default: { capture, init } }))

const MIN = 60_000

function memoryStorage(): Storage {
  const entries = new Map<string, string>()
  return {
    get length() {
      return entries.size
    },
    clear: () => entries.clear(),
    getItem: (key) => entries.get(key) ?? null,
    key: (index) => [...entries.keys()][index] ?? null,
    removeItem: (key) => entries.delete(key),
    setItem: (key, value) => entries.set(key, value),
  }
}

async function load(
  telemetry: unknown = { enabled: true, api_key: 'phc_t', host: 'h' },
) {
  vi.resetModules()
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => ({ ok: true, json: async () => ({ telemetry }) })),
  )
  vi.stubGlobal('window', {
    location: { href: 'http://x/' },
    addEventListener: vi.fn(),
  })
  vi.stubGlobal('document', {
    visibilityState: 'visible',
    addEventListener: vi.fn(),
  })
  const mod = await import('./telemetry')
  await mod.initTelemetry()
  return mod
}

const usage = () => capture.mock.calls.filter(([name]) => name === 'ade_usage')

beforeEach(() => {
  vi.useFakeTimers()
  vi.stubGlobal('localStorage', memoryStorage())
  capture.mockClear()
  init.mockClear()
})

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('offsetAt', () => {
  it('walks the list, then adds one hour per report', async () => {
    const { offsetAt, POST_SETUP_OFFSETS, VISIT_OFFSETS } = await import(
      './telemetry'
    )
    expect(VISIT_OFFSETS.map((_, i) => offsetAt(VISIT_OFFSETS, i))).toEqual([
      15 * MIN,
      30 * MIN,
      45 * MIN,
      60 * MIN,
    ])
    expect(offsetAt(VISIT_OFFSETS, 4)).toBe(120 * MIN)
    expect(offsetAt(VISIT_OFFSETS, 5)).toBe(180 * MIN)
    expect(POST_SETUP_OFFSETS).toEqual([
      0.5 * MIN,
      MIN,
      5 * MIN,
      30 * MIN,
      60 * MIN,
    ])
    expect(offsetAt(POST_SETUP_OFFSETS, 5)).toBe(120 * MIN)
  })
})

describe('labels', () => {
  it('keeps only ids that hold no user data', async () => {
    const { commandLabel, screenKind } = await import('./telemetry')
    expect(commandLabel({ id: 'action:setup', kind: 'action' })).toBe(
      'action:setup',
    )
    expect(commandLabel({ id: 'command:panel.split', kind: 'command' })).toBe(
      'command:panel.split',
    )
    expect(commandLabel({ id: 'file:/home/me/secret.txt', kind: 'file' })).toBe(
      'file',
    )
    expect(commandLabel({ id: 'chat:s_123', kind: 'chat' })).toBe('chat')
    expect(screenKind('chat:s_123')).toBe('chat')
    expect(screenKind('traces')).toBe('traces')
  })
})

describe('telemetry', () => {
  it('sends nothing and never inits when the worker says it is off', async () => {
    const t = await load({ enabled: false })
    t.trackScreens(['chat'])
    t.trackCommand({ id: 'action:setup', kind: 'action' })
    vi.advanceTimersByTime(3 * 60 * MIN)
    expect(init).not.toHaveBeenCalled()
    expect(capture).not.toHaveBeenCalled()
  })

  it('keeps URL hashes, query values and the referrer out of every event', async () => {
    await load()
    expect(init.mock.calls[0][1]).toMatchObject({
      disable_capture_url_hashes: true,
      mask_personal_data_properties: true,
      save_referrer: false,
    })
  })

  it('loads no per-click capture and no surveys', async () => {
    await load()
    expect(init.mock.calls[0][1]).toMatchObject({
      autocapture: false,
      rageclick: false,
      capture_dead_clicks: false,
      disable_surveys: true,
    })
  })

  it('reports a visit at 15, 30, 45, 60 minutes, then hourly, with the delta only', async () => {
    const t = await load()
    expect(init).toHaveBeenCalledOnce()
    t.trackScreens(['chat', 'traces'])
    expect(capture).toHaveBeenCalledWith(
      '$pageview',
      { screens: ['chat', 'traces'] },
      undefined,
    )

    vi.advanceTimersByTime(15 * MIN)
    expect(usage()).toHaveLength(1)
    expect(usage()[0][1]).toEqual({
      commands: [],
      screens: {
        chat: { opens: 1, ms: 15 * MIN },
        traces: { opens: 1, ms: 15 * MIN },
      },
    })

    t.trackScreens(['workers'])
    vi.advanceTimersByTime(15 * MIN)
    expect(usage()[1][1].screens).toEqual({
      workers: { opens: 1, ms: 15 * MIN },
    })

    vi.advanceTimersByTime(30 * MIN)
    expect(usage()).toHaveLength(4)
    vi.advanceTimersByTime(59 * MIN)
    expect(usage()).toHaveLength(4)
    vi.advanceTimersByTime(MIN)
    expect(usage()).toHaveLength(5)
  })

  it('skips an empty report', async () => {
    await load()
    vi.advanceTimersByTime(15 * MIN)
    expect(usage()).toHaveLength(0)
  })

  it('keeps only the last 25 commands', async () => {
    const t = await load()
    for (let i = 0; i < 30; i++)
      t.trackCommand({ id: `action:a${i}`, kind: 'action' })
    vi.advanceTimersByTime(15 * MIN)
    const commands = usage()[0][1].commands
    expect(commands).toHaveLength(25)
    expect(commands[0]).toBe('action:a5')
    expect(commands[24]).toBe('action:a29')
  })

  it('pauses screen time while the page is hidden', async () => {
    const t = await load()
    t.trackScreens(['chat'])
    vi.advanceTimersByTime(5 * MIN)
    t.setVisible(false)
    vi.advanceTimersByTime(5 * MIN)
    t.setVisible(true)
    vi.advanceTimersByTime(5 * MIN)
    expect(usage()[0][1].screens.chat.ms).toBe(10 * MIN)
  })

  it('runs the post-setup schedule once, then hourly', async () => {
    const t = await load()
    t.trackScreens(['chat'])
    t.setupCompleted()
    for (const minutes of [0.5, 0.5, 4, 25, 30, 60]) {
      t.trackCommand({ id: 'action:theme', kind: 'action' })
      vi.advanceTimersByTime(minutes * MIN)
    }
    expect(usage()).toHaveLength(6)

    capture.mockClear()
    t.setupCompleted()
    t.trackCommand({ id: 'action:theme', kind: 'action' })
    vi.advanceTimersByTime(MIN)
    expect(usage()).toHaveLength(0)
  })

  it('flushes on page close by beacon', async () => {
    const t = await load()
    t.trackCommand({ id: 'action:theme', kind: 'action' })
    t.flush('sendBeacon')
    expect(usage()[0][2]).toEqual({
      transport: 'sendBeacon',
      send_instantly: true,
    })
  })
})

describe('scrubEvent', () => {
  it('is the before_send hook', async () => {
    const t = await load()
    expect(init.mock.calls[0][1].before_send).toBe(t.scrubEvent)
  })

  it('drops the query string from every URL property', async () => {
    const { scrubEvent } = await import('./telemetry')
    const url = 'http://127.0.0.1:3113/?token=secret&context=%2Fhome%2Fme'
    const event = scrubEvent({
      uuid: 'u',
      event: '$pageview',
      properties: {
        $current_url: url,
        $session_entry_url: url,
        $referrer: url,
        $pathname: '/',
        screens: ['chat'],
      },
      $set: { $current_url: url },
      $set_once: { $initial_current_url: url, $initial_referrer: url },
    })
    expect(JSON.stringify(event)).not.toContain('secret')
    expect(JSON.stringify(event)).not.toContain('context')
    expect(event?.properties.$current_url).toBe('http://127.0.0.1:3113/')
    expect(event?.$set_once?.$initial_current_url).toBe(
      'http://127.0.0.1:3113/',
    )
    expect(event?.properties.screens).toEqual(['chat'])
  })

  it('drops the query string from URLs nested in web vitals', async () => {
    const { scrubEvent } = await import('./telemetry')
    const url = 'http://127.0.0.1:3113/?token=secret#/traces'
    const event = scrubEvent({
      uuid: 'u',
      event: '$web_vitals',
      properties: {
        $web_vitals_LCP_event: {
          name: 'LCP',
          value: 120,
          navigationURL: url,
          $current_url: url,
          attribution: { url },
        },
      },
    })
    expect(JSON.stringify(event)).not.toContain('secret')
    expect(JSON.stringify(event)).not.toContain('#/traces')
    expect(event?.properties.$web_vitals_LCP_event).toMatchObject({
      name: 'LCP',
      value: 120,
      navigationURL: 'http://127.0.0.1:3113/',
    })
  })

  it('drops the query string from heatmap URL keys and keeps every point', async () => {
    const { scrubEvent } = await import('./telemetry')
    const event = scrubEvent({
      uuid: 'u',
      event: '$web_vitals',
      properties: {
        $heatmap_data: {
          'http://127.0.0.1:3113/?token=secret': [{ x: 1, y: 1 }],
          'http://127.0.0.1:3113/?token=other': [{ x: 2, y: 2 }],
        },
      },
    })
    expect(JSON.stringify(event)).not.toContain('secret')
    expect(event?.properties.$heatmap_data).toEqual({
      'http://127.0.0.1:3113/': [
        { x: 1, y: 1 },
        { x: 2, y: 2 },
      ],
    })
  })

  it('sends no referrer, initial or current', async () => {
    const { scrubEvent } = await import('./telemetry')
    const referrer = 'https://github.com/acme/private-repo/issues/12'
    const event = scrubEvent({
      uuid: 'u',
      event: '$pageview',
      properties: {
        $referrer: referrer,
        $referring_domain: 'github.com',
        $session_entry_referrer: referrer,
        $session_entry_referring_domain: 'github.com',
        screens: ['chat'],
      },
      $set: { $referrer: referrer, $referring_domain: 'github.com' },
      $set_once: {
        $initial_referrer: referrer,
        $initial_referring_domain: 'github.com',
        $initial_current_url: 'http://127.0.0.1:3113/',
      },
    })
    expect(JSON.stringify(event)).not.toContain('github')
    expect(event?.$set_once).toEqual({
      $initial_current_url: 'http://127.0.0.1:3113/',
    })
  })

  it('stores no campaign parameters', async () => {
    await load()
    expect(init.mock.calls[0][1]).toMatchObject({ save_campaign_params: false })
  })

  it('still scrubs exceptions', async () => {
    const { scrubEvent } = await import('./telemetry')
    const event = scrubEvent({
      uuid: 'u',
      event: '$exception',
      properties: {
        $current_url: 'http://x/?q=secret',
        $exception_list: [{ type: 'TypeError', value: 'secret text' }],
      },
    })
    expect(JSON.stringify(event)).not.toContain('secret')
  })
})

describe('scrubException', () => {
  it('keeps only the error type of an exception', async () => {
    const { scrubException } = await import('./telemetry')
    const event = scrubException({
      uuid: 'u',
      event: '$exception',
      properties: {
        $exception_list: [
          {
            type: 'TypeError',
            value: "cannot read '/home/me/secret.txt'",
            stacktrace: {
              type: 'raw',
              frames: [{ filename: 'app.js', lineno: 3 }],
            },
          },
        ],
        $exception_message: "cannot read '/home/me/secret.txt'",
        $exception_values: ["cannot read '/home/me/secret.txt'"],
        $exception_steps: [{ message: 'typed text' }],
        $exception_type: 'TypeError',
      },
    })
    expect(JSON.stringify(event)).not.toContain('secret')
    expect(JSON.stringify(event)).not.toContain('typed text')
    expect(event?.properties.$exception_list).toEqual([
      {
        type: 'TypeError',
        value: 'TypeError',
        stacktrace: {
          type: 'raw',
          frames: [{ filename: 'app.js', lineno: 3 }],
        },
      },
    ])
    expect(event?.properties.$exception_message).toBe('TypeError')
  })

  it('leaves other events alone', async () => {
    const { scrubException } = await import('./telemetry')
    const event = {
      uuid: 'u',
      event: 'ade_usage',
      properties: { commands: ['x'] },
    }
    expect(scrubException(event)).toEqual(event)
  })
})
