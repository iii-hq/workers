import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createLoginPoller, type DevicePollStatus } from './device-login'

beforeEach(() => {
  vi.useFakeTimers()
})

afterEach(() => {
  vi.useRealTimers()
})

function poller(statuses: DevicePollStatus[] = []) {
  const poll = vi.fn(async () => statuses.shift() ?? 'pending')
  const results: (DevicePollStatus | Error)[] = []
  const handle = createLoginPoller({
    poll,
    onResult: (result) => results.push(result),
  })
  return { poll, results, handle }
}

describe('createLoginPoller', () => {
  it('polls after 4, 8, 16 and 32 seconds, then waits', async () => {
    const { poll, handle } = poller()
    handle.begin()
    await vi.advanceTimersByTimeAsync(3_999)
    expect(poll).toHaveBeenCalledTimes(0)
    await vi.advanceTimersByTimeAsync(1) // 4 s
    expect(poll).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(8_000) // 12 s
    expect(poll).toHaveBeenCalledTimes(2)
    await vi.advanceTimersByTimeAsync(16_000) // 28 s
    expect(poll).toHaveBeenCalledTimes(3)
    await vi.advanceTimersByTimeAsync(32_000) // 60 s
    expect(poll).toHaveBeenCalledTimes(4)
    await vi.advanceTimersByTimeAsync(600_000)
    expect(poll).toHaveBeenCalledTimes(4)
  })

  it('polls at once when the tab is focused again, then restarts the schedule', async () => {
    const { poll, handle } = poller()
    handle.begin()
    await vi.advanceTimersByTimeAsync(200_000) // schedule used up
    expect(poll).toHaveBeenCalledTimes(4)
    handle.focus()
    await vi.advanceTimersByTimeAsync(0)
    expect(poll).toHaveBeenCalledTimes(5)
    await vi.advanceTimersByTimeAsync(4_000)
    expect(poll).toHaveBeenCalledTimes(6)
  })

  it('stops on success, an expired code or a denial', async () => {
    for (const terminal of ['ok', 'expired', 'denied'] as const) {
      const { poll, results, handle } = poller(['pending', terminal])
      handle.begin()
      await vi.advanceTimersByTimeAsync(12_000)
      expect(results).toEqual(['pending', terminal])
      handle.focus()
      await vi.advanceTimersByTimeAsync(200_000)
      expect(poll).toHaveBeenCalledTimes(2)
    }
  })

  it('reports an error and keeps the schedule', async () => {
    const results: (DevicePollStatus | Error)[] = []
    const poll = vi
      .fn<() => Promise<DevicePollStatus>>()
      .mockRejectedValueOnce(new Error('engine down'))
      .mockResolvedValue('pending')
    const handle = createLoginPoller({ poll, onResult: (r) => results.push(r) })
    handle.begin()
    await vi.advanceTimersByTimeAsync(12_000)
    expect(results[0]).toBeInstanceOf(Error)
    expect(results[1]).toBe('pending')
  })

  it('does nothing after stop', async () => {
    const { poll, handle } = poller()
    handle.begin()
    handle.stop()
    handle.focus()
    await vi.advanceTimersByTimeAsync(200_000)
    expect(poll).toHaveBeenCalledTimes(0)
  })
})
