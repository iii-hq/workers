import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useWorkerLive } from '@iii-dev/console-ui/hooks'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  listRuns,
  type RetryResult,
  RUN_STATUSES,
  type RunFilters,
  type RunStatus,
  type RunSummary,
  readRun,
  requestRunMode,
  retryRun,
  type SecurityRun,
} from './security-scan-data'
import { isRepositoryScopeCurrent, isStreamLive } from './view-state.js'

/** The hook's handler; the runs stream below is bound to `${HANDLER_ID}::<browserId>`. */
const HANDLER_ID = 'iii::security-scan-ui::runs'
const RUN_STREAM = { stream_name: 'security-scan:runs', group_id: 'all' }
const NO_RUNS: RunSummary[] = []

export interface SecurityRunsLive {
  runs: RunSummary[]
  totalRuns: number
  statusCounts: Record<RunStatus, number>
  detail: SecurityRun | null
  loading: boolean
  detailLoading: boolean
  refreshing: boolean
  live: boolean
  listError: string | null
  detailError: string | null
  reconciliationRefreshRevision: number
  refresh(): void
  retry(run: RunSummary | SecurityRun): Promise<RetryResult>
  requestSuggestions(run: RunSummary | SecurityRun): Promise<RetryResult>
}

interface RepositoryRunList {
  repositoryKey: string
  runs: RunSummary[]
}

interface DetailState {
  runId: string | null
  run: SecurityRun | null
  error: string | null
}

export function useSecurityRunsLive(host: Host, filters: RunFilters, selectedId: string | null): SecurityRunsLive {
  const repositoryKey = filters.repository.trim()
  const {
    data,
    loading: fetching,
    error: listError,
    refresh,
    live: streamBound,
  } = useWorkerLive<RepositoryRunList>({
    iii: host.iii,
    // The runs feed is a `stream` trigger; the hook binds it to its handler
    // and polls only while the binding is missing.
    triggers: [{ type: 'stream', config: RUN_STREAM }],
    fetch: async () => ({
      repositoryKey,
      runs: await listRuns(host, { repository: repositoryKey, status: '' }),
    }),
    handlerId: HANDLER_ID,
  })

  // The hook fetches on mount by itself; a repository scope change refetches.
  const fetchedKeyRef = useRef(repositoryKey)
  useEffect(() => {
    if (fetchedKeyRef.current === repositoryKey) return
    fetchedKeyRef.current = repositoryKey
    refresh()
  }, [refresh, repositoryKey])

  const [connectionState, setConnectionState] = useState<unknown>('disconnected')
  useEffect(() => {
    try {
      return host.iii.addConnectionStateListener((state) => {
        setConnectionState(state)
        if (state === 'connected') refresh()
      })
    } catch {
      setConnectionState('disconnected')
    }
  }, [host, refresh])

  // The selected run: re-read on selection and whenever the list arrives
  // (stream event, poll, manual refresh).
  const [detail, setDetail] = useState<DetailState>({ runId: null, run: null, error: null })
  const [detailFetching, setDetailFetching] = useState(false)
  useEffect(() => {
    if (!selectedId) return
    let stale = false
    setDetailFetching(true)
    readRun(host, selectedId)
      .then(
        (run) => {
          if (stale) return
          setDetail({ runId: selectedId, run, error: run ? null : 'This run no longer exists.' })
        },
        (error: unknown) => {
          if (!stale) setDetail({ runId: selectedId, run: null, error: errorMessage(error) })
        },
      )
      .finally(() => {
        if (!stale) setDetailFetching(false)
      })
    return () => {
      stale = true
    }
  }, [data, host, selectedId])

  // Every list arrival is a reconciliation refresh signal.
  const [reconciliationRefreshRevision, setReconciliationRefreshRevision] = useState(0)
  useEffect(() => {
    setReconciliationRefreshRevision((current) => current + 1)
  }, [data])

  const scoped = data && isRepositoryScopeCurrent(repositoryKey, data.repositoryKey) ? data : null
  const allRuns = scoped?.runs ?? NO_RUNS
  const runs = useMemo(
    () => (filters.status ? allRuns.filter((run) => run.status === filters.status) : allRuns),
    [allRuns, filters.status],
  )
  const statusCounts = useMemo(() => {
    const counts = Object.fromEntries(RUN_STATUSES.map((status) => [status, 0])) as Record<RunStatus, number>
    for (const run of allRuns) counts[run.status] += 1
    return counts
  }, [allRuns])
  const detailIsCurrent = detail.runId === selectedId
  const currentDetail =
    detailIsCurrent && detail.run && (!repositoryKey || detail.run.repository === repositoryKey) ? detail.run : null

  const retry = useCallback(
    async (run: RunSummary | SecurityRun) => {
      const result = await retryRun(host, run)
      refresh()
      return result
    },
    [host, refresh],
  )

  const requestSuggestions = useCallback(
    async (run: RunSummary | SecurityRun) => {
      const result = await requestRunMode(host, run, 'suggest')
      refresh()
      return result
    },
    [host, refresh],
  )

  return {
    runs,
    totalRuns: allRuns.length,
    statusCounts,
    detail: currentDetail,
    loading: !scoped && !listError,
    detailLoading: selectedId !== null && (detailFetching || !detailIsCurrent),
    refreshing: fetching && scoped !== null,
    live: isStreamLive(streamBound, connectionState),
    listError,
    detailError: detailIsCurrent ? detail.error : null,
    reconciliationRefreshRevision,
    refresh,
    retry,
    requestSuggestions,
  }
}
