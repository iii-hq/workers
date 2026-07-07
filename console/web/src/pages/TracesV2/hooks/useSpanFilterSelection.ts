// Server-persisted span-filter selection for the trace detail views.
//
// The hidden span groups / workers picked in the funnel menu live in the
// engine's `console` configuration entry under `traces.spanFilters` (model
// in `lib/spanFilters.ts`). ONE instance of this hook sits at the page
// level and is handed to both the timeline and the waterfall, so the
// selection is shared across the detail tabs, survives reloads, and — the
// config being server-side — follows the engine across browser tabs.
//
// Toggles hit LOCAL state immediately; persistence is a debounced
// fire-and-forget read-modify-write of the whole entry
// (`configuration::set` has no partial-update surface), so a slow or
// unavailable configuration worker never lags the menu — the selection
// then simply lives in memory for the session. Cross-tab concurrency is
// last-write-wins, same as saved views.

import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  type ConsoleConfigValue,
  fetchConsoleConfigValue,
  setConsoleConfigValue,
} from '@/lib/console-config'
import {
  EMPTY_SPAN_FILTERS,
  parseSpanFilters,
  type SpanFilterControls,
  type SpanFilterSelection,
  withSpanFilters,
} from '../lib/spanFilters'

// Same key as `useTraceViews` — both hooks read the one `console` entry,
// sharing the React Query cache.
const CONSOLE_CONFIG_QUERY_KEY = ['consoleConfig']

const SAVE_DEBOUNCE_MS = 400

export function useSpanFilterSelection(): SpanFilterControls {
  const qc = useQueryClient()
  const { data } = useQuery<ConsoleConfigValue | null>({
    queryKey: CONSOLE_CONFIG_QUERY_KEY,
    queryFn: fetchConsoleConfigValue,
    staleTime: 30_000,
    retry: 1,
  })

  const [selection, setSelection] =
    useState<SpanFilterSelection>(EMPTY_SPAN_FILTERS)

  // Hydrate once from the server value; after that, local toggles win
  // over refetches (last-write-wins is fine for a single-operator tool).
  const hydrated = useRef(false)
  useEffect(() => {
    if (hydrated.current || data == null) return
    hydrated.current = true
    setSelection(parseSpanFilters(data))
  }, [data])

  const persist = useCallback(
    async (next: SpanFilterSelection) => {
      try {
        const current = await fetchConsoleConfigValue()
        // Configuration worker unavailable — keep the selection in-memory.
        if (current == null) return
        const value = withSpanFilters(current, next)
        await setConsoleConfigValue(value)
        qc.setQueryData(CONSOLE_CONFIG_QUERY_KEY, value)
      } catch {
        // Best-effort persistence; the in-memory selection stays live.
      }
    },
    [qc],
  )

  // Debounced save: a burst of toggles collapses into one write. `dirty`
  // keeps the mount + hydration passes from writing back what was read.
  const dirty = useRef(false)
  useEffect(() => {
    if (!dirty.current) return
    const timer = setTimeout(() => void persist(selection), SAVE_DEBOUNCE_MS)
    return () => clearTimeout(timer)
  }, [selection, persist])

  const toggleGroup = useCallback((key: string) => {
    dirty.current = true
    setSelection((prev) => {
      const hiddenGroups = new Set(prev.hiddenGroups)
      if (!hiddenGroups.delete(key)) hiddenGroups.add(key)
      return { ...prev, hiddenGroups }
    })
  }, [])

  const toggleWorker = useCallback((key: string) => {
    dirty.current = true
    setSelection((prev) => {
      const hiddenWorkers = new Set(prev.hiddenWorkers)
      if (!hiddenWorkers.delete(key)) hiddenWorkers.add(key)
      return { ...prev, hiddenWorkers }
    })
  }, [])

  const clear = useCallback(() => {
    dirty.current = true
    setSelection(EMPTY_SPAN_FILTERS)
  }, [])

  // Stable identity so consumers can hang memos off the controls object —
  // it only changes when the selection itself does.
  return useMemo(
    () => ({ ...selection, toggleGroup, toggleWorker, clear }),
    [selection, toggleGroup, toggleWorker, clear],
  )
}
