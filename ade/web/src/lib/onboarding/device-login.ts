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
}

/** After the code is out, and again after each focus: 4, 8, 16, 32 s apart. */
export const POLL_DELAYS_MS: readonly number[] = [4_000, 8_000, 16_000, 32_000]

const TERMINAL: ReadonlySet<DevicePollStatus> = new Set([
  'ok',
  'expired',
  'denied',
])

export interface LoginPoller {
  /** The code is out: poll on the schedule. */
  begin(): void
  /** The tab is back in focus: poll now, then restart the schedule. */
  focus(): void
  stop(): void
}

/**
 * Polls on a backoff schedule, once at a time, and stops for good on a
 * terminal status (`ok`, `expired`, `denied`). An error is reported and the
 * schedule goes on.
 */
export function createLoginPoller({
  poll,
  onResult,
  delays = POLL_DELAYS_MS,
}: {
  poll: () => Promise<DevicePollStatus>
  onResult: (result: DevicePollStatus | Error) => void
  delays?: readonly number[]
}): LoginPoller {
  let timer: ReturnType<typeof setTimeout> | undefined
  let step = 0
  let stopped = false
  let inFlight = false

  const schedule = () => {
    clearTimeout(timer)
    if (stopped || step >= delays.length) return
    timer = setTimeout(() => void check(), delays[step++])
  }

  const check = async () => {
    if (stopped || inFlight) return
    inFlight = true
    clearTimeout(timer)
    let result: DevicePollStatus | Error
    try {
      result = await poll()
    } catch (error) {
      result = error instanceof Error ? error : new Error(String(error))
    }
    inFlight = false
    if (stopped) return
    onResult(result)
    if (typeof result === 'string' && TERMINAL.has(result)) {
      stopped = true
      return
    }
    schedule()
  }

  return {
    begin() {
      step = 0
      schedule()
    },
    focus() {
      if (stopped) return
      step = 0
      void check()
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
