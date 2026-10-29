/**
 * The console's and the network's feed (design §8.4, the web panels'
 * pattern): bind the tab's event first, then seed with a read of the newest
 * 200; drop entries at or below the last seq seen, keep the newest 500.
 * Bound only while visible; visible again catches up from the last seq. A
 * filter change seeds again. The worker's data is not trusted: an entry
 * without a numeric seq is skipped, a repeated seq keeps its last entry, and
 * a row that cannot be drawn is left out instead of failing the list.
 */

import { createEffect, createList, createSignal, iii, type ListSource, onCleanup, untrack } from '@iii-dev/native-ui'
import { type LogRead, listen } from './api'
import { readable } from './format'

export const SEED_LIMIT = 200
export const MAX_ENTRIES = 500

export interface Feed<R extends object> {
  rows: ListSource<R>
  /** The last read's failure, in words. */
  error: () => string | null
  /** The first seed answered. */
  loaded: () => boolean
  /** Entries the worker's ring buffer dropped before the seed. */
  dropped: () => number
  /** Empties the list (what was seen stays seen). */
  clear(): void
  /** Seeds again. */
  retry(): void
}

export function createFeed<E extends { seq: number }, R extends { key: string; kind: number }>(options: {
  session: string
  trigger: string
  /** Reads with the current filters; `sinceSeq` for a catch-up. */
  read: (sinceSeq: number | undefined) => Promise<LogRead<E>>
  /** Whether a live entry passes the current filters. */
  matches: (entry: E) => boolean
  row: (entry: E) => R
  /** Reactive: the filters; a change seeds again. */
  filters: () => string
}): Feed<R> {
  const rows = createList<R>({ key: 'key', kind: 'kind' })
  const [error, setError] = createSignal<string | null>(null)
  const [loaded, setLoaded] = createSignal(false)
  const [dropped, setDropped] = createSignal(0)
  const [bound, setBound] = createSignal(false)
  let lastSeq = 0
  let seededFor: string | null = null
  /** Live entries that pass the filters and arrived while a full seed was out: the newest MAX_ENTRIES. */
  let early: E[] | null = null
  let generation = 0

  const matches = (entry: E) => {
    try {
      return options.matches(entry)
    } catch {
      return false
    }
  }

  /** The rows of `entries`, one per seq (the last entry of a seq wins), skipping what cannot be drawn. */
  const rowsOf = (entries: E[]): R[] => {
    const bySeq = new Map<number, E>()
    for (const e of entries) {
      bySeq.delete(e.seq)
      bySeq.set(e.seq, e)
    }
    const out: R[] = []
    for (const e of bySeq.values()) {
      try {
        out.push(options.row(e))
      } catch {
        // A malformed entry: left out.
      }
    }
    return out
  }

  const append = (entries: E[]) => {
    const fresh = rowsOf(entries)
    if (!fresh.length) return
    rows.splice(rows.items().length, 0, ...fresh)
    const over = rows.items().length - MAX_ENTRIES
    if (over > 0) rows.splice(0, over)
  }

  const live = (payload: { session_id?: string; entry?: E }) => {
    const entry = payload.entry
    if (payload.session_id !== options.session || !valid(entry)) return
    if (early) {
      if (!matches(entry)) return
      early.push(entry)
      // Bounded like the list: a page logging thousands of lines during the seed keeps the newest.
      if (early.length >= 2 * MAX_ENTRIES) early.splice(0, early.length - MAX_ENTRIES)
      return
    }
    if (entry.seq <= lastSeq) return
    lastSeq = entry.seq
    if (matches(entry)) append([entry])
  }

  const load = async (filters: string) => {
    const gen = ++generation
    const full = filters !== seededFor || lastSeq === 0
    if (full) early = []
    try {
      const res = await options.read(full ? undefined : lastSeq)
      if (gen !== generation) return
      const entries = (Array.isArray(res?.entries) ? res.entries : []).filter(valid)
      const last = typeof res?.last_seq === 'number' && Number.isFinite(res.last_seq) ? res.last_seq : 0
      setError(null)
      setDropped(typeof res?.dropped === 'number' ? res.dropped : 0)
      if (full) {
        const after = (early ?? []).filter((e) => e.seq > last)
        early = null
        seededFor = filters
        lastSeq = after.reduce((max, e) => Math.max(max, e.seq), Math.max(last, maxSeq(entries)))
        rows.set(rowsOf([...entries, ...after]).slice(-MAX_ENTRIES))
      } else {
        const fresh = entries.filter((e) => e.seq > lastSeq)
        lastSeq = Math.max(lastSeq, last, maxSeq(fresh))
        append(fresh)
      }
    } catch (err) {
      if (gen !== generation) return
      early = null
      setError(readable(err))
    } finally {
      if (gen === generation) setLoaded(true)
    }
  }

  createEffect(() => {
    if (!iii.env.visible) return
    const off = listen<{ session_id?: string; entry?: E }>(options.trigger, { session_id: options.session }, live)
    let alive = true
    onCleanup(() => {
      alive = false
      off()
      setBound(false)
    })
    void off.ready.then(() => {
      if (alive) setBound(true)
    })
  })

  createEffect(() => {
    if (!bound()) return
    const filters = options.filters()
    untrack(() => void load(filters))
  })

  return {
    rows,
    error,
    loaded,
    dropped,
    clear: () => rows.set([]),
    retry: () => {
      seededFor = null
      void load(untrack(options.filters))
    },
  }
}

function valid<E extends { seq: number }>(entry: E | null | undefined): entry is E {
  return !!entry && typeof entry === 'object' && typeof entry.seq === 'number' && Number.isFinite(entry.seq)
}

const maxSeq = (entries: { seq: number }[]) => entries.reduce((max, e) => Math.max(max, e.seq), 0)

/**
 * A filter field's test on live entries: a case-insensitive substring. Never
 * the person's text as a regex in the VM: on the device a catastrophic
 * pattern backtracks into the 250 ms watchdog, which ends the whole app.
 */
export function textFilter(text: string): (value: unknown) => boolean {
  const needle = text.trim().toLowerCase()
  if (!needle) return () => true
  return (value) => typeof value === 'string' && value.toLowerCase().includes(needle)
}

/**
 * The same filter as the worker's `pattern` (a Rust regex, linear time):
 * the text escaped, case-insensitive. Undefined for an empty field.
 */
export function workerPattern(text: string): string | undefined {
  const t = text.trim()
  return t ? `(?i)${t.replace(/[\\.+*?()|[\]{}^$#&\-~]/g, '\\$&')}` : undefined
}
