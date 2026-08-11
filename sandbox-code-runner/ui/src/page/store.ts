/**
 * The fleet store — the page's single source of truth for sandboxes,
 * catalog images, and code-runner runtimes. PUSH-driven: the worker's
 * fleet watcher emits `{ kind: "fleet" }` events carrying full
 * `sandbox::list` snapshots, applied here via `applyFleetEvent` — the
 * house doctrine's "one get on mount, push after". The store performs
 * exactly one initial fetch (retried with backoff until it lands) and
 * after that NO timers run: fleet changes arrive as events, runtime-kind
 * events trigger a single debounced `list_runtimes` re-read, and the
 * refresh button stays for humans.
 *
 * A `sandbox::list` failure that reads as function-not-found flips
 * `daemonAbsent` — rendered as its own empty state; the watcher emits the
 * transition back when the daemon appears.
 *
 * Framework-free on purpose: timers and the trigger transport are
 * injectable, so store.test.ts drives bootstrap, backoff, and event
 * application deterministically. useFleet.ts is the React face.
 */

import { asRecord } from '../lib/payload'

export const BOOTSTRAP_RETRY_MS = 3_000
export const MAX_BACKOFF_MS = 30_000
export const RUNTIMES_DEBOUNCE_MS = 250
export const EXEC_SLOTS = 4

export const LIST_FN = 'sandbox::list'
export const CATALOG_FN = 'sandbox::catalog::list'
export const RUNTIMES_FN = 'sandbox-code-runner::list_runtimes'

export interface SandboxSummary {
  sandbox_id: string
  name: string | null
  image: string
  age_secs: number
  exec_in_progress: boolean
  exec_in_flight: number
  exec_slots_free: number
  stopped: boolean
  /** Idle-reap fields the daemon does not report yet — parsed tolerantly
   *  so the TTL countdown lights up the day it does. */
  idle_secs: number | null
  idle_timeout_secs: number | null
  reap_in_secs: number | null
}

/** Seconds until the daemon reaps this sandbox, when derivable: an
 *  explicit `reap_in_secs` wins, else `idle_timeout_secs - idle_secs`. */
export function deriveReapInSecs(sandbox: {
  idle_secs: number | null
  idle_timeout_secs: number | null
  reap_in_secs: number | null
}): number | null {
  if (sandbox.reap_in_secs !== null) return sandbox.reap_in_secs
  if (sandbox.idle_timeout_secs !== null && sandbox.idle_secs !== null)
    return sandbox.idle_timeout_secs - sandbox.idle_secs
  return null
}

export interface CatalogImage {
  name: string
  oci_ref: string
  kind: string
}

export interface RuntimeInfo {
  runtime_id: string
  lang: string
  sandbox_id: string | null
  created_at_ms: number | null
  registered_functions: string[]
  /** true = the backing VM left sandbox::list (reaped or stopped);
   *  null = the worker could not reconcile, liveness unknown. */
  vm_gone: boolean | null
}

/** A sandbox that just left the fleet — kept visible briefly so a reaper
 *  kill is an event the operator sees, not a silent disappearance. */
export interface FleetTombstone {
  sandbox: SandboxSummary
  goneAt: number
}

export interface FleetState {
  sandboxes: SandboxSummary[]
  images: CatalogImage[]
  runtimes: RuntimeInfo[]
  tombstones: FleetTombstone[]
  loading: boolean
  error: string | null
  daemonAbsent: boolean
  lastUpdatedAt: number | null
  /** When the current `sandboxes` snapshot landed. Change-only events go
   *  quiet while ages keep growing, so displayed ages are
   *  `age_secs + (now - snapshotAt) / 1000`. */
  snapshotAt: number | null
}

/** How long a vanished sandbox stays visible as a tombstone row. */
export const TOMBSTONE_TTL_MS = 30_000

/**
 * Sandboxes present in `prev` but absent from `next` become tombstones;
 * an id that reappears (unlikely, but ids are daemon-owned) is resurrected.
 */
export function reconcileTombstones(
  existing: FleetTombstone[],
  prev: SandboxSummary[],
  next: SandboxSummary[],
  at: number,
): FleetTombstone[] {
  const alive = new Set(next.map((s) => s.sandbox_id))
  const kept = existing.filter(
    (t) => !alive.has(t.sandbox.sandbox_id) && at - t.goneAt < TOMBSTONE_TTL_MS,
  )
  const known = new Set(kept.map((t) => t.sandbox.sandbox_id))
  for (const s of prev) {
    if (!alive.has(s.sandbox_id) && !known.has(s.sandbox_id)) {
      kept.push({ sandbox: s, goneAt: at })
    }
  }
  return kept
}

/* ── tolerant parsing ─────────────────────────────────────────────────── */

const asString = (v: unknown): string | null =>
  typeof v === 'string' && v.length > 0 ? v : null

const asNumber = (v: unknown): number | null =>
  typeof v === 'number' && Number.isFinite(v) ? v : null

/**
 * `{ sandboxes: [...] }` → summaries. Rows missing `sandbox_id` are
 * dropped; every other field degrades to a sane default, and the two
 * newer slot fields fall back to what `exec_in_progress` implies so an
 * older daemon still renders honestly.
 */
export function parseSandboxList(value: unknown): SandboxSummary[] {
  const list = asRecord(value)?.sandboxes
  if (!Array.isArray(list)) return []
  const out: SandboxSummary[] = []
  for (const item of list) {
    const rec = asRecord(item)
    const id = rec && asString(rec.sandbox_id)
    if (!rec || !id) continue
    const inProgress = rec.exec_in_progress === true
    const inFlight = asNumber(rec.exec_in_flight) ?? (inProgress ? 1 : 0)
    out.push({
      sandbox_id: id,
      name: asString(rec.name),
      image: asString(rec.image) ?? '',
      age_secs: asNumber(rec.age_secs) ?? 0,
      exec_in_progress: inProgress || inFlight > 0,
      exec_in_flight: inFlight,
      exec_slots_free:
        asNumber(rec.exec_slots_free) ?? Math.max(0, EXEC_SLOTS - inFlight),
      stopped: rec.stopped === true,
      idle_secs: asNumber(rec.idle_secs),
      idle_timeout_secs: asNumber(rec.idle_timeout_secs),
      reap_in_secs: asNumber(rec.reap_in_secs),
    })
  }
  return out
}

/** `{ images: [{ name, oci_ref, kind }] }` → catalog rows (name required). */
export function parseCatalog(value: unknown): CatalogImage[] {
  const list = asRecord(value)?.images
  if (!Array.isArray(list)) return []
  const out: CatalogImage[] = []
  for (const item of list) {
    const rec = asRecord(item)
    const name = rec && asString(rec.name)
    if (!rec || !name) continue
    out.push({
      name,
      oci_ref: asString(rec.oci_ref) ?? '',
      kind: asString(rec.kind) ?? 'custom',
    })
  }
  return out
}

/**
 * `{ runtimes: [...] }` (or a bare array) → runtime rows. Field names on
 * this surface are still settling, so the obvious aliases are accepted:
 * `runtime_id`/`id`, `lang`/`language`, `created_at_ms`/`created_at`,
 * `registered_functions`/`functions` (strings, or objects carrying a
 * `function_id`/`id`).
 */
export function parseRuntimes(value: unknown): RuntimeInfo[] {
  const raw = asRecord(value)?.runtimes ?? value
  if (!Array.isArray(raw)) return []
  const out: RuntimeInfo[] = []
  for (const item of raw) {
    const rec = asRecord(item)
    const id = rec && (asString(rec.runtime_id) ?? asString(rec.id))
    if (!rec || !id) continue
    const fnsRaw = Array.isArray(rec.registered_functions)
      ? rec.registered_functions
      : Array.isArray(rec.functions)
        ? rec.functions
        : []
    const fns: string[] = []
    for (const fn of fnsRaw) {
      const fromString = asString(fn)
      const fromRecord = asRecord(fn)
      const resolved =
        fromString ??
        (fromRecord
          ? (asString(fromRecord.function_id) ?? asString(fromRecord.id))
          : null)
      if (resolved) fns.push(resolved)
    }
    out.push({
      runtime_id: id,
      lang: asString(rec.lang) ?? asString(rec.language) ?? 'unknown',
      sandbox_id: asString(rec.sandbox_id),
      created_at_ms: asNumber(rec.created_at_ms) ?? asNumber(rec.created_at),
      vm_gone: typeof rec.vm_gone === 'boolean' ? rec.vm_gone : null,
      registered_functions: fns,
    })
  }
  return out
}

/** Does this error read as "the function does not exist on the bus"?
 *  "function" must sit right next to the not-found phrase (one token of
 *  slack for the id) — prose that merely contains "was not found" later
 *  in the sentence must not flag the daemon absent. */
export function isFunctionNotFound(message: string): boolean {
  return /\bfunction(\s+\S+)?\s+not[ _-]?(found|registered)\b|\bunknown function\b|\bno such function\b/i.test(
    message,
  )
}

/** Delay before the next poll after `consecutiveErrors` failures:
 *  0 → 3s (healthy cadence), 1 → 3s, 2 → 6s, 3 → 12s, 4 → 24s, 5+ → 30s. */
export function backoffDelayMs(consecutiveErrors: number): number {
  if (consecutiveErrors <= 1) return BOOTSTRAP_RETRY_MS
  return Math.min(BOOTSTRAP_RETRY_MS * 2 ** (consecutiveErrors - 1), MAX_BACKOFF_MS)
}

/* ── the store ────────────────────────────────────────────────────────── */

export type TriggerFn = (
  functionId: string,
  payload?: Record<string, unknown>,
) => Promise<unknown>

export interface FleetStoreDeps {
  isVisible?: () => boolean
  setTimer?: (fn: () => void, ms: number) => unknown
  clearTimer?: (id: unknown) => void
  now?: () => number
}

export interface FleetStore {
  getState(): FleetState
  subscribe(listener: () => void): () => void
  /** One initial fetch (catalog included); everything after is push. */
  start(): void
  /** Drop timers and in-flight completions. */
  stop(): void
  /** Apply a pushed `{ kind: "fleet" }` event; returns false for other
   *  kinds (the caller re-reads runtimes for those). */
  applyFleetEvent(payload: unknown): boolean
  /** Debounced single re-read of `list_runtimes` (runtime-kind events). */
  refreshRuntimes(): void
  /** Fetch now. `withCatalog` re-reads the image catalog too (the manual
   *  refresh button passes true; live-event pokes leave it false). */
  refresh(withCatalog?: boolean): void
}

export function createFleetStore(
  trigger: TriggerFn,
  deps: FleetStoreDeps = {},
): FleetStore {
  const setTimer =
    deps.setTimer ?? ((fn: () => void, ms: number) => setTimeout(fn, ms))
  const clearTimer = deps.clearTimer ?? ((id: unknown) => clearTimeout(id as number))
  const now = deps.now ?? Date.now

  let state: FleetState = {
    sandboxes: [],
    images: [],
    runtimes: [],
    tombstones: [],
    loading: true,
    error: null,
    daemonAbsent: false,
    lastUpdatedAt: null,
    snapshotAt: null,
  }
  const listeners = new Set<() => void>()
  let timer: unknown = null
  let running = false
  let fetching = false
  let queued: boolean | null = null
  let errors = 0
  let catalogLoaded = false
  // Bumped on stop(): in-flight completions from a previous run are dropped
  // instead of writing into (or re-scheduling) a stopped store.
  let generation = 0

  const set = (patch: Partial<FleetState>) => {
    state = { ...state, ...patch }
    for (const listener of listeners) listener()
  }

  const clearPending = () => {
    if (timer !== null) {
      clearTimer(timer)
      timer = null
    }
  }

  const schedule = (ms: number) => {
    clearPending()
    if (!running) return
    timer = setTimer(tick, ms)
  }

  function tick() {
    timer = null
    void fetchOnce(false)
  }

  async function fetchOnce(withCatalog: boolean): Promise<void> {
    if (fetching) {
      // Coalesce: remember the strongest request and re-run once.
      queued = queued || withCatalog
      return
    }
    fetching = true
    clearPending()
    const gen = generation
    // Loading only before the first data lands — background polls after
    // that must not flicker the UI's loading affordances.
    set({ loading: state.lastUpdatedAt === null })
    try {
      const sandboxes = parseSandboxList(await trigger(LIST_FN, {}))
      if (gen !== generation) return

      // Runtimes ride along; the code-runner being absent or mid-restart
      // must not fail the fleet read.
      let runtimes: RuntimeInfo[] = []
      try {
        runtimes = parseRuntimes(await trigger(RUNTIMES_FN, {}))
      } catch {
        runtimes = []
      }

      let images = state.images
      if (withCatalog || !catalogLoaded) {
        try {
          images = parseCatalog(await trigger(CATALOG_FN, {}))
          catalogLoaded = true
        } catch {
          /* keep the previous catalog */
        }
      }
      if (gen !== generation) return

      errors = 0
      set({
        sandboxes,
        runtimes,
        images,
        tombstones: reconcileTombstones(
          state.tombstones,
          state.sandboxes,
          sandboxes,
          now(),
        ),
        loading: false,
        error: null,
        daemonAbsent: false,
        lastUpdatedAt: now(),
        snapshotAt: now(),
      })
    } catch (err) {
      if (gen !== generation) return
      errors += 1
      const message = err instanceof Error ? err.message : String(err)
      set({
        loading: false,
        error: message,
        daemonAbsent: isFunctionNotFound(message),
      })
    } finally {
      if (gen === generation) {
        fetching = false
        if (queued !== null) {
          const catalog = queued
          queued = null
          void fetchOnce(catalog)
        } else if (errors > 0 && state.lastUpdatedAt === null) {
          // Bootstrap only: events own the steady state, so retry timers
          // exist solely until the first snapshot lands.
          schedule(backoffDelayMs(errors))
        }
      }
    }
  }

  let runtimesTimer: unknown = null

  const refreshRuntimes = () => {
    if (!running || runtimesTimer !== null) return
    runtimesTimer = setTimer(() => {
      runtimesTimer = null
      if (!running) return
      void (async () => {
        try {
          const runtimes = parseRuntimes(await trigger(RUNTIMES_FN, {}))
          set({ runtimes })
        } catch {
          /* the strip goes stale until the next event retries */
        }
      })()
    }, RUNTIMES_DEBOUNCE_MS)
  }

  const applyFleetEvent = (payload: unknown): boolean => {
    const rec = asRecord(payload)
    if (!rec || rec.kind !== 'fleet') return false
    const sandboxes = parseSandboxList({ sandboxes: rec.sandboxes })
    set({
      sandboxes,
      tombstones: reconcileTombstones(
        state.tombstones,
        state.sandboxes,
        sandboxes,
        now(),
      ),
      daemonAbsent: rec.daemon_absent === true,
      loading: false,
      error: null,
      lastUpdatedAt: now(),
      snapshotAt: now(),
    })
    refreshRuntimes()
    return true
  }

  return {
    getState: () => state,
    subscribe(listener) {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    start() {
      if (running) return
      running = true
      void fetchOnce(true)
    },
    stop() {
      running = false
      generation += 1
      fetching = false
      queued = null
      clearPending()
      if (runtimesTimer !== null) {
        clearTimer(runtimesTimer)
        runtimesTimer = null
      }
    },
    refresh(withCatalog = false) {
      if (!running) return
      void fetchOnce(withCatalog)
    },
    applyFleetEvent,
    refreshRuntimes,
  }
}
