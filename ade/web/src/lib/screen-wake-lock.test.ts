import { afterEach, describe, expect, it, vi } from 'vitest'
import { createScreenWakeLock, withScreenWakeLock } from './screen-wake-lock'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((yes, no) => {
    resolve = yes
    reject = no
  })
  return { promise, resolve, reject }
}

function sentinel() {
  const lock = Object.assign(new EventTarget(), {
    released: false,
    release: vi.fn(async () => {
      lock.released = true
      lock.dispatchEvent(new Event('release'))
    }),
  })
  return lock
}

function browser() {
  const document = Object.assign(new EventTarget(), {
    visibilityState: 'visible',
  })
  const request = vi.fn<() => Promise<WakeLockSentinel>>()
  vi.stubGlobal('document', document)
  vi.stubGlobal('navigator', { wakeLock: { request } })
  return {
    document,
    request,
    visibility(state: string) {
      document.visibilityState = state
      document.dispatchEvent(new Event('visibilitychange'))
    },
  }
}

async function settle() {
  for (let i = 0; i < 8; i++) await Promise.resolve()
}

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('screen wake lock leases', () => {
  it('does nothing without browser support (including SSR)', () => {
    vi.stubGlobal('document', undefined)
    const acquire = createScreenWakeLock()
    expect(() => acquire()()).not.toThrow()
    vi.stubGlobal('document', new EventTarget())
    vi.stubGlobal('navigator', {})
    expect(() => acquire()()).not.toThrow()
  })

  it('shares one request across concurrent chats/voice and releases only the last lease', async () => {
    const { request, document } = browser()
    const lock = sentinel()
    request.mockResolvedValue(lock as unknown as WakeLockSentinel)
    const acquire = createScreenWakeLock()
    const offChat = acquire()
    const offVoice = acquire()
    await settle()
    expect(request).toHaveBeenCalledExactlyOnceWith('screen')
    offChat()
    offChat()
    expect(lock.release).not.toHaveBeenCalled()
    offVoice()
    expect(lock.release).toHaveBeenCalledTimes(1)
    document.dispatchEvent(new Event('pointerdown'))
    expect(request).toHaveBeenCalledTimes(1)
  })

  it('releases while hidden and reacquires on return only while work remains', async () => {
    const { request, visibility } = browser()
    const first = sentinel()
    const second = sentinel()
    request
      .mockResolvedValueOnce(first as unknown as WakeLockSentinel)
      .mockResolvedValueOnce(second as unknown as WakeLockSentinel)
    const off = createScreenWakeLock()()
    await settle()
    visibility('hidden')
    expect(first.release).toHaveBeenCalledTimes(1)
    visibility('visible')
    await settle()
    expect(request).toHaveBeenCalledTimes(2)
    off()
    visibility('hidden')
    visibility('visible')
    expect(second.release).toHaveBeenCalledTimes(1)
    expect(request).toHaveBeenCalledTimes(2)
  })

  it('does not acquire for work that starts and ends in a hidden document', () => {
    const { request, visibility } = browser()
    visibility('hidden')
    const off = createScreenWakeLock()()
    expect(request).not.toHaveBeenCalled()
    off()
    visibility('visible')
    expect(request).not.toHaveBeenCalled()
  })

  it('releases a late acquisition after the activity ends', async () => {
    const { request } = browser()
    const pending = deferred<WakeLockSentinel>()
    const lock = sentinel()
    request.mockReturnValue(pending.promise)
    const off = createScreenWakeLock()()
    off()
    pending.resolve(lock as unknown as WakeLockSentinel)
    await settle()
    expect(lock.release).toHaveBeenCalledTimes(1)
    expect(request).toHaveBeenCalledTimes(1)
  })

  it('serializes stop/start races, including Strict Mode effect remounts', async () => {
    const { request } = browser()
    const pending = deferred<WakeLockSentinel>()
    const stale = sentinel()
    const current = sentinel()
    request
      .mockReturnValueOnce(pending.promise)
      .mockResolvedValueOnce(current as unknown as WakeLockSentinel)
    const acquire = createScreenWakeLock()
    acquire()()
    const off = acquire()
    expect(request).toHaveBeenCalledTimes(1)
    pending.resolve(stale as unknown as WakeLockSentinel)
    await settle()
    expect(stale.release).toHaveBeenCalledTimes(1)
    expect(request).toHaveBeenCalledTimes(2)
    expect(current.release).not.toHaveBeenCalled()
    off()
    expect(current.release).toHaveBeenCalledTimes(1)
  })

  it('discards a request spanning a hide/show race and gets a fresh lock', async () => {
    const { request, visibility } = browser()
    const pending = deferred<WakeLockSentinel>()
    const stale = sentinel()
    const current = sentinel()
    request
      .mockReturnValueOnce(pending.promise)
      .mockResolvedValueOnce(current as unknown as WakeLockSentinel)
    const off = createScreenWakeLock()()
    visibility('hidden')
    visibility('visible')
    pending.resolve(stale as unknown as WakeLockSentinel)
    await settle()
    expect(stale.release).toHaveBeenCalledTimes(1)
    expect(request).toHaveBeenCalledTimes(2)
    off()
  })

  it('handles rejection without retry loops and retries on a user gesture', async () => {
    const { request, document } = browser()
    const lock = sentinel()
    request
      .mockRejectedValueOnce(new DOMException('denied', 'NotAllowedError'))
      .mockResolvedValueOnce(lock as unknown as WakeLockSentinel)
    const off = createScreenWakeLock()()
    await settle()
    expect(request).toHaveBeenCalledTimes(1)
    document.dispatchEvent(new Event('pointerdown'))
    await settle()
    expect(request).toHaveBeenCalledTimes(2)
    off()
  })

  it('recovers from OS revocation on interaction without fighting the OS', async () => {
    const { request, document } = browser()
    const first = sentinel()
    const next = sentinel()
    request
      .mockResolvedValueOnce(first as unknown as WakeLockSentinel)
      .mockResolvedValueOnce(next as unknown as WakeLockSentinel)
    const off = createScreenWakeLock()()
    await settle()
    await first.release()
    await settle()
    expect(request).toHaveBeenCalledTimes(1)
    document.dispatchEvent(new Event('keydown'))
    await settle()
    expect(request).toHaveBeenCalledTimes(2)
    off()
  })

  it('ignores release failures', async () => {
    const { request } = browser()
    const lock = sentinel()
    lock.release.mockRejectedValue(new Error('already revoked'))
    request.mockResolvedValue(lock as unknown as WakeLockSentinel)
    const off = createScreenWakeLock()()
    await settle()
    off()
    await settle()
    expect(lock.release).toHaveBeenCalledTimes(1)
  })

  it.each(['success', 'failure'] as const)(
    'releases local work on %s',
    async (outcome) => {
      const { request } = browser()
      const lock = sentinel()
      request.mockResolvedValue(lock as unknown as WakeLockSentinel)
      const task = deferred<string>()
      const run = withScreenWakeLock(async (input: string) => {
        expect(input).toBe('attachments')
        return task.promise
      })
      const result = run('attachments')
      await settle()
      expect(lock.release).not.toHaveBeenCalled()
      if (outcome === 'failure') {
        task.reject(new Error('upload failed'))
        await expect(result).rejects.toThrow('upload failed')
      } else {
        task.resolve('sent')
        await expect(result).resolves.toBe('sent')
      }
      expect(lock.release).toHaveBeenCalledTimes(1)
    },
  )
})
