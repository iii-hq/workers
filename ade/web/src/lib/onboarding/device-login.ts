/**
 * Sign-in for a provider that uses a device flow (GitHub Copilot): the
 * worker hands out a code, the person enters it on the provider's page, and
 * the ADE polls the worker until the sign-in lands. The device code stays
 * in memory here; only the user code is shown.
 */
import { getIiiClient } from '@/lib/iii-client'
import type { StepProgress } from './api'
import { addWorkersWithProgress, installedWorkerNames } from './api'
import type { DeviceProvider } from './catalog'

export type DevicePollStatus =
  | 'ok'
  | 'pending'
  | 'slow_down'
  | 'expired'
  | 'denied'

export interface DeviceCode {
  user_code: string
  verification_uri: string
  device_code: string
  expires_in?: number
  /** Seconds GitHub wants between polls. */
  interval?: number
}

/** The first check comes this long after Authenticate. */
export const FIRST_POLL_MS = 8_000
/** Then, and in a round started by a return to the tab or Retry, this apart. */
export const POLL_EVERY_MS = 5_000
/** Checks per round. */
export const POLL_TRIES = 5

const TERMINAL: ReadonlySet<DevicePollStatus> = new Set([
  'ok',
  'expired',
  'denied',
])

export interface LoginPoller {
  /** Authenticate: a round whose first check is `FIRST_POLL_MS` away. */
  begin(): void
  /** Back on the tab, or Retry: a new round, first check `POLL_EVERY_MS` away. */
  restart(): void
  stop(): void
}

/**
 * Polls in rounds of `POLL_TRIES` checks, one at a time, and stops for good
 * on a terminal status (`ok`, `expired`, `denied`). `onResult` gets each
 * result with its try number in the round; an error counts as a try. After
 * the last try of a round, `onIdle`.
 */
export function createLoginPoller({
  poll,
  onResult,
  onIdle,
  minIntervalMs = 0,
}: {
  poll: () => Promise<DevicePollStatus>
  onResult: (result: DevicePollStatus | Error, attempt: number) => void
  /** The round ran out; nothing is checked until the next restart. */
  onIdle?: () => void
  /**
   * GitHub's `interval`: a poll sooner than this after the last one (or
   * after `begin`) gets `slow_down`, never the token, so a due poll waits
   * for it. Each `slow_down` adds 5 s, as GitHub does.
   */
  minIntervalMs?: number
}): LoginPoller {
  let timer: ReturnType<typeof setTimeout> | undefined
  let attempt = 0
  let round = 0
  let stopped = false
  let gap = minIntervalMs
  let last = Date.now()

  const schedule = (delay: number) => {
    clearTimeout(timer)
    if (stopped) return
    const mine = round
    timer = setTimeout(
      () => void check(mine),
      Math.max(delay, last + gap - Date.now()),
    )
  }

  const check = async (mine: number) => {
    if (stopped || mine !== round) return
    last = Date.now()
    let result: DevicePollStatus | Error
    try {
      result = await poll()
    } catch (error) {
      result = error instanceof Error ? error : new Error(String(error))
    }
    // A restart while this poll was out started a new round; it owns the
    // count and the schedule now.
    if (stopped || mine !== round) return
    if (result === 'slow_down') gap += 5_000
    attempt += 1
    onResult(result, attempt)
    if (typeof result === 'string' && TERMINAL.has(result)) {
      stopped = true
      return
    }
    if (attempt >= POLL_TRIES) {
      onIdle?.()
      return
    }
    schedule(POLL_EVERY_MS)
  }

  const startRound = (delay: number) => {
    if (stopped) return
    round += 1
    attempt = 0
    schedule(delay)
  }

  return {
    begin() {
      last = Date.now()
      startRound(FIRST_POLL_MS)
    },
    restart() {
      startRound(POLL_EVERY_MS)
    },
    stop() {
      stopped = true
      clearTimeout(timer)
    },
  }
}

/**
 * Add the provider's worker when it is not running yet, then ask it for a
 * device code.
 */
export async function startDeviceLogin(
  provider: DeviceProvider,
  report: (progress: StepProgress) => void,
): Promise<DeviceCode> {
  const installed = await installedWorkerNames()
  if (!installed.has(provider.worker)) {
    await addWorkersWithProgress([provider.worker], report)
  }
  report({ note: 'asking GitHub for a code' })
  const client = await getIiiClient()
  const code = await client.trigger<Partial<DeviceCode>>(
    provider.loginStart,
    {},
  )
  if (!code?.user_code || !code.verification_uri || !code.device_code) {
    throw new Error(`${provider.title} did not return a sign-in code.`)
  }
  return code as DeviceCode
}

export async function pollDeviceLogin(
  provider: DeviceProvider,
  deviceCode: string,
): Promise<DevicePollStatus> {
  const client = await getIiiClient()
  const reply = await client.trigger<{ status?: DevicePollStatus }>(
    provider.loginPoll,
    { device_code: deviceCode },
  )
  return reply?.status ?? 'pending'
}
