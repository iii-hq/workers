import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { DEVICE_PROVIDERS, SUBSCRIPTION_PROVIDERS } from './catalog'
import {
  createLoginPoller,
  type DevicePollStatus,
  pollDeviceLogin,
  startDeviceLogin,
} from './device-login'

const trigger = vi.fn()

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ trigger }),
}))

vi.mock('./api', () => ({
  installedWorkerNames: async () =>
    new Set(['provider-openai-codex', 'provider-github-copilot']),
  addWorkersWithProgress: vi.fn(),
}))

beforeEach(() => {
  vi.useFakeTimers()
})

afterEach(() => {
  vi.useRealTimers()
})

function poller(statuses: DevicePollStatus[] = [], minIntervalMs = 0) {
  const poll = vi.fn(async () => statuses.shift() ?? 'pending')
  const results: { result: DevicePollStatus | Error; attempt: number }[] = []
  const onIdle = vi.fn()
  const handle = createLoginPoller({
    poll,
    onResult: (result, attempt) => results.push({ result, attempt }),
    onIdle,
    minIntervalMs,
  })
  return { poll, results, onIdle, handle }
}

describe('createLoginPoller', () => {
  it('polls 8 s after the code, then every 5 s, five tries in all', async () => {
    const { poll, results, onIdle, handle } = poller()
    handle.begin()
    await vi.advanceTimersByTimeAsync(7_999)
    expect(poll).toHaveBeenCalledTimes(0)
    await vi.advanceTimersByTimeAsync(1) // 8 s
    expect(poll).toHaveBeenCalledTimes(1)
    for (const total of [2, 3, 4, 5]) {
      await vi.advanceTimersByTimeAsync(5_000)
      expect(poll).toHaveBeenCalledTimes(total)
    }
    expect(results.map((entry) => entry.attempt)).toEqual([1, 2, 3, 4, 5])
    expect(onIdle).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(600_000)
    expect(poll).toHaveBeenCalledTimes(5)
  })

  it('starts a new round 5 s after a return to the tab or Retry', async () => {
    const { poll, results, onIdle, handle } = poller()
    handle.begin()
    await vi.advanceTimersByTimeAsync(200_000) // first round used up
    expect(poll).toHaveBeenCalledTimes(5)
    handle.restart()
    await vi.advanceTimersByTimeAsync(4_999)
    expect(poll).toHaveBeenCalledTimes(5)
    await vi.advanceTimersByTimeAsync(1)
    expect(poll).toHaveBeenCalledTimes(6)
    await vi.advanceTimersByTimeAsync(200_000)
    expect(poll).toHaveBeenCalledTimes(10)
    // The counter starts again with the round.
    expect(results.slice(5).map((entry) => entry.attempt)).toEqual([
      1, 2, 3, 4, 5,
    ])
    expect(onIdle).toHaveBeenCalledTimes(2)
  })

  it('a restart mid-round resets the count', async () => {
    const { results, handle } = poller()
    handle.begin()
    await vi.advanceTimersByTimeAsync(13_000) // tries 1 and 2
    handle.restart()
    await vi.advanceTimersByTimeAsync(5_000)
    expect(results.map((entry) => entry.attempt)).toEqual([1, 2, 1])
  })

  it('stops on success, an expired code or a denial', async () => {
    for (const terminal of ['ok', 'expired', 'denied', 'failed'] as const) {
      const { poll, results, handle } = poller(['pending', terminal])
      handle.begin()
      await vi.advanceTimersByTimeAsync(13_000)
      expect(results.map((entry) => entry.result)).toEqual([
        'pending',
        terminal,
      ])
      handle.restart()
      await vi.advanceTimersByTimeAsync(200_000)
      expect(poll).toHaveBeenCalledTimes(2)
    }
  })

  it('counts an error as a try and keeps going', async () => {
    const results: (DevicePollStatus | Error)[] = []
    const poll = vi
      .fn<() => Promise<DevicePollStatus>>()
      .mockRejectedValueOnce(new Error('engine down'))
      .mockResolvedValue('pending')
    const handle = createLoginPoller({ poll, onResult: (r) => results.push(r) })
    handle.begin()
    await vi.advanceTimersByTimeAsync(13_000)
    expect(results[0]).toBeInstanceOf(Error)
    expect(results[1]).toBe('pending')
  })

  it("keeps GitHub's interval between polls and widens it on slow_down", async () => {
    const { poll, handle } = poller(['slow_down'], 5_000)
    handle.begin()
    await vi.advanceTimersByTimeAsync(8_000) // slow_down: gap now 10 s
    expect(poll).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(9_999) // 5 s is due, 10 s is the floor
    expect(poll).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(1)
    expect(poll).toHaveBeenCalledTimes(2)
  })

  it('does nothing after stop', async () => {
    const { poll, handle } = poller()
    handle.begin()
    handle.stop()
    handle.restart()
    await vi.advanceTimersByTimeAsync(200_000)
    expect(poll).toHaveBeenCalledTimes(0)
  })
})

const codex = SUBSCRIPTION_PROVIDERS.find(
  (provider) => provider.toolId === 'codex',
)?.deviceLogin
const copilot = DEVICE_PROVIDERS[0]

describe('device login', () => {
  beforeEach(() => {
    trigger.mockReset()
  })

  it('follows the login id Codex hands out, until its deadline', async () => {
    if (!codex) throw new Error('Codex signs in from the ADE')
    trigger.mockResolvedValue({
      login_id: 'login-1',
      user_code: 'ABCD-1234',
      verification_uri: 'https://auth.openai.com/codex/device',
      expires_at: Math.floor(Date.now() / 1_000) + 900,
      interval: 5,
    })
    const code = await startDeviceLogin(codex, () => undefined)
    expect(trigger).toHaveBeenCalledWith(
      'provider::openai-codex::login::start',
      {},
    )
    expect(code).toMatchObject({
      device_code: 'login-1',
      user_code: 'ABCD-1234',
      interval: 5,
    })
    expect(code.expires_in).toBeGreaterThan(890)
  })

  it('polls Codex by login id and stops on an error or a replaced sign-in', async () => {
    if (!codex) throw new Error('Codex signs in from the ADE')
    trigger
      .mockResolvedValueOnce({ status: 'pending' })
      .mockResolvedValueOnce({
        status: 'error',
        error: { code: 'device_login_disabled', message: 'disabled' },
      })
      .mockResolvedValueOnce({ status: 'canceled' })
      .mockResolvedValueOnce({ status: 'ok' })
    expect(await pollDeviceLogin(codex, 'login-1')).toBe('pending')
    expect(trigger).toHaveBeenLastCalledWith(
      'provider::openai-codex::login::poll',
      { login_id: 'login-1' },
    )
    expect(await pollDeviceLogin(codex, 'login-1')).toBe('failed')
    expect(await pollDeviceLogin(codex, 'login-1')).toBe('failed')
    expect(await pollDeviceLogin(codex, 'login-1')).toBe('ok')
  })

  it('still polls GitHub Copilot by device code', async () => {
    trigger.mockResolvedValue({ status: 'slow_down' })
    expect(await pollDeviceLogin(copilot, 'device-1')).toBe('slow_down')
    expect(trigger).toHaveBeenCalledWith(
      'provider::github-copilot::login::poll',
      { device_code: 'device-1' },
    )
  })
})
