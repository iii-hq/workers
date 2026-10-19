import type { Host, SelectOption } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useLiveSignals } from '../catalog/engine'

/**
 * The judge providers a worker's `judge::evaluate` calls can reach: the
 * `judge-<provider>` workers registered next to the judge hub, in the namespace
 * the configured worker runs in (computer::run always goes through the hub).
 * Providers register their functions as internal, so only a listing with
 * internal functions shows them. The judge worker's own UI reads them the same
 * way (judge/ui listProviders); injected UI cannot import across workers, so
 * this is a copy on purpose.
 */

const LIST_TIMEOUT_MS = 10_000
const STATUS_TIMEOUT_MS = 5_000
const OPERATION_POLL_MS = 1_500
/** An add that compose finished but whose worker never registers is stuck. */
const REGISTER_GIVE_UP_MS = 120_000
const PROVIDER_FUNCTION = /^judge-([a-z0-9-]{1,64})::(evaluate|status)$/
/** What the judge hub installs with: its default provider. */
export const HUB_WORKER = 'judge'
export const DEFAULT_PROVIDER_WORKER = 'judge-typesafe'

export interface JudgeCatalog {
  /** Where `owner` (else the hub) runs; '' when neither is registered. */
  namespace: string
  hub: boolean
  /** Registered providers; `hosted` answers `judge-<key>::status` (holds a key). */
  providers: readonly { key: string; hosted: boolean }[]
}

export interface JudgeProvider {
  key: string
  hosted: boolean
  /** Why it cannot answer (a hosted judge with no key), or null. */
  problem: string | null
}

/** `engine::functions::list {include_internal: true}` → what `owner`'s namespace has. */
export function judgeCatalog(reply: unknown, owner: string): JudgeCatalog {
  const raw = (reply as { functions?: unknown } | null | undefined)?.functions
  const rows = (Array.isArray(raw) ? raw : []).filter(
    (row): row is { function_id: string; namespace?: unknown } =>
      Boolean(row) && typeof row === 'object' && typeof (row as { function_id?: unknown }).function_id === 'string',
  )
  const namespaceOf = (id: string) => rows.find((row) => row.function_id === id)?.namespace
  const found = namespaceOf(owner) ?? namespaceOf('judge::evaluate')
  const namespace = typeof found === 'string' ? found : ''
  // ponytail: the first `owner` row wins; two Compose projects on one engine
  // would need the form to know its own namespace.
  const here = rows.filter((row) => (typeof row.namespace === 'string' ? row.namespace : '') === namespace)
  const seen = new Map<string, { evaluates: boolean; hosted: boolean }>()
  for (const row of here) {
    const match = PROVIDER_FUNCTION.exec(row.function_id)
    if (!match) continue
    const entry = seen.get(match[1]) ?? { evaluates: false, hosted: false }
    if (match[2] === 'status') entry.hosted = true
    else entry.evaluates = true
    seen.set(match[1], entry)
  }
  return {
    namespace,
    hub: here.some((row) => row.function_id === 'judge::evaluate'),
    providers: [...seen]
      .filter(([, entry]) => entry.evaluates)
      .map(([key, entry]) => ({ key, hosted: entry.hosted }))
      .sort((a, b) => a.key.localeCompare(b.key)),
  }
}

/** `judge-<key>::status` → why it cannot answer, or null when it can. */
export function statusProblem(reply: unknown): string | null {
  const status = reply as { configured?: unknown; reason?: unknown } | null | undefined
  if (status?.configured !== false) return null
  return typeof status.reason === 'string' && status.reason.trim() ? status.reason.trim() : 'No usable API key'
}

/**
 * Select options: every registered provider, the ones that cannot answer
 * disabled with the reason, and a stored provider that is not installed kept
 * selectable so it can be seen and cleared.
 */
export function judgeOptions(providers: readonly JudgeProvider[], stored: string | undefined): SelectOption[] {
  const options: SelectOption[] = providers.map((provider) => ({
    value: provider.key,
    label: provider.key,
    description: provider.problem ?? (provider.hosted ? 'Hosted, key set' : 'Local model, loads on first use'),
    disabled: provider.problem !== null,
  }))
  if (stored && !options.some((option) => option.value === stored)) {
    options.push({ value: stored, label: stored, description: `judge-${stored} is not installed` })
  }
  return options
}

export interface JudgeSetup {
  /** Workers compose already declares but are not running: restart them. */
  start: readonly string[]
  /** Workers compose does not have: add them. */
  add: readonly string[]
}

/**
 * What makes a judge usable when none is: the hub (it installs with
 * judge-typesafe), else a provider. Declared-but-stopped workers are started,
 * the rest added. null while one is usable or nothing is known yet.
 */
export function judgeSetup(catalog: JudgeCatalog, compose: unknown): JudgeSetup | null {
  const needed = !catalog.hub ? [HUB_WORKER] : catalog.providers.length === 0 ? [DEFAULT_PROVIDER_WORKER] : []
  if (needed.length === 0) return null
  const projects = (compose as { projects?: unknown } | null | undefined)?.projects
  const project = (Array.isArray(projects) ? projects : []).find(
    (entry) => entry && typeof entry === 'object' && (!catalog.namespace || entry.namespace === catalog.namespace),
  ) as { containers?: unknown } | undefined
  const declared = new Set(
    (Array.isArray(project?.containers) ? project.containers : [])
      .map((entry) => (entry && typeof entry === 'object' ? (entry as { container?: unknown }).container : undefined))
      .filter((name): name is string => typeof name === 'string'),
  )
  return {
    start: needed.filter((name) => declared.has(name)),
    add: needed.filter((name) => !declared.has(name)),
  }
}

/** One line for a `compose::operation` snapshot: `phase · container · detail`. */
export function operationNote(snapshot: unknown): string | null {
  const s = snapshot as
    | {
        phase?: unknown
        completed?: unknown
        total?: unknown
        last_event?: { container?: unknown; detail?: unknown } | null
      }
    | null
    | undefined
  const parts = [s?.phase, s?.last_event?.container, s?.last_event?.detail].filter(
    (part): part is string => typeof part === 'string' && part.trim() !== '',
  )
  if (typeof s?.completed === 'number' && typeof s?.total === 'number' && s.total > 0) {
    parts.push(`${s.completed} of ${s.total}`)
  }
  return parts.length > 0 ? parts.join(' · ') : null
}

function operationId(): string {
  // crypto.randomUUID only exists in secure contexts; a LAN-IP console has none.
  const id =
    typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
      ? crypto.randomUUID()
      : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`
  return `ade-judge:${id}`
}

export interface JudgeProviders {
  catalog: JudgeCatalog | null
  providers: readonly JudgeProvider[] | null
  /** What a one-click setup would do; null when a judge is usable. */
  setup: JudgeSetup | null
  error: string | null
  reload(): void
}

/**
 * The providers `owner`'s judge calls can reach, re-read whenever a worker or
 * function comes or goes. Status is asked of hosted providers only: a local
 * one would start loading gigabytes just because the form opened.
 */
export function useJudgeProviders(host: Host, owner: string): JudgeProviders {
  const [state, setState] = useState<Omit<JudgeProviders, 'reload'>>({
    catalog: null,
    providers: null,
    setup: null,
    error: null,
  })
  const seq = useRef(0)

  const reload = useCallback(() => {
    const current = ++seq.current
    void (async () => {
      try {
        const listed = await host.iii.trigger(
          'engine::functions::list',
          { include_internal: true },
          { timeoutMs: LIST_TIMEOUT_MS },
        )
        const catalog = judgeCatalog(listed, owner)
        const providers = await Promise.all(
          catalog.providers.map(async ({ key, hosted }) => ({
            key,
            hosted,
            problem: hosted
              ? await host.iii
                  .trigger(`judge-${key}::status`, {}, { timeoutMs: STATUS_TIMEOUT_MS })
                  .then(statusProblem, (error: unknown) => `Status unavailable: ${errorMessage(error)}`)
              : null,
          })),
        )
        const compose =
          catalog.hub && catalog.providers.length > 0
            ? null
            : await host.iii.trigger('compose::list', {}, { timeoutMs: STATUS_TIMEOUT_MS }).catch(() => null)
        if (current !== seq.current) return
        setState({ catalog, providers, setup: judgeSetup(catalog, compose), error: null })
      } catch (error) {
        if (current !== seq.current) return
        setState((previous) => ({ ...previous, error: errorMessage(error) }))
      }
    })()
  }, [host, owner])

  useEffect(() => {
    reload()
  }, [reload])
  useLiveSignals(host, ['engine::functions-available', 'engine::workers-available'], reload)

  return { ...state, reload }
}

export type SetupState =
  | { phase: 'idle' }
  | { phase: 'running'; note: string | null }
  | { phase: 'failed'; error: string }

/**
 * Run a {@link JudgeSetup}: restart the declared workers, `compose::add` the
 * missing ones and follow the add to its end. Success is the judge
 * registering (the catalog stops asking for a setup), unless it never does.
 */
export function useJudgeSetup(host: Host, pending: boolean) {
  const [state, setState] = useState<SetupState>({ phase: 'idle' })
  const alive = useRef(true)
  useEffect(
    () => () => {
      alive.current = false
    },
    [],
  )

  // The worker registered: the setup is done, whatever is still in flight.
  useEffect(() => {
    if (!pending && state.phase === 'running') setState({ phase: 'idle' })
  }, [pending, state.phase])

  // Added but never registered: say so instead of spinning forever.
  useEffect(() => {
    if (state.phase !== 'running' || !pending) return
    const timer = window.setTimeout(
      () =>
        setState({
          phase: 'failed',
          error: 'It was installed but has not started. Check its logs in Settings → Workers.',
        }),
      REGISTER_GIVE_UP_MS,
    )
    return () => window.clearTimeout(timer)
  }, [state.phase, pending])

  const run = useCallback(
    async (setup: JudgeSetup) => {
      const report = (next: SetupState) => {
        if (alive.current) setState(next)
      }
      report({ phase: 'running', note: null })
      try {
        for (const container of setup.start) {
          await host.iii.trigger('compose::restart', { container }, { timeoutMs: 600_000 })
        }
        if (setup.add.length > 0) {
          const requested = operationId()
          const accepted = await host.iii.trigger<{
            operation_id?: unknown
            status?: unknown
            error?: { message?: unknown }
          }>('compose::add', { workers: setup.add, operation_id: requested }, { timeoutMs: 600_000 })
          if (accepted?.status === 'failed') {
            throw new Error(
              typeof accepted.error?.message === 'string' ? accepted.error.message : 'compose::add failed',
            )
          }
          // ponytail: polls compose::operation; the `compose-operation` trigger
          // (judge/ui addWorker) pushes the end instead, worth it only if many
          // tabs install at once.
          for (;;) {
            const snapshot = await host.iii.trigger<{ status?: unknown; last_event?: { detail?: unknown } | null }>(
              'compose::operation',
              // A daemon that ignores the caller's id ran it under its own.
              { operation_id: typeof accepted?.operation_id === 'string' ? accepted.operation_id : requested },
              { timeoutMs: STATUS_TIMEOUT_MS },
            )
            if (snapshot?.status === 'failed' || snapshot?.status === 'cancelled') {
              const detail = snapshot.last_event?.detail
              throw new Error(typeof detail === 'string' && detail ? detail : `compose::add ${snapshot.status}`)
            }
            if (snapshot?.status !== 'running') break
            report({ phase: 'running', note: operationNote(snapshot) })
            await new Promise((resolve) => window.setTimeout(resolve, OPERATION_POLL_MS))
            if (!alive.current) return
          }
        }
        report({ phase: 'running', note: 'Waiting for it to register' })
      } catch (error) {
        report({ phase: 'failed', error: errorMessage(error) })
      }
    },
    [host],
  )

  return { state, run }
}
