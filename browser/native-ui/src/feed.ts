/**
 * The console's and the network's feed (design §8.4, the web panels'
 * pattern): bind the tab's event first, then seed with a read of the newest
 * 200; drop entries at or below the last seq seen, keep the newest 500.
 * Bound only while visible; visible again catches up from the last seq. A
 * filter change seeds again.
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
  /** Live entries that arrived while a full seed was out. */
  let early: E[] | null = null
  let generation = 0

  const append = (entries: E[]) => {
    if (!entries.length) return
    rows.splice(rows.items().length, 0, ...entries.map(options.row))
    const over = rows.items().length - MAX_ENTRIES
    if (over > 0) rows.splice(0, over)
  }

  const live = (payload: { session_id?: string; entry?: E }) => {
    const entry = payload.entry
    if (payload.session_id !== options.session || !entry || typeof entry.seq !== 'number') return
    if (early) {
      early.push(entry)
      return
    }
    if (entry.seq <= lastSeq) return
    lastSeq = entry.seq
    if (options.matches(entry)) append([entry])
  }

  const load = async (filters: string) => {
    const gen = ++generation
    const full = filters !== seededFor || lastSeq === 0
    if (full) early = []
    try {
      const res = await options.read(full ? undefined : lastSeq)
      if (gen !== generation) return
      const entries = Array.isArray(res?.entries) ? res.entries : []
      const last = typeof res?.last_seq === 'number' ? res.last_seq : 0
      setError(null)
      setDropped(typeof res?.dropped === 'number' ? res.dropped : 0)
      if (full) {
        const after = (early ?? []).filter((e) => e.seq > last && options.matches(e))
        early = null
        seededFor = filters
        lastSeq = Math.max(last, ...after.map((e) => e.seq))
        rows.set([...entries, ...after].slice(-MAX_ENTRIES).map(options.row))
      } else {
        const fresh = entries.filter((e) => e.seq > lastSeq)
        lastSeq = Math.max(lastSeq, last)
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

/** A pattern as the worker reads it is a regex; as the live filter, case-insensitive (the web's rule). */
export function patternTest(pattern: string): ((text: string) => boolean) | null {
  if (!pattern) return () => true
  try {
    const re = new RegExp(pattern, 'i')
    return (text) => re.test(text)
  } catch {
    return null
  }
}
