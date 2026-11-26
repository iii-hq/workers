import type { Host } from '@iii-dev/console-ui'
import { errorCode, errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { EVENTS_FN, signInRequired, workerError } from './model'
import type {
  AgentRun,
  ChangeEvent,
  Check,
  Connection,
  Deploy,
  Project,
  Run,
  SecretRequest,
  Status,
  WorkerError,
} from './types'

export type HubState = { status: Status | null; error: string | null; loaded: boolean; checking: number }

export type Hub = {
  get(): HubState
  subscribe(listener: () => void): () => void
  refresh(fresh?: boolean): Promise<void>
  /** A check is running: the page shows it everywhere, not only on the button that asked. */
  checking(delta: 1 | -1): void
  record(kind: string, id: string): unknown
  recordVersion(): number
}

const hubs = new WeakMap<Host, Hub>()
const bySort = (a: Deploy, b: Deploy) => Date.parse(b.created_at) - Date.parse(a.created_at)

/** Applies one `iii-cloud::on-change` record to the status: upsert by kind and id. */
export function applyChange(s: Status, ev: ChangeEvent): Status {
  switch (ev.kind) {
    case 'connection':
      return { ...s, connection: ev.record as Connection }
    case 'project':
      return { ...s, project: ev.record as Project }
    case 'check': {
      const c = ev.record as Check
      const env = s.project?.environment?.id
      const current = !s.check || s.check.id === c.id || Date.parse(c.checked_at) >= Date.parse(s.check.checked_at)
      return (!env || c.environment_id === env) && current ? { ...s, check: c } : s
    }
    case 'run': {
      const r = ev.record as Run
      return s.check && s.check.id === r.check_id ? { ...s, check: { ...s.check, run: r } } : s
    }
    case 'secret': {
      const r = ev.record as SecretRequest
      const rest = s.secrets.filter((x) => x.id !== r.id)
      const next = [...rest, r].sort(
        (a, b) =>
          Number(b.state === 'waiting') - Number(a.state === 'waiting') ||
          Date.parse(b.asked_at) - Date.parse(a.asked_at),
      )
      return { ...s, secrets: next }
    }
    case 'agent':
      return { ...s, agent: ev.record as AgentRun }
    case 'deploy': {
      const d = ev.record as Deploy
      return { ...s, deploys: [d, ...s.deploys.filter((x) => x.id !== d.id)].sort(bySort).slice(0, 20) }
    }
    default:
      return s
  }
}

/** One hub per tab: one `on` handler and one observer, shared by every mounted page and card. */
export function hubOf(host: Host): Hub {
  const known = hubs.get(host)
  if (known) return known
  let state: HubState = { status: null, error: null, loaded: false, checking: 0 }
  let version = 0
  let seq = 0
  const records = new Map<string, unknown>()
  const listeners = new Set<() => void>()
  let dispose: (() => void) | null = null
  const emit = () => {
    for (const l of [...listeners]) l()
  }
  const set = (next: Partial<HubState>) => {
    state = { ...state, ...next }
    emit()
  }
  const remember = (kind: string, id: string, record: unknown) => {
    records.set(`${kind}:${id}`, record)
    version += 1
  }
  const onEvent = (ev: ChangeEvent) => {
    if (!ev || typeof ev !== 'object' || !ev.kind || !ev.record) return
    remember(ev.kind, ev.id, ev.record)
    if (ev.kind === 'run') {
      const r = ev.record as Run
      const c = records.get(`check:${r.check_id}`) as Check | undefined
      if (c) remember('check', c.id, { ...c, run: r })
    }
    set({ status: state.status ? applyChange(state.status, ev) : state.status })
  }
  const hub: Hub = {
    get: () => state,
    record: (kind, id) => records.get(`${kind}:${id}`),
    recordVersion: () => version,
    checking(delta) {
      set({ checking: Math.max(0, state.checking + delta) })
    },
    async refresh(fresh = false) {
      const mine = ++seq
      try {
        const next = await host.iii.call<Status>('iii-cloud::status', fresh ? { refresh: true } : {})
        if (mine !== seq) return
        if (next.check) remember('check', next.check.id, next.check)
        if (next.check?.run) remember('run', next.check.run.id, next.check.run)
        for (const d of next.deploys) remember('deploy', d.id, d)
        for (const r of next.secrets) remember('secret', r.id, r)
        set({ status: next, error: null, loaded: true })
      } catch (err) {
        if (mine === seq) set({ error: errorMessage(err), loaded: true })
      }
    },
    subscribe(listener) {
      listeners.add(listener)
      if (listeners.size === 1) {
        // A card in the chat can mount before any page: it still needs the env names.
        if (!state.loaded) void hub.refresh(false)
        const off = host.iii.on<ChangeEvent>(EVENTS_FN, onEvent)
        const unobserve = host.iii.observe('iii-cloud::on-change', `${EVENTS_FN}::${host.iii.browserId}`)
        dispose = () => {
          unobserve()
          off()
        }
      }
      return () => {
        listeners.delete(listener)
        if (listeners.size === 0) {
          dispose?.()
          dispose = null
        }
      }
    },
  }
  hubs.set(host, hub)
  return hub
}

/** The status, the 30 s refresh while the page is visible, and a first read on mount. */
export function useStatus(host: Host, { poll }: { poll: boolean }) {
  const hub = hubOf(host)
  const state = useSyncExternalStore(hub.subscribe, hub.get)
  useEffect(() => {
    void hub.refresh(false)
    if (!poll) return
    const t = setInterval(() => {
      if (document.visibilityState === 'visible') void hub.refresh(true)
    }, 30_000)
    return () => clearInterval(t)
  }, [hub, poll])
  return { ...state, refresh: hub.refresh }
}

/** A card's record: the first one it was handed, then the same record by id as the hub sees it. */
export function useRecord<T>(host: Host, kind: string, id: string, initial: T): T {
  const hub = hubOf(host)
  useSyncExternalStore(hub.subscribe, hub.recordVersion)
  return (hub.record(kind, id) as T | undefined) ?? initial
}

/** A check is running somewhere on this page. */
export function useChecking(host: Host): boolean {
  const hub = hubOf(host)
  return useSyncExternalStore(hub.subscribe, () => hub.get().checking > 0)
}

export function useStatusSnapshot(host: Host): Status | null {
  const hub = hubOf(host)
  return useSyncExternalStore(hub.subscribe, () => hub.get()).status
}

/** The worker stopped answering: the page shows the last status it had. */
export function useStale(host: Host): boolean {
  const hub = hubOf(host)
  return useSyncExternalStore(hub.subscribe, () => hub.get().error !== null)
}

/** Out of date, or iii cloud wants a fresh sign-in: deploy, rollback and every save stay locked. */
export const useLocked = (host: Host, status: Status) => useStale(host) || signInRequired(status)

const LONG = 20 * 60_000

/** Runs one worker call at a time per key, keeping the busy key and the worker's error. */
export function useActions(host: Host) {
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<WorkerError | null>(null)
  const alive = useRef(true)
  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
    }
  }, [])
  const call = useCallback(
    async <T>(key: string, id: string, payload: Record<string, unknown> = {}): Promise<T | null> => {
      setBusy(key)
      setError(null)
      const isCheck = id === 'iii-cloud::check'
      if (isCheck) hubOf(host).checking(1)
      try {
        const out = await host.iii.call<T>(id, payload, {
          timeoutMs: id === 'iii-cloud::check' || id === 'iii-cloud::deploy' ? LONG : 60_000,
        })
        // The event also arrives over the hub; this keeps the page right if it does not.
        void hubOf(host).refresh(false)
        return out
      } catch (err) {
        if (alive.current) setError(workerError(err, errorCode(err), errorMessage(err)))
        return null
      } finally {
        if (isCheck) hubOf(host).checking(-1)
        if (alive.current) setBusy(null)
      }
    },
    [host],
  )
  return { busy, error, setError, call }
}
