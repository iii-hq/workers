import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const presence = vi.hoisted(() => ({ registered: new Set<string>() }))

vi.mock('@/lib/function-presence', () => ({
  functionRegistered: async (id: string) => presence.registered.has(id),
}))

const shared = vi.hoisted(() => ({
  trigger: vi.fn<(id: string, payload?: unknown) => Promise<unknown>>(),
}))

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ trigger: shared.trigger }),
}))

import {
  __resetChromiumMissingForTests,
  browserLabel,
  type ChromiumInstallProgress,
  type ChromiumStatusResponse,
  checkChromiumMissing,
  chromiumMissing,
  describeProgress,
  followChromiumInstall,
  manualInstall,
  megabytes,
  mentionsChromiumMissing,
  parseProgress,
  progressFraction,
  readChromiumState,
  stateFromDoctor,
  stateFromStatus,
} from './chromium'

const notFound = () =>
  new Error('function_not_found: browser::chromium::status')

const MISSING: ChromiumStatusResponse = {
  found: false,
  engine: 'chromium',
  searched: ['/usr/bin/chromium', '/usr/bin/google-chrome'],
  install: {
    supported: true,
    platform: 'linux64',
    dir: '~/.cache/iii/browser/chrome',
    approx_download_mb: 172,
  },
}

function event(
  phase: ChromiumInstallProgress['phase'],
  extra: Partial<ChromiumInstallProgress> = {},
): ChromiumInstallProgress {
  return {
    job_id: 'job-1',
    phase,
    bytes_done: 0,
    timestamp: 1,
    ...extra,
  }
}

describe('stateFromStatus', () => {
  it('reads a found Chromium with where it came from', () => {
    const state = stateFromStatus({
      found: true,
      path: '/usr/bin/chromium',
      version: 'Chromium 131.0.6778.85',
      source: 'system',
      engine: 'chromium',
      install: { supported: true, platform: 'linux64', dir: '/c' },
    })
    expect(state).toMatchObject({
      found: true,
      path: '/usr/bin/chromium',
      source: 'system',
      canInstall: true,
      legacy: false,
    })
    expect(chromiumMissing(state)).toBe(false)
  })

  it('reads a missing Chromium the worker can download', () => {
    const state = stateFromStatus(MISSING)
    expect(state).toMatchObject({
      found: false,
      canInstall: true,
      platform: 'linux64',
      installDir: '~/.cache/iii/browser/chrome',
      approxDownloadMb: 172,
    })
    expect(chromiumMissing(state)).toBe(true)
  })

  it('keeps the reason when the platform has no download', () => {
    const state = stateFromStatus({
      found: false,
      engine: 'chromium',
      install: {
        supported: false,
        dir: '/c',
        approx_download_mb: 0,
        reason: 'Chrome for Testing has no build for freebsd x86_64',
      },
    })
    expect(state.canInstall).toBe(false)
    expect(state.installReason).toContain('freebsd')
    // No size from the worker: the usual one.
    expect(state.approxDownloadMb).toBe(200)
  })

  it('carries the latest download job', () => {
    const state = stateFromStatus({
      ...MISSING,
      job: event('downloading', { bytes_done: 10, bytes_total: 100 }),
    })
    expect(state.job?.phase).toBe('downloading')
  })

  it('never calls a Lightpanda worker short of Chromium', () => {
    expect(
      chromiumMissing(stateFromStatus({ ...MISSING, engine: 'lightpanda' })),
    ).toBe(false)
    expect(chromiumMissing(null)).toBe(false)
  })
})

describe('stateFromDoctor', () => {
  it('reads an older worker with no Chromium as missing, manual only', () => {
    const state = stateFromDoctor({ engine: 'chromium' })
    expect(state).toMatchObject({ found: false, canInstall: false })
    expect(state.legacy).toBe(true)
    expect(chromiumMissing(state)).toBe(true)
  })

  it('reads its chromium_path as found', () => {
    const state = stateFromDoctor({
      engine: 'chromium',
      chromium_path: '/usr/bin/google-chrome',
      chromium_version: 'Google Chrome 131',
    })
    expect(state).toMatchObject({
      found: true,
      version: 'Google Chrome 131',
    })
  })
})

describe('readChromiumState', () => {
  it('asks browser::chromium::status', async () => {
    const trigger = vi.fn(async () => MISSING)
    const state = await readChromiumState({ trigger } as never)
    expect(trigger).toHaveBeenCalledWith(
      'browser::chromium::status',
      {},
      expect.anything(),
    )
    expect(state?.canInstall).toBe(true)
  })

  it('falls back to browser::doctor on a worker without it', async () => {
    const trigger = vi.fn(async (id: string) => {
      if (id === 'browser::chromium::status') throw notFound()
      return { engine: 'chromium' }
    })
    const state = await readChromiumState({ trigger } as never)
    expect(trigger).toHaveBeenLastCalledWith(
      'browser::doctor',
      {},
      expect.anything(),
    )
    expect(state).toMatchObject({ found: false, legacy: true })
  })

  it('is null when no browser worker answers', async () => {
    const trigger = vi.fn(async () => {
      throw notFound()
    })
    await expect(readChromiumState({ trigger } as never)).resolves.toBeNull()
  })

  it('rejects on any other failure', async () => {
    const trigger = vi.fn(async () => {
      throw new Error('timeout')
    })
    await expect(readChromiumState({ trigger } as never)).rejects.toThrow(
      'timeout',
    )
  })
})

/** A fake bus: install answers, status answers, events are pushed by hand. */
function bus({
  install = { job_id: 'job-1', status: 'started' } as unknown,
  status = MISSING as ChromiumStatusResponse,
  installError = null as unknown,
} = {}) {
  const calls: string[] = []
  let handler: ((payload: unknown) => void) | null = null
  const unsubscribe = vi.fn()
  const trigger = vi.fn(async (id: string) => {
    calls.push(id)
    if (id === 'browser::chromium::install') {
      if (installError) throw installError
      return install
    }
    if (id === 'browser::chromium::status') return status
    throw notFound()
  })
  const subscribe = vi.fn(
    async (
      type: string,
      _config: Record<string, unknown>,
      onEvent: (payload: unknown) => void,
    ) => {
      calls.push(`subscribe ${type}`)
      handler = onEvent
      return unsubscribe
    },
  )
  return {
    calls,
    trigger,
    subscribe,
    unsubscribe,
    emit: (payload: unknown) => handler?.(payload),
    setStatus: (next: ChromiumStatusResponse) => {
      status = next
    },
  }
}

const flush = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve()
}

describe('followChromiumInstall', () => {
  it('subscribes to progress before it starts the download', async () => {
    const fake = bus()
    const seen: string[] = []
    const done = followChromiumInstall({
      client: fake as never,
      subscribe: fake.subscribe as never,
      onProgress: (progress) => seen.push(progress.phase),
    })
    await flush()
    expect(fake.calls).toEqual([
      'subscribe browser::chromium-install-progress',
      'browser::chromium::install',
    ])
    fake.emit(event('downloading', { bytes_done: 5, bytes_total: 10 }))
    fake.emit(event('done', { version: '131.0', path: '/c/131/chrome' }))
    await expect(done).resolves.toEqual({
      ok: true,
      version: '131.0',
      path: '/c/131/chrome',
    })
    expect(seen).toEqual(['downloading', 'done'])
    expect(fake.unsubscribe).toHaveBeenCalledTimes(1)
  })

  it('ignores other jobs and replays this job’s early events', async () => {
    const fake = bus()
    const seen: string[] = []
    let releaseInstall: (value: unknown) => void = () => undefined
    fake.trigger.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          releaseInstall = resolve
        }),
    )
    const done = followChromiumInstall({
      client: fake as never,
      subscribe: fake.subscribe as never,
      onProgress: (progress) =>
        seen.push(`${progress.job_id}:${progress.phase}`),
    })
    await flush()
    // Before install answered: one event of ours, one of someone else's.
    fake.emit(event('resolving'))
    fake.emit({ ...event('downloading'), job_id: 'job-2' })
    releaseInstall({ job_id: 'job-1', status: 'started' })
    await flush()
    fake.emit({ ...event('done'), job_id: 'job-2' })
    fake.emit(event('failed', { error: 'disk full', hint: 'free 400 MB' }))
    await expect(done).resolves.toEqual({
      ok: false,
      error: 'disk full',
      hint: 'free 400 MB',
    })
    expect(seen).toEqual(['job-1:resolving', 'job-1:failed'])
  })

  it('finishes at once when Chromium is already installed', async () => {
    const fake = bus({
      install: {
        status: 'already-installed',
        version: '131.0',
        path: '/usr/bin/chromium',
      },
    })
    await expect(
      followChromiumInstall({
        client: fake as never,
        subscribe: fake.subscribe as never,
        onProgress: () => undefined,
      }),
    ).resolves.toEqual({
      ok: true,
      version: '131.0',
      path: '/usr/bin/chromium',
    })
    expect(fake.unsubscribe).toHaveBeenCalled()
  })

  it('says why the platform cannot download, without the marker', async () => {
    const fake = bus({
      installError: new Error(
        'chromium_install_unsupported: no Chrome for Testing build for linux arm64',
      ),
    })
    await expect(
      followChromiumInstall({
        client: fake as never,
        subscribe: fake.subscribe as never,
        onProgress: () => undefined,
      }),
    ).resolves.toEqual({
      ok: false,
      error: 'no Chrome for Testing build for linux arm64',
    })
  })

  describe('after 30 s without an event', () => {
    beforeEach(() => {
      vi.useFakeTimers()
    })
    afterEach(() => {
      vi.useRealTimers()
    })

    it('reads status once and finishes on its terminal job', async () => {
      const fake = bus()
      const done = followChromiumInstall({
        client: fake as never,
        subscribe: fake.subscribe as never,
        onProgress: () => undefined,
      })
      await flush()
      fake.emit(event('downloading', { bytes_done: 1, bytes_total: 10 }))
      fake.setStatus({
        ...MISSING,
        found: true,
        version: '131.0',
        job: event('done', { version: '131.0', path: '/c/chrome' }),
      })
      await vi.advanceTimersByTimeAsync(29_999)
      expect(fake.trigger).not.toHaveBeenCalledWith(
        'browser::chromium::status',
        expect.anything(),
        expect.anything(),
      )
      await vi.advanceTimersByTimeAsync(1)
      await expect(done).resolves.toEqual({
        ok: true,
        version: '131.0',
        path: '/c/chrome',
      })
      const reads = fake.calls.filter(
        (id) => id === 'browser::chromium::status',
      )
      expect(reads).toHaveLength(1)
    })

    it('never reads again on its own: only a new event re-arms the wait', async () => {
      const fake = bus()
      const done = followChromiumInstall({
        client: fake as never,
        subscribe: fake.subscribe as never,
        onProgress: () => undefined,
      })
      await flush()
      fake.emit(event('extracting'))
      fake.setStatus({ ...MISSING, job: event('extracting') })
      await vi.advanceTimersByTimeAsync(30_000)
      await vi.advanceTimersByTimeAsync(10 * 60_000)
      const reads = () =>
        fake.calls.filter((id) => id === 'browser::chromium::status').length
      expect(reads()).toBe(1)
      fake.emit(event('verifying'))
      fake.emit(event('done', { version: '131.0' }))
      await expect(done).resolves.toMatchObject({ ok: true })
      expect(reads()).toBe(1)
    })

    it('gives up plainly when no event ever reached the page', async () => {
      const fake = bus({
        status: { ...MISSING, job: event('downloading') },
      })
      const done = followChromiumInstall({
        client: fake as never,
        subscribe: fake.subscribe as never,
        onProgress: () => undefined,
      })
      await flush()
      await vi.advanceTimersByTimeAsync(30_000)
      await expect(done).resolves.toMatchObject({
        ok: false,
        error: expect.stringContaining('cannot follow this download live'),
      })
    })

    it('reports a download the worker lost (it restarted)', async () => {
      const fake = bus({ status: MISSING })
      const done = followChromiumInstall({
        client: fake as never,
        subscribe: fake.subscribe as never,
        onProgress: () => undefined,
      })
      await flush()
      fake.emit(event('downloading'))
      await vi.advanceTimersByTimeAsync(30_000)
      await expect(done).resolves.toMatchObject({
        ok: false,
        error: expect.stringContaining('download stopped'),
      })
    })
  })

  it('stops following when aborted', async () => {
    const fake = bus()
    const abort = new AbortController()
    const done = followChromiumInstall({
      client: fake as never,
      subscribe: fake.subscribe as never,
      onProgress: () => undefined,
      signal: abort.signal,
    })
    await flush()
    abort.abort()
    await expect(done).resolves.toEqual({ ok: false, error: 'cancelled' })
  })
})

describe('progress wording', () => {
  it('rejects payloads that are not progress', () => {
    expect(parseProgress({ phase: 'done' })).toBeNull()
    expect(parseProgress({ job_id: 'j', phase: 'nope' })).toBeNull()
    expect(parseProgress(event('done'))?.phase).toBe('done')
  })

  it('reads sizes the way a person does', () => {
    expect(megabytes(42_400_000)).toBe('42 MB')
    expect(megabytes(1_250_000)).toBe('1.3 MB')
  })

  it('describes each phase in plain words', () => {
    expect(
      describeProgress(
        event('downloading', {
          version: '131.0',
          bytes_done: 42_000_000,
          bytes_total: 170_000_000,
        }),
      ),
    ).toBe('Downloading Chromium 131.0 · 42 MB of 170 MB')
    expect(describeProgress(event('resolving'))).toContain('latest stable')
    expect(describeProgress(event('verifying'))).toContain('starts')
  })

  it('has a fraction only while downloading with a known size', () => {
    expect(
      progressFraction(
        event('downloading', { bytes_done: 25, bytes_total: 100 }),
      ),
    ).toBe(0.25)
    expect(progressFraction(event('downloading'))).toBeUndefined()
    expect(progressFraction(event('extracting'))).toBeUndefined()
    expect(progressFraction(event('done'))).toBe(1)
  })
})

describe('manualInstall', () => {
  it('gives one command per system', () => {
    expect(manualInstall('linux64').command).toBe('sudo apt install chromium')
    expect(manualInstall('mac-arm64').command).toBe(
      'brew install --cask google-chrome',
    )
    expect(manualInstall('mac-x64').system).toBe('macOS')
    expect(manualInstall('win64').command).toBe('winget install Google.Chrome')
  })
})

describe('mentionsChromiumMissing', () => {
  it('finds the marker in a string or an error envelope', () => {
    expect(mentionsChromiumMissing('chromium_missing: no Chromium')).toBe(true)
    expect(
      mentionsChromiumMissing({
        error: { kind: 'handler', message: 'chromium_missing: install it' },
      }),
    ).toBe(true)
    expect(mentionsChromiumMissing({ error: { message: 'timeout' } })).toBe(
      false,
    )
    expect(mentionsChromiumMissing(undefined)).toBe(false)
  })
})

describe('checkChromiumMissing', () => {
  beforeEach(() => {
    __resetChromiumMissingForTests()
    presence.registered = new Set()
    shared.trigger.mockReset()
  })

  it('asks nothing when no browser worker is registered', async () => {
    await expect(checkChromiumMissing(100_000)).resolves.toBe(false)
    expect(shared.trigger).not.toHaveBeenCalled()
  })

  it('reads once, then reuses the answer for 30 s', async () => {
    presence.registered = new Set(['browser::chromium::status'])
    shared.trigger.mockResolvedValue(MISSING)
    await expect(checkChromiumMissing(100_000)).resolves.toBe(true)
    await expect(checkChromiumMissing(110_000)).resolves.toBe(true)
    expect(shared.trigger).toHaveBeenCalledTimes(1)
    shared.trigger.mockResolvedValue({ ...MISSING, found: true })
    await expect(checkChromiumMissing(140_000)).resolves.toBe(false)
    expect(shared.trigger).toHaveBeenCalledTimes(2)
  })
})

describe('browserLabel', () => {
  it('names a bare version Chromium and keeps a product name as is', () => {
    expect(browserLabel('155.0.8059.39')).toBe('Chromium 155.0.8059.39')
    expect(browserLabel('Google Chrome for Testing 155.0.8059.39')).toBe(
      'Google Chrome for Testing 155.0.8059.39',
    )
    expect(browserLabel(null)).toBe('Chromium')
  })
})
