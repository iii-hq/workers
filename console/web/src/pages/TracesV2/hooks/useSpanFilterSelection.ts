// Server-persisted span-filter selection for the trace detail views.
//
// The hidden span groups / workers picked in the funnel menu live in the
// engine's `console` configuration entry under `traces.spanFilters` (model
// in `lib/spanFilters.ts`). ONE instance of this hook sits at the page
// level and is handed to both the timeline and the waterfall, so the
// selection is shared across the detail tabs, survives reloads, and — the
// config being server-side — follows the engine across browser tabs.
//
// On top of the user's own picks, functions registered with
// `trace_hidden: true` metadata (session/context bookkeeping,
// `harness::turn` — see workers/docs/sops/trace-hidden-functions.md) are
// hidden BY DEFAULT: their group keys merge into `hiddenGroups` unless the
// user unhid them, and that unhide persists as `shownGroups`.
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
  EMPTY_HIDDEN_IDS,
  fetchTraceHiddenFunctionIds,
} from '@/lib/trace-hidden-functions'
import {
  EMPTY_SPAN_FILTER_PREFS,
  effectiveSpanFilters,
  parseSpanFilters,
  type SpanFilterControls,
  type SpanFilterPrefs,
  withSpanFilters,
} from '../lib/spanFilters'

// Same key as `useTraceViews` — both hooks read the one `console` entry,
// sharing the React Query cache.
const CONSOLE_CONFIG_QUERY_KEY = ['consoleConfig']

const TRACE_HIDDEN_FUNCTIONS_QUERY_KEY = ['traceHiddenFunctions']

const SAVE_DEBOUNCE_MS = 400

export function useSpanFilterSelection(): SpanFilterControls {
  const qc = useQueryClient()
  const { data } = useQuery<ConsoleConfigValue | null>({
    queryKey: CONSOLE_CONFIG_QUERY_KEY,
    queryFn: fetchConsoleConfigValue,
    staleTime: 30_000,
    retry: 1,
  })

  // Producer-default-hidden groups (`trace_hidden: true` registration
  // metadata). Registrations only change on worker deploys, so a long
  // staleTime is fine; failures resolve to the empty set (hide nothing).
  const { data: producerHidden } = useQuery<ReadonlySet<string>>({
    queryKey: TRACE_HIDDEN_FUNCTIONS_QUERY_KEY,
    queryFn: fetchTraceHiddenFunctionIds,
    staleTime: 5 * 60_000,
    retry: 1,
  })
  const producerHiddenRef = useRef<ReadonlySet<string>>(EMPTY_HIDDEN_IDS)
  producerHiddenRef.current = producerHidden ?? EMPTY_HIDDEN_IDS

  const [prefs, setPrefs] = useState<SpanFilterPrefs>(EMPTY_SPAN_FILTER_PREFS)

  // Hydrate once from the server value; after that, local toggles win
  // over refetches (last-write-wins is fine for a single-operator tool).
  const hydrated = useRef(false)
  useEffect(() => {
    if (hydrated.current || data == null) return
    hydrated.current = true
    setPrefs(parseSpanFilters(data))
  }, [data])

  const persist = useCallback(
    async (next: SpanFilterPrefs) => {
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
    const timer = setTimeout(() => void persist(prefs), SAVE_DEBOUNCE_MS)
    return () => clearTimeout(timer)
  }, [prefs, persist])

  const toggleGroup = useCallback((key: string) => {
    dirty.current = true
    setPrefs((prev) => {
      const defaults = producerHiddenRef.current
      const hiddenGroups = new Set(prev.hiddenGroups)
      const shownGroups = new Set(prev.shownGroups)
      const effectivelyHidden =
        hiddenGroups.has(key) || (defaults.has(key) && !shownGroups.has(key))
      if (effectivelyHidden) {
        // Unhide: drop any direct hide; a producer default additionally
        // needs a persistent "shown" override.
        hiddenGroups.delete(key)
        if (defaults.has(key)) shownGroups.add(key)
      } else {
        hiddenGroups.add(key)
        shownGroups.delete(key)
      }
      return { ...prev, hiddenGroups, shownGroups }
    })
  }, [])

  const toggleWorker = useCallback((key: string) => {
    dirty.current = true
    setPrefs((prev) => {
      const hiddenWorkers = new Set(prev.hiddenWorkers)
      if (!hiddenWorkers.delete(key)) hiddenWorkers.add(key)
      return { ...prev, hiddenWorkers }
    })
  }, [])

  // Internal families (`iii.tag.hidden`) are hidden by DEFAULT; the pref
  // records the ones the user revealed.
  const toggleInternal = useCallback((family: string) => {
    dirty.current = true
    setPrefs((prev) => {
      const shownInternal = new Set(prev.shownInternal)
      if (!shownInternal.delete(family)) shownInternal.add(family)
      return { ...prev, shownInternal }
    })
  }, [])

  // "show all": also overrides every producer default and reveals the
  // internal families currently in view (passed by the menu — families
  // derive from spans, the hook cannot know them), so the trace really
  // does show everything until the user hides things again.
  const clear = useCallback((visibleInternal?: readonly string[]) => {
    dirty.current = true
    setPrefs((prev) => ({
      hiddenGroups: new Set(),
      hiddenWorkers: new Set(),
      shownGroups: new Set(producerHiddenRef.current),
      shownInternal: new Set([
        ...prev.shownInternal,
        ...(visibleInternal ?? []),
      ]),
    }))
  }, [])

  const selection = useMemo(
    () => effectiveSpanFilters(prefs, producerHidden ?? EMPTY_HIDDEN_IDS),
    [prefs, producerHidden],
  )

  // Stable identity so consumers can hang memos off the controls object —
  // it only changes when the selection itself does.
  return useMemo(
    () => ({ ...selection, toggleGroup, toggleWorker, toggleInternal, clear }),
    [selection, toggleGroup, toggleWorker, toggleInternal, clear],
  )
}
