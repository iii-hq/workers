import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useMemo, useState } from 'react'
import { useWorkerLifecycle } from '@/hooks/use-worker-lifecycle'
import { getDefaultBackend } from '@/lib/backend'
import { stopSupervisorWorker } from '../api/workers'
import { fetchMergedWorkers } from '../lib/merge-workers'
import {
  filterWorkerRows,
  type WorkersFilterState,
} from '../types'

export const workersKeys = {
  all: ['workers'] as const,
  runtime: () => [...workersKeys.all, 'runtime'] as const,
}

const WORKERS_RUNTIME_WATCH_FN = 'iii::console::workers_runtime'

const RUNTIME_OPERATIONS = [
  'add',
  'remove',
  'update',
  'start',
  'stop',
  'clear',
] as const

const RUNTIME_STAGES = ['done', 'failed'] as const

export function useWorkersLive() {
  const qc = useQueryClient()
  const enabled = getDefaultBackend().id === 'real'
  const [filters, setFilters] = useState<WorkersFilterState>({
    search: '',
    tag: null,
    runtime: null,
  })
  const [stoppingName, setStoppingName] = useState<string | null>(null)

  const query = useQuery({
    queryKey: workersKeys.runtime(),
    queryFn: fetchMergedWorkers,
    enabled,
  })

  useWorkerLifecycle({
    enabled,
    fnId: WORKERS_RUNTIME_WATCH_FN,
    operations: RUNTIME_OPERATIONS,
    stages: RUNTIME_STAGES,
    onEvent: () => {
      void qc.invalidateQueries({ queryKey: workersKeys.runtime() })
    },
  })

  const rows = useMemo(() => {
    const source = query.data ?? []
    return filterWorkerRows(source, filters)
  }, [query.data, filters])

  const stopMutation = useMutation({
    mutationFn: stopSupervisorWorker,
    onMutate: (name) => {
      setStoppingName(name)
    },
    onSettled: () => {
      setStoppingName(null)
      void qc.invalidateQueries({ queryKey: workersKeys.runtime() })
    },
  })

  function updateFilters(next: Partial<WorkersFilterState>) {
    setFilters((cur) => ({ ...cur, ...next }))
  }

  function clearFilters() {
    setFilters({ search: '', tag: null, runtime: null })
  }

  function stopWorker(name: string) {
    stopMutation.mutate(name)
  }

  return {
    rows,
    allRows: query.data ?? [],
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
  }
}
