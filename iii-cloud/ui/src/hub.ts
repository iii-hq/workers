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
  /** The last `check` call's refusal for an environment, kept until the next check call or check record of it. */
  checkError(envId: string): WorkerError | null
  setCheckError(envId: string, error: WorkerError | null): void
  /**
   * The check that was current when the person replaced the source of an environment's check (the Change panel,
   * "Check <tip>", first-time Confirm): exactly that check is stale, any other is newer. null: not replaced. Kept in
   * storage per environment, so a reload or another environment's round trip does not forget it.
   */
  movedFrom(envId: string): string | null
  /** The compose file picked with that change: the retry after a refused check checks it, not the old check's. */
  pickedManifest(envId: string): string | null
  setPickedManifest(envId: string, manifest: string | null): void
  /** The environment the latest `project::use` selected, until the next status. */
  selectedEnv(): string
  /**
   * `project::use` succeeded; `envId` is the environment it returned. `changed`: the caller changed what is checked
   * (branch, compose file, source), so the environment's latest check is of the old source. A plain re-selection of an
   * environment for a route changes nothing and marks nothing.
   */
  used(envId: string, changed: boolean): void
  /** The worker answered that it refused the `use`: the move marked before sending it goes back to what it was. */
  restoreMove(envId: string, before: string | null): void
  /** The person stopped the agent of this environment: nothing starts one by itself until they check or ask again. */
  stopped(envId: string): boolean
  markStopped(envId: string, on: boolean): void
}

// Small things the person did, kept past this hub (and a reload) in localStorage; the memory copy only serves a
// browser without storage.
const memory = new Map<string, string>()
const readKey = (key: string): string | null => {
  try {
    const v = localStorage.getItem(key)
    if (v !== null) return v
  } catch {
    // no storage: the memory copy below
  }
  return memory.get(key) ?? null
}
const writeKey = (key: string, value: string | null) => {
  if (value === null) memory.delete(key)
  else memory.set(key, value)
  try {
    if (value === null) localStorage.removeItem(key)
    else localStorage.setItem(key, value)
  } catch {
    // storage is best effort: the in-memory copy keeps this tab right
  }
}
const STOPPED = (envId: string) => `iii-cloud:stopped:${envId}`
const MOVED = (envId: string) => `iii-cloud:moved:${envId}`
const PICKED = (envId: string) => `iii-cloud:picked:${envId}`
const readStopped = (envId: string) => readKey(STOPPED(envId)) === '1'
const writeStopped = (envId: string, on: boolean) => writeKey(STOPPED(envId), on ? '1' : null)
/** Forget every remembered stop, move and pick: storage and memory (tests). */
export function clearHubStore() {
  const ours = (k: string) => /^iii-cloud:(stopped|moved|picked):/.test(k)
  try {
    for (const k of Object.keys(localStorage).filter(ours)) localStorage.removeItem(k)
  } catch {
    // no storage
  }
  memory.clear()
}
/** Forget the memory copy only, as a reload does: what is left is what storage holds (tests). */
export function forgetHubMemory() {
  memory.clear()
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
  const checkErrors = new Map<string, WorkerError>()
  let pendingEnv = ''
  let pendingSeq = 0
  /** A check that arrived for an environment: a recorded move is over once it is another one than the replaced. */
  const newer = (envId: string, checkId: string) => {
    const m = readKey(MOVED(envId))
    if (m && m !== checkId) {
      writeKey(MOVED(envId), null)
      writeKey(PICKED(envId), null)
    }
  }
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
    // A check that arrived (the agent's own, one that outlived the page's call) replaces a refusal kept for it.
    if (ev.kind === 'check') {
      const c = ev.record as Check
      checkErrors.delete(c.environment_id)
      newer(c.environment_id, c.id)
    }
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
    checkError: (envId) => checkErrors.get(envId) ?? null,
    setCheckError(envId, error) {
      if (error) checkErrors.set(envId, error)
      else if (!checkErrors.delete(envId)) return
      emit()
    },
    stopped: readStopped,
    markStopped: writeStopped,
    movedFrom: (envId) => readKey(MOVED(envId)),
    pickedManifest: (envId) => readKey(PICKED(envId)),
    setPickedManifest(envId, manifest) {
      writeKey(PICKED(envId), manifest)
      emit()
    },
    selectedEnv: () => pendingEnv || (state.status?.project?.environment?.id ?? ''),
    used(envId, changed) {
      pendingEnv = envId
      pendingSeq = seq
      if (changed) {
        // That environment's latest check, not the selected one's (`status.check`) when another is selected now.
        const s = state.status
        const latest =
          s?.check && s.check.environment_id === envId
            ? s.check.id
            : s?.checks?.find((x) => x.environment_id === envId)?.id
        // Never overwrite a recorded move with nothing: no check known means there is no stale one to hold back.
        if (latest) writeKey(MOVED(envId), latest)
      }
      emit()
    },
    restoreMove(envId, before) {
      writeKey(MOVED(envId), before)
      emit()
    },
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
        // The next status read after a `use` says what is selected, whatever it reports.
        if (mine > pendingSeq) pendingEnv = ''
        // The newest check of each environment ends a move recorded for it, when it is another one.
        const seen = new Set<string>()
        for (const c of [...(next.check ? [next.check] : []), ...(next.checks ?? [])]) {
          if (seen.has(c.environment_id)) continue
          seen.add(c.environment_id)
          newer(c.environment_id, c.id)
        }
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
        // Another tab marked a move or stopped the agent (`localStorage`): what this tab shows from it is read again.
        const onStorage = (e: StorageEvent) => {
          if (!e.key || e.key.startsWith('iii-cloud:')) emit()
        }
        window.addEventListener('storage', onStorage)
        const off = host.iii.on<ChangeEvent>(EVENTS_FN, onEvent)
        const unobserve = host.iii.observe('iii-cloud::on-change', `${EVENTS_FN}::${host.iii.browserId}`)
        dispose = () => {
          window.removeEventListener('storage', onStorage)
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

/** The last refused `check` of an environment (kept after the card that asked is gone), and when its source moved. */
export function useCheckError(host: Host, envId: string): WorkerError | null {
  const hub = hubOf(host)
  return useSyncExternalStore(hub.subscribe, () => hub.checkError(envId))
}
export function useMovedFrom(host: Host, envId: string): string | null {
  const hub = hubOf(host)
  return useSyncExternalStore(hub.subscribe, () => hub.movedFrom(envId))
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
    async <T>(
      key: string,
      id: string,
      payload: Record<string, unknown> = {},
      /** `changesSource`: this `project::use` changes what is checked (not a plain re-selection for a route). */
      opts: { changesSource?: boolean } = {},
    ): Promise<T | null> => {
      setBusy(key)
      setError(null)
      const isCheck = id === 'iii-cloud::check'
      // The environment the call is for, taken now: the person may open another one before it ends.
      const checkEnv = isCheck ? hubOf(host).selectedEnv() : ''
      if (isCheck) {
        hubOf(host).checking(1)
        hubOf(host).setCheckError(checkEnv, null)
        // A new check is the person asking again: the agent may start on what it finds.
        hubOf(host).markStopped(checkEnv, false)
      }
      // A `use` that changes the source is marked BEFORE it is sent: the page may give up on it (60 s) while the worker
      // goes on and finishes it. Only the worker's answer that it refused takes the mark back.
      const marksMove = id === 'iii-cloud::project::use' && !!opts.changesSource
      const useEnv = typeof payload.environment_id === 'string' ? payload.environment_id : hubOf(host).selectedEnv()
      const before = marksMove ? hubOf(host).movedFrom(useEnv) : null
      if (marksMove) hubOf(host).used(useEnv, true)
      try {
        const out = await host.iii.call<T>(id, payload, {
          timeoutMs: id === 'iii-cloud::check' || id === 'iii-cloud::deploy' ? LONG : 60_000,
        })
        // Every `project::use` goes through here. Its environment is the one the Project it returns says (the payload may
        // name none, or the worker may fall back to another); the callers that change the source say so.
        if (id === 'iii-cloud::project::use') {
          const named = (out as { environment?: { id?: string } | null } | null)?.environment?.id
          // The worker fell back to another environment than the one marked up front: that one was not replaced.
          if (marksMove && named && named !== useEnv) hubOf(host).restoreMove(useEnv, before)
          hubOf(host).used(
            named ?? (typeof payload.environment_id === 'string' ? payload.environment_id : hubOf(host).selectedEnv()),
            !!opts.changesSource,
          )
        }
        // The event also arrives over the hub; this keeps the page right if it does not.
        void hubOf(host).refresh(false)
        return out
      } catch (err) {
        const refused = workerError(err, errorCode(err), errorMessage(err))
        if (alive.current) setError(refused)
        // The worker answered (it is not a timeout or a lost connection): it refused the `use`, nothing was replaced.
        if (
          marksMove &&
          !/timeout|timed out|abort|network|connection|closed|disconnect/i.test(`${refused.code} ${refused.message}`)
        )
          hubOf(host).restoreMove(useEnv, before)
        // The card that asked may be gone (the page went to the Deploy screen): the hub keeps the refusal.
        if (isCheck) hubOf(host).setCheckError(checkEnv, refused)
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
