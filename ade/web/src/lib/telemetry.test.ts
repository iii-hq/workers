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
