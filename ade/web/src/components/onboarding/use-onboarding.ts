import { useCallback, useEffect, useRef, useState } from 'react'
import {
  detectKeys,
  installedWorkerNames,
  readableError,
  readConsoleConfig,
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
  describeStep,
  type KeyDetection,
  type PlanStep,
  type ProviderState,
  servesUsableModels,
  type ToolScan,
} from '@/lib/onboarding/plan'

/**
 * Which part of setup an action belongs to; each step shows its own. `keys`
 * is the models step's key search (adding the secrets worker), kept apart
 * from connecting so its log reads as what it is.
 */
export type ActivityGroup = 'keys' | 'models' | 'judge'

export interface ActivityEntry {
  id: number
  group: ActivityGroup
  title: string
  /** The engine operation behind it, shown in mono. */
  detail: string
  status: 'running' | 'done' | 'failed'
  /** Live progress line while running; the outcome once finished. */
  note?: string
  progress?: number
  /** Workers this entry added, for the summary. */
  workers?: string[]
}

export interface MachineSnapshot {
  tools: ToolScan[] | null
  toolsError: string | null
  providers: ProviderState[] | null
  providersError: string | null
  /** `null` until the secrets worker is running to look. */
  detections: KeyDetection[] | null
  installed: ReadonlySet<string>
  consoleConfig: Record<string, unknown> | null
}

const EMPTY: MachineSnapshot = {
  tools: null,
  toolsError: null,
  providers: null,
  providersError: null,
  detections: null,
  installed: new Set(),
  consoleConfig: null,
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
  const [scanning, setScanning] = useState(false)
  const [activity, setActivity] = useState<ActivityEntry[]>([])
  const [running, setRunning] = useState<ActivityGroup | null>(null)
  const nextId = useRef(1)
  const onSettledRef = useRef(onSettled)
  onSettledRef.current = onSettled
  const cancel = useRef({ cancelled: false })

  const refresh = useCallback(async () => {
    setScanning(true)
    const [tools, providers, installed, consoleConfig] = await Promise.all([
      settle(scanMachine()),
      settle(readProviderStates()),
      settle(installedWorkerNames()),
      readConsoleConfig(),
    ])
    const names = installed.value ?? new Set<string>()
    const detections = names.has(SECRETS_WORKER)
      ? await settle(detectKeys(DETECTED_KEY_NAMES))
      : { value: null, error: null }
    setSnapshot({
      tools: tools.value ?? [],
      toolsError: tools.error,
      providers: providers.value
        ? withWorkerPresence(providers.value, installed.value)
        : null,
      providersError: providers.error,
      detections: detections.value,
      installed: names,
      consoleConfig,
    })
    setScanning(false)
  }, [])

  useEffect(() => {
    if (!enabled) return
    const token = { cancelled: false }
    cancel.current = token
    void refresh()
    return () => {
      token.cancelled = true
    }
  }, [enabled, refresh])

  const patch = useCallback((id: number, next: Partial<ActivityEntry>) => {
    setActivity((current) =>
      current.map((entry) => (entry.id === id ? { ...entry, ...next } : entry)),
    )
  }, [])

  /** Run `steps` in order; stops at the first failure. Resolves to success. */
  const run = useCallback(
    async (group: ActivityGroup, steps: readonly PlanStep[]) => {
      setRunning(group)
      const consoleConfig =
        snapshot.consoleConfig ?? (await readConsoleConfig())
      let ok = true
      for (const step of steps) {
        const id = nextId.current++
        const { title, detail } = describeStep(step)
        setActivity((current) => [
          ...current,
          {
            id,
            group,
            title,
            detail,
            status: 'running',
            workers: step.kind === 'add-workers' ? step.workers : undefined,
          },
        ])
        try {
          const result = await runStep(step, {
            consoleConfig,
            signal: cancel.current,
            report: ({ note, progress }) => patch(id, { note, progress }),
          })
          patch(id, { status: 'done', note: result.note, progress: undefined })
        } catch (error) {
          patch(id, {
            status: 'failed',
            note: readableError(error),
            progress: undefined,
          })
          ok = false
          break
        }
      }
      await refresh()
      setRunning(null)
      onSettledRef.current?.()
      return ok
    },
    [patch, refresh, snapshot.consoleConfig],
  )

  const secretsInstalled = snapshot.installed.has(SECRETS_WORKER)
  const judgeInstalled = snapshot.installed.has(JUDGE_HUB_WORKER)

  return {
    snapshot,
    scanning,
    refresh,
    activity,
    running,
    run,
    secretsInstalled,
    judgeInstalled,
  }
}

export type OnboardingController = ReturnType<typeof useOnboarding>
