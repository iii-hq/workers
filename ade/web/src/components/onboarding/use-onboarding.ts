import { useCallback, useEffect, useRef, useState } from 'react'
import {
  detectKeys,
  installedWorkerNames,
  readableError,
  readConsoleConfig,
  readJudgeProvider,
  readProviderStates,
  runStep,
  scanMachine,
} from '@/lib/onboarding/api'
import {
  JUDGE_HUB_WORKER,
  JUDGE_OPTIONS,
  KEY_PROVIDERS,
  SECRETS_WORKER,
  SUBSCRIPTION_PROVIDERS,
} from '@/lib/onboarding/catalog'
import {
  BROWSER_WORKER,
  type ChromiumInstallOutcome,
  type ChromiumInstallProgress,
  type ChromiumState,
  chromiumMissing,
  describeProgress,
  followChromiumInstall,
  progressFraction,
  readChromiumState,
  setChromiumMissing,
} from '@/lib/onboarding/chromium'
import {
  describeStep,
  type KeyDetection,
  type PlanStep,
  type ProviderState,
  servesUsableModels,
  stepDetail,
  type ToolScan,
} from '@/lib/onboarding/plan'
import { envFileName, getSecretsStatus } from '@/lib/secrets'

/** Which part of setup an action belongs to; each step shows its own. */
export type ActivityGroup = 'models' | 'browser' | 'judge'

export interface ActivityEntry {
  id: number
  group: ActivityGroup
  /** One plain sentence (`describeStep`). */
  title: string
  /** The engine operation behind it (`stepDetail`), shown in mono. */
  detail?: string
  /** `skipped`: the person moved on from a slow step ("Skip for now"). */
  status: 'running' | 'done' | 'failed' | 'skipped'
  /** Live progress line while running; the outcome once finished. */
  note?: string
  progress?: number
  /** When the step started (ms since epoch), for "taking longer". */
  startedAt?: number
  /** When `note` last changed, for its seconds counter. */
  noteAt?: number
  /** Workers this entry added, for the summary. */
  workers?: string[]
}

/** Looks again after a run that left a provider it did not touch serving
 * nothing, this far apart. */
const SETTLE_LOOKS = 4
const SETTLE_MS = 2_000

/** Providers serving usable models in `snapshot`, by id. */
function servingProviders(snapshot: MachineSnapshot): Set<string> {
  return new Set(
    (snapshot.providers ?? [])
      .filter(servesUsableModels)
      .map((provider) => provider.id),
  )
}

/** A step running longer than this offers "Skip for now". */
export const SLOW_STEP_MS = 30_000

/** The outcome error of a Chromium download the person skipped. */
export const SKIPPED = 'skipped'

export interface MachineSnapshot {
  tools: ToolScan[] | null
  toolsError: string | null
  providers: ProviderState[] | null
  providersError: string | null
  /**
   * `null` when the secrets worker is not running to look (it comes with
   * llm-router, so only a project set up before that lacks it).
   */
  detections: KeyDetection[] | null
  /** The secrets worker's env file, by name (`.env` unless configured). */
  envFile: string
  installed: ReadonlySet<string>
  /** The judge hub's `provider`, `null` when Judge is not set up or unread. */
  judgeProvider: string | null
  consoleConfig: Record<string, unknown> | null
  /**
   * Chromium where the browser worker runs; `null` when no browser worker
   * is installed, or it was not read yet.
   */
  browser: ChromiumState | null
  browserError: string | null
}

const EMPTY: MachineSnapshot = {
  tools: null,
  toolsError: null,
  providers: null,
  providersError: null,
  detections: null,
  envFile: envFileName(null),
  installed: new Set(),
  judgeProvider: null,
  consoleConfig: null,
  browser: null,
  browserError: null,
}

const PROVIDER_WORKERS = new Map(
  [...SUBSCRIPTION_PROVIDERS, ...KEY_PROVIDERS].map((provider) => [
    provider.providerId,
    provider.worker,
  ]),
)

/**
 * The router learns that a provider went away only when a dispatch fails or
 * it restarts, so a worker removed a minute ago still reads as available
 * with its whole catalog. The engine's worker list is the truth: a known
 * provider whose worker is not connected is not connected.
 */
export function withWorkerPresence(
  providers: readonly ProviderState[],
  installed: ReadonlySet<string> | null,
): ProviderState[] {
  if (!installed || installed.size === 0) return [...providers]
  return providers.map((provider) => {
    const worker = PROVIDER_WORKERS.get(provider.id)
    return worker && !installed.has(worker)
      ? { ...provider, available: false, modelCount: 0 }
      : provider
  })
}

/**
 * Chat models the router serves right now from providers whose worker is
 * connected and whose models are usable (`servesUsableModels`) — the
 * wizard's own reading. An unconfigured key provider can still list models,
 * but the picker cannot use them, so they do not count. Throws when the
 * router or the engine cannot answer.
 */
export async function connectedModelCount(): Promise<number> {
  const [providers, installed] = await Promise.all([
    readProviderStates(),
    installedWorkerNames(),
  ])
  return withWorkerPresence(providers, installed)
    .filter(servesUsableModels)
    .reduce((sum, provider) => sum + provider.modelCount, 0)
}

/** Every env var the wizard can reuse: provider keys and the hosted judge's. */
export const DETECTED_KEY_NAMES = [
  ...KEY_PROVIDERS.map((provider) => provider.envVar),
  ...JUDGE_OPTIONS.flatMap((option) => (option.envVar ? [option.envVar] : [])),
]

async function settle<T>(
  promise: Promise<T>,
): Promise<{ value: T | null; error: string | null }> {
  try {
    return { value: await promise, error: null }
  } catch (error) {
    return { value: null, error: readableError(error) }
  }
}

/**
 * The wizard's live view of the machine plus the log of everything it did.
 * `run` executes a plan one step at a time and records each step as an
 * activity entry the user watches — the transparency the wizard promises.
 */
export function useOnboarding(
  enabled: boolean,
  /** Called after a plan ran, so the rest of the ADE picks up new models. */
  onSettled?: () => void,
) {
  const [snapshot, setSnapshot] = useState<MachineSnapshot>(EMPTY)
  /** The latest snapshot, for a run that started on an older render. */
  const snapshotRef = useRef<MachineSnapshot>(EMPTY)
  const [scanning, setScanning] = useState(false)
  const [activity, setActivity] = useState<ActivityEntry[]>([])
  const [running, setRunning] = useState<ActivityGroup | null>(null)
  const nextId = useRef(1)
  const onSettledRef = useRef(onSettled)
  onSettledRef.current = onSettled
  const cancel = useRef({ cancelled: false })
  /** Ends the step running now, if any, and moves on to the next one. */
  const skipCurrent = useRef<(() => void) | null>(null)

  const refresh = useCallback(async () => {
    setScanning(true)
    const [tools, providers, installed, consoleConfig] = await Promise.all([
      settle(scanMachine()),
      settle(readProviderStates()),
      settle(installedWorkerNames()),
      readConsoleConfig(),
    ])
    const names = installed.value ?? new Set<string>()
    const [detections, secrets, browser, judgeProvider] = await Promise.all([
      names.has(SECRETS_WORKER)
        ? settle(detectKeys(DETECTED_KEY_NAMES))
        : { value: null, error: null },
      names.has(SECRETS_WORKER)
        ? settle(getSecretsStatus())
        : { value: undefined, error: null },
      names.has(BROWSER_WORKER)
        ? settle(readChromiumState())
        : { value: null, error: null },
      names.has(JUDGE_HUB_WORKER) ? readJudgeProvider() : null,
    ])
    if (browser.value) setChromiumMissing(chromiumMissing(browser.value))
    const next: MachineSnapshot = {
      tools: tools.value ?? [],
      toolsError: tools.error,
      providers: providers.value
        ? withWorkerPresence(providers.value, installed.value)
        : null,
      providersError: providers.error,
      detections: detections.value,
      envFile: envFileName(secrets.value?.env_file),
      installed: names,
      judgeProvider,
      consoleConfig,
      browser: browser.value,
      browserError: browser.error,
    }
    snapshotRef.current = next
    setSnapshot(next)
    setScanning(false)
    return next
  }, [])

  /** Look at the machine again ("Scan again"). */
  const rescan = useCallback(async () => {
    await refresh()
  }, [refresh])

  /** Read Chromium again (after installing it by hand), nothing else. */
  const checkChromium = useCallback(async () => {
    const browser = await settle(readChromiumState())
    if (browser.value) setChromiumMissing(chromiumMissing(browser.value))
    setSnapshot((current) => ({
      ...current,
      browser: browser.value ?? current.browser,
      browserError: browser.error,
    }))
    return browser.value
  }, [])

  useEffect(() => {
    if (!enabled) return
    void refresh()
  }, [enabled, refresh])

  // A run keeps going when setup is closed — what it asked of the engine
  // finishes either way, and reopening shows where it got to — and stops
  // following only when the wizard itself goes away.
  useEffect(() => {
    const token = { cancelled: false }
    cancel.current = token
    return () => {
      token.cancelled = true
      skipCurrent.current?.()
    }
  }, [])

  const patch = useCallback((id: number, next: Partial<ActivityEntry>) => {
    setActivity((current) =>
      current.map((entry) =>
        entry.id !== id
          ? entry
          : {
              ...entry,
              ...next,
              // A new progress line restarts its seconds counter.
              ...(next.note !== undefined && next.note !== entry.note
                ? { noteAt: Date.now() }
                : {}),
            },
      ),
    )
  }, [])

  /** "Skip for now" on a slow step: end it and carry on with the next. */
  const skipStep = useCallback(() => skipCurrent.current?.(), [])

  /** Run `steps` in order; stops at the first failure. Resolves to success. */
  const run = useCallback(
    async (group: ActivityGroup, steps: readonly PlanStep[]) => {
      setRunning(group)
      const servingBefore = servingProviders(snapshotRef.current)
      const consoleConfig =
        snapshot.consoleConfig ?? (await readConsoleConfig())
      let ok = true
      for (const step of steps) {
        const id = nextId.current++
        setActivity((current) => [
          ...current,
          {
            id,
            group,
            title: describeStep(step),
            detail: stepDetail(step),
            status: 'running',
            startedAt: Date.now(),
            workers: step.kind === 'add-workers' ? step.workers : undefined,
          },
        ])
        // Each step waits on its own token, so skipping one stops only its
        // waits; the wizard going away ends it the same way.
        const token = { cancelled: cancel.current.cancelled }
        const skipped = new Promise<'skipped'>((resolve) => {
          skipCurrent.current = () => {
            token.cancelled = true
            resolve('skipped')
          }
        })
        try {
          const result = await Promise.race([
            runStep(step, {
              consoleConfig,
              signal: token,
              report: ({ note, progress }) => patch(id, { note, progress }),
            }),
            skipped,
          ])
          if (result === 'skipped') {
            // The skipped step may still finish on the engine; setup no
            // longer waits for it.
            patch(id, {
              status: 'skipped',
              note: 'skipped — setup moved on',
              progress: undefined,
            })
            continue
          }
          patch(id, { status: 'done', note: result.note, progress: undefined })
        } catch (error) {
          patch(id, {
            status: 'failed',
            note: readableError(error),
            progress: undefined,
          })
          ok = false
          break
        } finally {
          skipCurrent.current = null
        }
      }
      // A provider that served models before this run, and that the run did
      // not remove, reads as gone only while it re-registers (the router or
      // its worker restarting, a slow Keychain read): look again before the
      // step shows it disconnected. Signing in to one provider must not
      // appear to sign the others out.
      const removed = new Set(
        steps.flatMap((step) =>
          step.kind === 'remove-workers' ? step.workers : [],
        ),
      )
      const kept = [...servingBefore].filter(
        (id) => !removed.has(PROVIDER_WORKERS.get(id) ?? `provider-${id}`),
      )
      let after = await refresh()
      for (let look = 0; look < SETTLE_LOOKS; look++) {
        const serving = servingProviders(after)
        if (kept.every((id) => serving.has(id)) || cancel.current.cancelled) {
          break
        }
        await new Promise((resolve) => window.setTimeout(resolve, SETTLE_MS))
        after = await refresh()
      }
      setRunning(null)
      onSettledRef.current?.()
      return ok
    },
    [patch, refresh, snapshot.consoleConfig],
  )

  const [chromiumProgress, setChromiumProgress] =
    useState<ChromiumInstallProgress | null>(null)

  /**
   * Download Chromium for the browser worker, logged like any other setup
   * action, with every progress event kept for the step's progress bar.
   */
  const installChromium =
    useCallback(async (): Promise<ChromiumInstallOutcome> => {
      setRunning('browser')
      setChromiumProgress(null)
      const id = nextId.current++
      setActivity((current) => [
        ...current,
        {
          id,
          group: 'browser',
          title: 'Download Chromium',
          status: 'running',
          startedAt: Date.now(),
        },
      ])
      const abort = new AbortController()
      const token = cancel.current
      let skipped = false
      skipCurrent.current = () => {
        skipped = true
        abort.abort()
      }
      const outcome = await followChromiumInstall({
        signal: abort.signal,
        onProgress: (progress) => {
          if (token.cancelled) abort.abort()
          setChromiumProgress(progress)
          patch(id, {
            note: describeProgress(progress),
            progress: progressFraction(progress),
          })
        },
      })
      skipCurrent.current = null
      if (skipped && !outcome.ok) {
        patch(id, {
          status: 'skipped',
          note: 'skipped — setup moved on',
          progress: undefined,
        })
        await checkChromium()
        setRunning(null)
        return { ok: false, error: SKIPPED }
      }
      if (outcome.ok) {
        patch(id, {
          status: 'done',
          note: 'Chromium is ready',
          progress: undefined,
        })
      } else {
        patch(id, {
          status: 'failed',
          note: outcome.hint
            ? `${outcome.error} ${outcome.hint}`
            : outcome.error,
          progress: undefined,
        })
      }
      await checkChromium()
      setRunning(null)
      return outcome
    }, [checkChromium, patch])

  const judgeInstalled = snapshot.installed.has(JUDGE_HUB_WORKER)

  return {
    snapshot,
    scanning,
    refresh: rescan,
    activity,
    running,
    run,
    skipStep,
    judgeInstalled,
    checkChromium,
    installChromium,
    chromiumProgress,
  }
}

export type OnboardingController = ReturnType<typeof useOnboarding>
