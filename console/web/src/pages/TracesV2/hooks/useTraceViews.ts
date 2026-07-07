// Server-persisted saved views for the TRACES tab.
//
// Views live in the engine's `console` configuration entry under
// `traces.views` (see `lib/tracesViews.ts` for the model). Every mutation is
// a read-modify-write of the WHOLE entry value — `configuration::set` has no
// partial-update surface — serialized through React Query so concurrent
// saves from this tab don't interleave. Cross-tab concurrency stays
// last-write-wins.
//
// `available: false` means the configuration worker (or the `console` entry)
// isn't reachable; callers hide the views UI instead of erroring.

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useCallback, useState } from 'react'
import {
  type ConsoleConfigValue,
  fetchConsoleConfigValue,
  setConsoleConfigValue,
} from '@/lib/console-config'
import { loadActiveTracesViewId, saveActiveTracesViewId } from '@/lib/storage'
import {
  newViewId,
  parseViews,
  type TracesView,
  type TracesViewConfig,
  withViews,
} from '../lib/tracesViews'

const CONSOLE_CONFIG_QUERY_KEY = ['consoleConfig']

export interface UseTraceViewsReturn {
  views: TracesView[]
  isLoading: boolean
  /** False when the configuration worker / `console` entry is unreachable. */
  available: boolean
  activeViewId: string | null
  setActiveViewId: (id: string | null) => void
  saveView: (name: string, config: TracesViewConfig) => Promise<TracesView>
  updateView: (id: string, config: TracesViewConfig) => Promise<void>
  renameView: (id: string, name: string) => Promise<void>
  deleteView: (id: string) => Promise<void>
}

export function useTraceViews(): UseTraceViewsReturn {
  const qc = useQueryClient()
  const [activeViewId, setActiveViewIdState] = useState<string | null>(() =>
    loadActiveTracesViewId(),
  )

  const { data, isLoading } = useQuery<ConsoleConfigValue | null>({
    queryKey: CONSOLE_CONFIG_QUERY_KEY,
    queryFn: fetchConsoleConfigValue,
    staleTime: 30_000,
    retry: 1,
  })

  const available = data !== null && data !== undefined
  const views = available ? parseViews(data) : []

  const setActiveViewId = useCallback((id: string | null) => {
    setActiveViewIdState(id)
    saveActiveTracesViewId(id)
  }, [])

  // One mutation funnel: take the freshest server value, transform the views
  // array, write the whole entry back, then refresh the cache.
  const mutation = useMutation({
    mutationFn: async (transform: (views: TracesView[]) => TracesView[]) => {
      const current = (await fetchConsoleConfigValue()) ?? {}
      const next = withViews(current, transform(parseViews(current)))
      await setConsoleConfigValue(next)
      return next
    },
    onSuccess: (next) => {
      qc.setQueryData(CONSOLE_CONFIG_QUERY_KEY, next)
    },
  })

  const saveView = useCallback(
    async (name: string, config: TracesViewConfig): Promise<TracesView> => {
      const view: TracesView = { id: newViewId(), name, ...config }
      await mutation.mutateAsync((existing) => [...existing, view])
      return view
    },
    [mutation],
  )

  const updateView = useCallback(
    async (id: string, config: TracesViewConfig): Promise<void> => {
      await mutation.mutateAsync((existing) =>
        existing.map((v) =>
          v.id === id ? { ...v, ...config, id, name: v.name } : v,
        ),
      )
    },
    [mutation],
  )

  const renameView = useCallback(
    async (id: string, name: string): Promise<void> => {
      await mutation.mutateAsync((existing) =>
        existing.map((v) => (v.id === id ? { ...v, name } : v)),
      )
    },
    [mutation],
  )

  const deleteView = useCallback(
    async (id: string): Promise<void> => {
      await mutation.mutateAsync((existing) =>
        existing.filter((v) => v.id !== id),
      )
    },
    [mutation],
  )

  return {
    views,
    isLoading,
    available,
    activeViewId,
    setActiveViewId,
    saveView,
    updateView,
    renameView,
    deleteView,
  }
}
