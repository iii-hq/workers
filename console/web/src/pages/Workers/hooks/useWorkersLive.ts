import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useMemo, useState } from 'react'
import { useWorkerLifecycle } from '@/hooks/use-worker-lifecycle'
import { getDefaultBackend } from '@/lib/backend'
import { composeContainerAction, stopSupervisorWorker } from '../api/workers'
import { fetchWorkersView } from '../lib/merge-workers'
import { takePendingWorkerSearch } from '../pending-selection'
import {
  type ComposeAction,
  filterWorkerRows,
  type WorkersFilterState,
} from '../types'
import { useComposeChanged } from './useComposeChanged'

export const workersKeys = {
  all: ['workers'] as const,
  runtime: () => [...workersKeys.all, 'runtime'] as const,
}

const WORKERS_RUNTIME_WATCH_FN = 'iii::console::workers_runtime'
const COMPOSE_WATCH_FN = 'iii::console::workers_compose'

const RUNTIME_OPERATIONS = [
  'add',
  'remove',
  'update',
  'start',
  'stop',
  'clear',
] as const

const RUNTIME_STAGES = ['done', 'failed'] as const

export interface PendingComposeAction {
  container: string
  action: ComposeAction
}

export function useWorkersLive() {
  const qc = useQueryClient()
  const enabled = getDefaultBackend().id === 'real'
  // A caller (the command palette) can ask for this page filtered to one
  // worker. It is consumed once, at mount, so a later visit opens unfiltered.
  const [filters, setFilters] = useState<WorkersFilterState>(() => ({
    search: takePendingWorkerSearch() ?? '',
    tag: null,
    runtime: null,
    management: null,
  }))
  const [stoppingName, setStoppingName] = useState<string | null>(null)
  const [pendingCompose, setPendingCompose] =
    useState<PendingComposeAction | null>(null)

  const query = useQuery({
    queryKey: workersKeys.runtime(),
    queryFn: fetchWorkersView,
    enabled,
  })

  const invalidate = () => {
    void qc.invalidateQueries({ queryKey: workersKeys.runtime() })
  }

  useWorkerLifecycle({
    enabled,
    fnId: WORKERS_RUNTIME_WATCH_FN,
    operations: RUNTIME_OPERATIONS,
    stages: RUNTIME_STAGES,
    onEvent: invalidate,
  })

  useComposeChanged({
    enabled,
    fnId: COMPOSE_WATCH_FN,
    onEvent: invalidate,
  })

  const rows = useMemo(() => {
    const source = query.data?.rows ?? []
    return filterWorkerRows(source, filters)
  }, [query.data, filters])

  const stopMutation = useMutation({
    mutationFn: stopSupervisorWorker,
    onMutate: (name) => {
      setStoppingName(name)
    },
    onSettled: () => {
      setStoppingName(null)
      invalidate()
    },
  })

  const composeMutation = useMutation({
    mutationFn: ({ action, container }: PendingComposeAction) =>
      composeContainerAction(action, container),
    onMutate: (pending) => {
      setPendingCompose(pending)
    },
    onSettled: () => {
      setPendingCompose(null)
      invalidate()
    },
  })

  function updateFilters(next: Partial<WorkersFilterState>) {
    setFilters((cur) => ({ ...cur, ...next }))
  }

  function clearFilters() {
    setFilters({ search: '', tag: null, runtime: null, management: null })
  }

  function stopWorker(name: string) {
    stopMutation.mutate(name)
  }

  function composeAction(action: ComposeAction, container: string) {
    composeMutation.mutate({ action, container })
  }

  return {
    rows,
    allRows: query.data?.rows ?? [],
    compose: query.data?.compose ?? null,
    filters,
    updateFilters,
    clearFilters,
    isLoading: enabled && query.isLoading,
    isError: query.isError,
    error: query.error,
    refetch: query.refetch,
    stoppingName,
    stopWorker,
    stopError: stopMutation.error,
    pendingCompose,
    composeAction,
    composeError: composeMutation.error,
    clearComposeError: composeMutation.reset,
  }
}
