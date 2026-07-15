// Server-persisted saved views for the TRACES tab.
//
// Views live in the engine's `console` configuration entry under
// `traces.views`, and the ACTIVE view pointer next to them under
// `traces.activeViewId` (see `lib/tracesViews.ts` for the model), so the
// selection follows the engine — not the browser — and a fresh config
// defaults to the seeded sessions view. Every mutation is a
// read-modify-write of the WHOLE entry value — `configuration::set` has no
// partial-update surface. Cross-tab concurrency stays last-write-wins.
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
import {
  DEFAULT_VIEW_ID,
  newViewId,
  parseActiveViewId,
  parseViews,
  type TracesView,
  type TracesViewConfig,
  withActiveViewId,
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

  const { data, isLoading } = useQuery<ConsoleConfigValue | null>({
    queryKey: CONSOLE_CONFIG_QUERY_KEY,
    queryFn: fetchConsoleConfigValue,
    staleTime: 30_000,
    retry: 1,
  })

  const available = data !== null && data !== undefined
  const views = available ? parseViews(data) : []

  // Selection made in THIS tab; once set it wins over the server pointer so
  // a slow write (or an unreachable configuration worker) never lags the
  // dropdown. `undefined` = no local choice yet.
  const [chosenViewId, setChosenViewId] = useState<string | null | undefined>(
    undefined,
  )
  const storedViewId = available ? parseActiveViewId(data) : undefined
  // No pointer recorded (fresh engine): default to the seeded sessions
  // view. TracesV2's initial-apply effect clears the selection if that
  // view doesn't exist on this engine.
  const serverViewId =
    storedViewId === undefined && available ? DEFAULT_VIEW_ID : storedViewId
  const activeViewId =
    chosenViewId === undefined ? (serverViewId ?? null) : chosenViewId

  // One mutation funnel: take the freshest server value, transform the whole
  // entry value, write it back, then refresh the cache.
  const mutation = useMutation({
    mutationFn: async (
      transform: (value: ConsoleConfigValue) => ConsoleConfigValue,
    ) => {
      const current = (await fetchConsoleConfigValue()) ?? {}
      const next = transform(current)
      await setConsoleConfigValue(next)
      return next
    },
    onSuccess: (next) => {
      qc.setQueryData(CONSOLE_CONFIG_QUERY_KEY, next)
    },
  })

  const setActiveViewId = useCallback(
    (id: string | null) => {
      setChosenViewId(id)
      // Best-effort server persist; the in-memory selection stays live even
      // when the configuration worker is unreachable.
      mutation
        .mutateAsync((value) => withActiveViewId(value, id))
        .catch(() => {})
    },
    [mutation],
  )

  const mutateViews = useCallback(
    (transform: (views: TracesView[]) => TracesView[]) =>
      mutation.mutateAsync((value) =>
        withViews(value, transform(parseViews(value))),
      ),
    [mutation],
  )

  const saveView = useCallback(
    async (name: string, config: TracesViewConfig): Promise<TracesView> => {
      const view: TracesView = { id: newViewId(), name, ...config }
      await mutateViews((existing) => [...existing, view])
      return view
    },
    [mutateViews],
  )

  const updateView = useCallback(
    async (id: string, config: TracesViewConfig): Promise<void> => {
      await mutateViews((existing) =>
        existing.map((v) =>
          v.id === id ? { ...v, ...config, id, name: v.name } : v,
        ),
      )
    },
    [mutateViews],
  )

  const renameView = useCallback(
    async (id: string, name: string): Promise<void> => {
      await mutateViews((existing) =>
        existing.map((v) => (v.id === id ? { ...v, name } : v)),
      )
    },
    [mutateViews],
  )

  const deleteView = useCallback(
    async (id: string): Promise<void> => {
      // Deleting the selected view also clears the selection — folded into
      // the SAME write so the two updates can't interleave and the pointer
      // never dangles. The absent-pointer case resolves to the default view
      // first, so deleting the seeded view on a fresh engine records an
      // explicit "all traces" too.
      if (id === activeViewId) setChosenViewId(null)
      await mutation.mutateAsync((value) => {
        const next = withViews(
          value,
          parseViews(value).filter((v) => v.id !== id),
        )
        const pointer = parseActiveViewId(value)
        const effective = pointer === undefined ? DEFAULT_VIEW_ID : pointer
        return effective === id ? withActiveViewId(next, null) : next
      })
    },
    [mutation, activeViewId],
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
