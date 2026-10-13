/**
 * Chromium for the browser worker: is there one, and getting one.
 *
 * The harness template ships the `browser` worker so agents can open the
 * pages they build and check them. On a machine with no Chrome or Chromium
 * every `browser::sessions::start` fails, so setup looks for one and offers to
 * download it (`browser::chromium::install`, a Chrome for Testing build kept
 * in the worker's cache directory).
 *
 * Progress arrives as `browser::chromium-install-progress` events — no
 * polling. A follower subscribes before it starts the download, and reads
 * `browser::chromium::status` exactly once if the events go quiet for a while
 * (a missed terminal event, a worker that restarted mid-download).
 *
 * A browser worker older than these functions (0.2.25) only has
 * `browser::doctor`: setup can tell Chromium is missing, but can offer only
 * the manual install.
 */

import { useSyncExternalStore } from 'react'
import { subscribeEngineTrigger } from '@/lib/engine-trigger'
import { functionRegistered } from '@/lib/function-presence'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import { isMissingFunction } from '@/lib/secrets'

export const BROWSER_WORKER = 'browser'
export const CHROMIUM_STATUS_FN = 'browser::chromium::status'
export const CHROMIUM_INSTALL_FN = 'browser::chromium::install'
export const BROWSER_DOCTOR_FN = 'browser::doctor'
export const CHROMIUM_PROGRESS_TRIGGER = 'browser::chromium-install-progress'
/** The marker the browser worker puts in every "no Chromium" error. */
export const CHROMIUM_MISSING_MARKER = 'chromium_missing'
/** How long the events may go quiet before the follower reads status once. */
export const CHROMIUM_SILENCE_MS = 30_000
/** Shown before the worker says how big its download is. */
const DEFAULT_DOWNLOAD_MB = 200

export type ChromiumSource =
  | 'configured'
  | 'env'
  | 'system'
  | 'managed'
  | 'playwright'
  | 'puppeteer'

export type ChromiumPlatform =
  | 'linux64'
  | 'linux-arm64'
  | 'mac-arm64'
  | 'mac-x64'
  | 'win64'

export type ChromiumPhase =
  | 'resolving'
  | 'downloading'
  | 'extracting'
  | 'verifying'
  | 'done'
  | 'failed'

/** One `browser::chromium-install-progress` event. */
export interface ChromiumInstallProgress {
  job_id: string
  phase: ChromiumPhase
  version?: string
  bytes_done: number
  bytes_total?: number
  path?: string
  error?: string
  hint?: string
  timestamp: number
}

/** `browser::chromium::status`, as the worker answers it. */
export interface ChromiumStatusResponse {
  found: boolean
  path?: string
  version?: string
  source?: ChromiumSource
  engine?: string
  searched?: string[]
  install?: {
    supported: boolean
    platform?: ChromiumPlatform
    dir?: string
    approx_download_mb?: number
    reason?: string
  }
  job?: ChromiumInstallProgress
}

/** `browser::doctor`, the fields the fallback reads. */
export interface BrowserDoctorResponse {
  engine?: string
  chromium_path?: string
  chromium_version?: string
}

/** What setup knows about Chromium where the browser worker runs. */
export interface ChromiumState {
  found: boolean
  path?: string
  version?: string
  source?: ChromiumSource
  /** `chromium` or `lightpanda`; Lightpanda needs no Chromium. */
  engine: string
  /** The worker can download Chromium itself (a supported platform). */
  canInstall: boolean
  /** Why it cannot, when it cannot. */
  installReason?: string
  /** The worker's platform, when it said (the ADE may run elsewhere). */
  platform?: ChromiumPlatform
  /** Where a download is kept. */
  installDir?: string
  approxDownloadMb: number
  /** Only `browser::doctor` answered: an older browser worker. */
  legacy: boolean
  /** The worker's latest download, running or finished. */
  job?: ChromiumInstallProgress
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
}

function asString(value: unknown): string | undefined {
  return typeof value === 'string' && value ? value : undefined
}

const PHASES: readonly ChromiumPhase[] = [
  'resolving',
  'downloading',
  'extracting',
  'verifying',
  'done',
  'failed',
]

/** A progress payload, or `null` when it is not one. */
export function parseProgress(value: unknown): ChromiumInstallProgress | null {
  const row = asRecord(value)
  const jobId = asString(row?.job_id)
  const phase = row?.phase
  if (!row || !jobId || !PHASES.includes(phase as ChromiumPhase)) return null
  return {
    job_id: jobId,
    phase: phase as ChromiumPhase,
    version: asString(row.version),
    bytes_done: typeof row.bytes_done === 'number' ? row.bytes_done : 0,
    bytes_total:
      typeof row.bytes_total === 'number' && row.bytes_total > 0
        ? row.bytes_total
        : undefined,
    path: asString(row.path),
    error: asString(row.error),
    hint: asString(row.hint),
    timestamp: typeof row.timestamp === 'number' ? row.timestamp : 0,
  }
}

export function stateFromStatus(status: ChromiumStatusResponse): ChromiumState {
  const install = status.install
  return {
    found: status.found === true,
    path: asString(status.path),
    version: asString(status.version),
    source: status.source,
    engine: asString(status.engine) ?? 'chromium',
    canInstall: install?.supported === true,
    installReason: asString(install?.reason),
    platform: install?.platform,
    installDir: asString(install?.dir),
    approxDownloadMb:
      typeof install?.approx_download_mb === 'number' &&
      install.approx_download_mb > 0
        ? install.approx_download_mb
        : DEFAULT_DOWNLOAD_MB,
    legacy: false,
    job: parseProgress(status.job) ?? undefined,
  }
}

export function stateFromDoctor(doctor: BrowserDoctorResponse): ChromiumState {
  const engine = asString(doctor.engine) ?? 'chromium'
  const path = asString(doctor.chromium_path)
  return {
    found: path !== undefined,
    path,
    version: asString(doctor.chromium_version),
    engine,
    canInstall: false,
    installReason:
      'This version of the browser worker cannot download Chromium by itself. Update it, or install Chromium yourself.',
    approxDownloadMb: DEFAULT_DOWNLOAD_MB,
    legacy: true,
  }
}

/** The browser worker drives Chromium and none was found. */
export function chromiumMissing(state: ChromiumState | null): boolean {
  return state !== null && state.engine === 'chromium' && !state.found
}

type TriggerOnly = Pick<IiiClient, 'trigger'>

/**
 * Ask the browser worker. `null` when no browser worker answers either
 * function; other failures reject.
 */
export async function readChromiumState(
  client?: TriggerOnly,
): Promise<ChromiumState | null> {
  const iii = client ?? (await getIiiClient())
  try {
    const status = await iii.trigger<ChromiumStatusResponse>(
      CHROMIUM_STATUS_FN,
      {},
      { timeoutMs: 15_000 },
    )
    return stateFromStatus(status ?? { found: false })
  } catch (error) {
    if (!isMissingFunction(error)) throw error
  }
  try {
    const doctor = await iii.trigger<BrowserDoctorResponse>(
      BROWSER_DOCTOR_FN,
      {},
      { timeoutMs: 15_000 },
    )
    return stateFromDoctor(doctor ?? {})
  } catch (error) {
    if (isMissingFunction(error)) return null
    throw error
  }
}

export type ChromiumInstallOutcome =
  | { ok: true; version?: string; path?: string }
  | { ok: false; error: string; hint?: string }

interface InstallResponse {
  job_id?: string
  status?: 'started' | 'running' | 'already-installed'
  path?: string
  version?: string
}

export interface FollowInstallOptions {
  /** Every progress event of this download, in order. */
  onProgress: (progress: ChromiumInstallProgress) => void
  /** Download again even when a Chromium is already found. */
  force?: boolean
  /** Stop following (the wizard closed); resolves as cancelled. */
  signal?: AbortSignal
  silenceMs?: number
  /** Test seams. */
  client?: TriggerOnly
  subscribe?: typeof subscribeEngineTrigger
}

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message.replace(/^Error:\s*/i, '')
  const row = asRecord(error)
  const message = asString(row?.message)
  if (message) return message.trim()
  return String(error)
}

/** `chromium_install_unsupported: …` → the sentence after the marker. */
function withoutMarker(message: string): string {
  return message.replace(/^[a-z_]+:\s*/, '')
}

/**
 * Start a download and follow it to its end. Subscribes to the progress
 * trigger first, then calls `browser::chromium::install`, and resolves on
 * the job's `done` or `failed` event. If no event arrives for `silenceMs`,
 * it reads `browser::chromium::status` once; the silence timer re-arms only
 * when a new event arrives, never after that read.
 */
export function followChromiumInstall(
  options: FollowInstallOptions,
): Promise<ChromiumInstallOutcome> {
  const {
    onProgress,
    force,
    signal,
    silenceMs = CHROMIUM_SILENCE_MS,
    subscribe = subscribeEngineTrigger,
  } = options
  return new Promise<ChromiumInstallOutcome>((resolve) => {
    let settled = false
    let jobId: string | null = null
    let silence: ReturnType<typeof setTimeout> | undefined
    let unsubscribe: (() => void) | undefined
    /** Events of this job that reached us; none means they cannot. */
    let seen = 0
    // Events that land before `install` answered with the job id.
    const early: ChromiumInstallProgress[] = []
    let client: TriggerOnly | undefined = options.client

    const finish = (outcome: ChromiumInstallOutcome) => {
      if (settled) return
      settled = true
      if (silence !== undefined) clearTimeout(silence)
      unsubscribe?.()
      signal?.removeEventListener('abort', onAbort)
      resolve(outcome)
    }
    const onAbort = () => finish({ ok: false, error: 'cancelled' })

    const outcomeOf = (
      progress: ChromiumInstallProgress,
    ): ChromiumInstallOutcome | null => {
      if (progress.phase === 'done') {
        return { ok: true, version: progress.version, path: progress.path }
      }
      if (progress.phase === 'failed') {
        return {
          ok: false,
          error: progress.error ?? 'The download failed.',
          hint: progress.hint,
        }
      }
      return null
    }

    const checkOnce = async () => {
      if (settled || !client) return
      let status: ChromiumState | null
      try {
        status = await readChromiumState(client)
      } catch {
        // The worker did not answer; keep waiting for its events.
        return
      }
      if (settled) return
      const job = status?.job
      if (job && job.job_id === jobId) {
        onProgress(job)
        const outcome = outcomeOf(job)
        if (outcome) finish(outcome)
        else if (seen === 0) {
          // Still running, but not one event reached this page: there is
          // nothing to wait on, and asking again would be polling.
          finish({
            ok: false,
            error:
              'Setup cannot follow this download live. It keeps going in the background.',
            hint: 'Use Check again in a minute or two.',
          })
        }
        return
      }
      // This process knows no such job: the worker restarted. A Chromium it
      // finished before going away still counts.
      if (status?.found) {
        finish({ ok: true, version: status.version, path: status.path })
      } else {
        finish({
          ok: false,
          error:
            'The download stopped: the browser worker restarted or lost track of it.',
          hint: 'Try again.',
        })
      }
    }

    const armSilence = () => {
      if (silence !== undefined) clearTimeout(silence)
      silence = setTimeout(() => {
        silence = undefined
        void checkOnce()
      }, silenceMs)
    }

    const accept = (progress: ChromiumInstallProgress) => {
      if (settled || progress.job_id !== jobId) return
      seen += 1
      onProgress(progress)
      const outcome = outcomeOf(progress)
      if (outcome) {
        finish(outcome)
        return
      }
      armSilence()
    }

    if (signal?.aborted) {
      onAbort()
      return
    }
    signal?.addEventListener('abort', onAbort)

    void (async () => {
      try {
        client ??= await getIiiClient()
      } catch (error) {
        finish({ ok: false, error: errorText(error) })
        return
      }
      try {
        const off = await subscribe(
          CHROMIUM_PROGRESS_TRIGGER,
          {},
          (payload) => {
            const progress = parseProgress(payload)
            if (!progress) return
            if (jobId === null) early.push(progress)
            else accept(progress)
          },
          { handler: 'iii::console::onboarding::chromium_progress' },
        )
        if (settled) {
          off()
          return
        }
        unsubscribe = off
      } catch {
        // No live events: the silence read below still ends the wait.
      }
      let started: InstallResponse | null
      try {
        started = await client.trigger<InstallResponse>(
          CHROMIUM_INSTALL_FN,
          force ? { force: true } : {},
          { timeoutMs: 30_000 },
        )
      } catch (error) {
        finish({ ok: false, error: withoutMarker(errorText(error)) })
        return
      }
      if (settled) return
      if (started?.status === 'already-installed' || !started?.job_id) {
        finish({ ok: true, version: started?.version, path: started?.path })
        return
      }
      jobId = started.job_id
      armSilence()
      for (const progress of early.splice(0)) accept(progress)
    })()
  })
}

/** `42 MB`, rounded the way a person reads a download. */
export function megabytes(bytes: number): string {
  const mb = bytes / 1_000_000
  return `${mb >= 10 ? Math.round(mb) : Math.round(mb * 10) / 10} MB`
}

/** 0..1 while the size is known; `undefined` otherwise. */
export function progressFraction(
  progress: ChromiumInstallProgress | null | undefined,
): number | undefined {
  if (!progress) return undefined
  if (progress.phase === 'done') return 1
  if (progress.phase !== 'downloading' || !progress.bytes_total) {
    return undefined
  }
  return Math.min(1, progress.bytes_done / progress.bytes_total)
}

/** The line under the progress bar, in plain words. */
export function describeProgress(progress: ChromiumInstallProgress): string {
  const version = progress.version ? `Chromium ${progress.version}` : 'Chromium'
  switch (progress.phase) {
    case 'resolving':
      return 'Finding the latest stable Chromium…'
    case 'downloading':
      return progress.bytes_total
        ? `Downloading ${version} · ${megabytes(progress.bytes_done)} of ${megabytes(progress.bytes_total)}`
        : `Downloading ${version} · ${megabytes(progress.bytes_done)}`
    case 'extracting':
      return `Unpacking ${version}…`
    case 'verifying':
      return `Checking that ${version} starts…`
    case 'done':
      return `${version} is ready`
    case 'failed':
      return progress.error ?? 'The download failed.'
  }
}

export interface ManualInstall {
  /** `Ubuntu or Debian`, `macOS`, `Windows`. */
  system: string
  command: string
  /** Another way, in a sentence. */
  alternative: string
}

function hostPlatform(): ChromiumPlatform | undefined {
  if (typeof navigator === 'undefined') return undefined
  const agent = navigator.userAgent
  if (/Mac|iPhone|iPad|iPod/.test(agent)) return 'mac-arm64'
  if (/Windows/.test(agent)) return 'win64'
  if (/Linux|X11/.test(agent)) return 'linux64'
  return undefined
}

/**
 * The one command that installs a Chromium the worker finds by itself, for
 * the worker's platform when it said, else for the browser's.
 */
export function manualInstall(platform?: ChromiumPlatform): ManualInstall {
  const target = platform ?? hostPlatform() ?? 'linux64'
  if (target === 'mac-arm64' || target === 'mac-x64') {
    return {
      system: 'macOS',
      command: 'brew install --cask google-chrome',
      alternative:
        'Or download Google Chrome from google.com/chrome and drag it to Applications.',
    }
  }
  if (target === 'win64') {
    return {
      system: 'Windows',
      command: 'winget install Google.Chrome',
      alternative: 'Or download Google Chrome from google.com/chrome.',
    }
  }
  return {
    system: 'Ubuntu or Debian',
    command: 'sudo apt install chromium',
    alternative:
      'On Fedora: sudo dnf install chromium. Or install Google Chrome from google.com/chrome.',
  }
}

/** A browser tool error that says Chromium is missing. */
export function mentionsChromiumMissing(value: unknown): boolean {
  if (value === null || value === undefined) return false
  if (typeof value === 'string') return value.includes(CHROMIUM_MISSING_MARKER)
  try {
    return JSON.stringify(value).includes(CHROMIUM_MISSING_MARKER)
  } catch {
    return false
  }
}

// ---------------------------------------------------------------------------
// "Is Chromium missing?" for surfaces outside the wizard (the command
// palette). Updated by the wizard's own reading, by a chat error that says
// so, and by `checkChromiumMissing` — a one-shot read the palette makes when
// it opens. Nothing here repeats on a timer.
// ---------------------------------------------------------------------------

let knownMissing = false
let lastCheck = 0
const missingListeners = new Set<() => void>()
/** A palette opened twice in a row does not ask the worker twice. */
const CHECK_REUSE_MS = 30_000

export function setChromiumMissing(missing: boolean): void {
  if (missing === knownMissing) return
  knownMissing = missing
  for (const listener of missingListeners) listener()
}

function subscribeMissing(listener: () => void): () => void {
  missingListeners.add(listener)
  return () => {
    missingListeners.delete(listener)
  }
}

function readMissing(): boolean {
  return knownMissing
}

/** Whether the last reading said the browser worker has no Chromium. */
export function useChromiumMissing(): boolean {
  return useSyncExternalStore(subscribeMissing, readMissing, readMissing)
}

/**
 * Read once whether the browser worker lacks Chromium, and remember it.
 * Asks nothing when no browser worker is registered (the cached function
 * catalog says so), and nothing again within 30 s of the last read.
 */
export async function checkChromiumMissing(
  now: number = Date.now(),
): Promise<boolean> {
  if (now - lastCheck < CHECK_REUSE_MS) return knownMissing
  lastCheck = now
  try {
    const [status, doctor] = await Promise.all([
      functionRegistered(CHROMIUM_STATUS_FN),
      functionRegistered(BROWSER_DOCTOR_FN),
    ])
    if (!status && !doctor) {
      setChromiumMissing(false)
      return false
    }
    const missing = chromiumMissing(await readChromiumState())
    setChromiumMissing(missing)
    return missing
  } catch {
    return knownMissing
  }
}

/** Test seam: forget what was read. */
export function __resetChromiumMissingForTests(): void {
  knownMissing = false
  lastCheck = 0
  missingListeners.clear()
}

/**
 * How to name an installed browser: a bare version gets the "Chromium"
 * prefix; a `--version` line that already names the product ("Google Chrome
 * for Testing 155.0.8059.39", "Chromium 140.0…") is shown as is.
 */
export function browserLabel(version: string | null | undefined): string {
  const v = version?.trim()
  if (!v) return 'Chromium'
  return /^[\d.]+$/.test(v) ? `Chromium ${v}` : v
}
