import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react'
import { createRefreshGate } from './refresh-gate.js'
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

const DOORBELL_DEBOUNCE_MS = 160

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function tabIsHidden(): boolean {
  return (
    typeof document !== 'undefined' && document.visibilityState === 'hidden'
  )
}

function repositoryKey(filters: RunFilters): string {
  return filters.repository.trim()
}

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
  repositoryKey: string | null
  runs: RunSummary[]
}

export function useSecurityRunsLive(
  host: Host,
  filters: RunFilters,
  selectedId: string | null,
): SecurityRunsLive {
  const activeRepositoryKey = repositoryKey(filters)
  const [runList, setRunList] = useState<RepositoryRunList>({
    repositoryKey: null,
    runs: [],
  })
  const [detail, setDetail] = useState<SecurityRun | null>(null)
  const [detailRepositoryKey, setDetailRepositoryKey] = useState<string | null>(
    null,
  )
  const [loading, setLoading] = useState(true)
  const [detailLoading, setDetailLoading] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [streamBound, setStreamBound] = useState(false)
  const [connectionState, setConnectionState] =
    useState<unknown>('disconnected')
  const [listError, setListError] = useState<string | null>(null)
  const [detailError, setDetailError] = useState<{
    runId: string
    message: string
  } | null>(null)
  const [reconciliationRefreshRevision, setReconciliationRefreshRevision] =
    useState(0)

  const listLoadedRef = useRef(false)
  const filtersRef = useRef(filters)
  const currentRepositoryKeyRef = useRef(activeRepositoryKey)
  const selectedIdRef = useRef(selectedId)
  filtersRef.current = filters
  currentRepositoryKeyRef.current = activeRepositoryKey
  selectedIdRef.current = selectedId

  const listTaskRef = useRef<() => Promise<void>>(async () => {})
  listTaskRef.current = async () => {
    const activeFilters = filtersRef.current
    const requestKey = repositoryKey(activeFilters)
    if (listLoadedRef.current) setRefreshing(true)
    else setLoading(true)

    try {
      const next = await listRuns(host, { ...activeFilters, status: '' })
      if (requestKey !== currentRepositoryKeyRef.current) return
      setRunList({ repositoryKey: requestKey, runs: next })
      setListError(null)
      listLoadedRef.current = true
    } catch (error) {
      if (requestKey !== currentRepositoryKeyRef.current) return
      setRunList((current) =>
        current.repositoryKey === requestKey
          ? current
          : { repositoryKey: requestKey, runs: [] },
      )
      setListError(message(error))
    } finally {
      if (requestKey === currentRepositoryKeyRef.current) {
        setLoading(false)
        setRefreshing(false)
      }
    }
  }

  const listGateRef = useRef<ReturnType<typeof createRefreshGate> | null>(null)
  if (!listGateRef.current) {
    listGateRef.current = createRefreshGate(() => listTaskRef.current())
  }
  const loadList = useCallback(() => {
    void listGateRef.current?.request()
  }, [])

  const detailTaskRef = useRef<() => Promise<void>>(async () => {})
  detailTaskRef.current = async () => {
    const runId = selectedIdRef.current
    const requestKey = currentRepositoryKeyRef.current
    if (!runId) {
      setDetail(null)
      setDetailRepositoryKey(null)
      setDetailError(null)
      setDetailLoading(false)
      return
    }

    setDetailLoading(true)
    try {
      const next = await readRun(host, runId)
      if (
        runId !== selectedIdRef.current ||
        requestKey !== currentRepositoryKeyRef.current
      )
        return
      setDetailRepositoryKey(requestKey)
      if (next && requestKey && next.repository !== requestKey) {
        setDetail(null)
        setDetailError(null)
        return
      }
      setDetail(next)
      setDetailError(
        next ? null : { runId, message: 'This run no longer exists.' },
      )
    } catch (error) {
      if (
        runId !== selectedIdRef.current ||
        requestKey !== currentRepositoryKeyRef.current
      )
        return
      setDetailRepositoryKey(requestKey)
      setDetailError({ runId, message: message(error) })
    } finally {
      if (
        runId === selectedIdRef.current &&
        requestKey === currentRepositoryKeyRef.current
      ) {
        setDetailLoading(false)
      }
    }
  }

  const detailGateRef = useRef<ReturnType<typeof createRefreshGate> | null>(
    null,
  )
  if (!detailGateRef.current) {
    detailGateRef.current = createRefreshGate(() => detailTaskRef.current())
  }
  const loadDetail = useCallback(() => {
    void detailGateRef.current?.request()
  }, [])

  const refresh = useCallback(() => {
    if (tabIsHidden()) return
    setReconciliationRefreshRevision((current) => current + 1)
    loadList()
    if (selectedIdRef.current) loadDetail()
  }, [loadDetail, loadList])

  const refreshRef = useRef(refresh)
  useEffect(() => {
    refreshRef.current = refresh
  }, [refresh])

  useEffect(() => {
    listLoadedRef.current = false
    setRunList((current) => ({
      repositoryKey: current.repositoryKey,
      runs: [],
    }))
    setLoading(true)
    setRefreshing(false)
    setListError(null)
    loadList()
  }, [activeRepositoryKey, loadList])

  useEffect(() => {
    setDetail(null)
    setDetailRepositoryKey(null)
    setDetailError(null)
    if (selectedId) loadDetail()
    else setDetailLoading(false)
  }, [activeRepositoryKey, loadDetail, selectedId])

  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')
  useEffect(() => {
    const localFunctionId = `iii::security-scan-ui::runs-doorbell::${instanceId}`
    let debounceTimer: number | undefined
    const disposers: Array<() => void> = []
    setStreamBound(false)

    const scheduleRefresh = () => {
      if (tabIsHidden()) return
      if (debounceTimer !== undefined) window.clearTimeout(debounceTimer)
      debounceTimer = window.setTimeout(
        () => refreshRef.current(),
        DOORBELL_DEBOUNCE_MS,
      )
    }

    try {
      disposers.push(host.iii.on(localFunctionId, scheduleRefresh))
      disposers.push(
        host.iii.registerTrigger({
          type: 'stream',
          function_id: `${localFunctionId}::${host.iii.browserId}`,
          config: { stream_name: 'security-scan:runs', group_id: 'all' },
        }),
      )
      setStreamBound(true)
    } catch {
      for (const dispose of disposers) dispose()
      disposers.length = 0
      setStreamBound(false)
    }

    return () => {
      if (debounceTimer !== undefined) window.clearTimeout(debounceTimer)
      for (const dispose of disposers) dispose()
    }
  }, [host, instanceId])

  useEffect(() => {
    setConnectionState('disconnected')
    try {
      return host.iii.addConnectionStateListener((state) => {
        setConnectionState(state)
        if (state === 'connected') refreshRef.current()
      })
    } catch {
      setConnectionState('disconnected')
      return undefined
    }
  }, [host])

  useEffect(() => {
    if (typeof document === 'undefined') return
    const onVisibilityChange = () => {
      if (document.visibilityState === 'visible') refreshRef.current()
    }
    document.addEventListener('visibilitychange', onVisibilityChange)
    return () =>
      document.removeEventListener('visibilitychange', onVisibilityChange)
  }, [])

  const listScopeIsCurrent = isRepositoryScopeCurrent(
    activeRepositoryKey,
    runList.repositoryKey,
  )
  const allRuns = useMemo(
    () => (listScopeIsCurrent ? runList.runs : []),
    [listScopeIsCurrent, runList.runs],
  )
  const runs = useMemo(
    () =>
      filters.status
        ? allRuns.filter((run) => run.status === filters.status)
        : allRuns,
    [allRuns, filters.status],
  )
  const statusCounts = useMemo(() => {
    const counts = Object.fromEntries(
      RUN_STATUSES.map((status) => [status, 0]),
    ) as Record<RunStatus, number>
    for (const run of allRuns) counts[run.status] += 1
    return counts
  }, [allRuns])
  const live = isStreamLive(streamBound, connectionState)
  const detailScopeIsCurrent = isRepositoryScopeCurrent(
    activeRepositoryKey,
    detailRepositoryKey,
  )
  const currentDetail =
    detailScopeIsCurrent &&
    listScopeIsCurrent &&
    detail?.run_id === selectedId &&
    (!activeRepositoryKey || detail.repository === activeRepositoryKey)
      ? detail
      : null

  const retry = useCallback(
    async (run: RunSummary | SecurityRun) => {
      const runId = await retryRun(host, run)
      refreshRef.current()
      return runId
    },
    [host],
  )

  const requestSuggestions = useCallback(
    async (run: RunSummary | SecurityRun) => {
      const result = await requestRunMode(host, run, 'suggest')
      refreshRef.current()
      return result
    },
    [host],
  )

  return {
    runs,
    totalRuns: allRuns.length,
    statusCounts,
    detail: currentDetail,
    loading: loading || !listScopeIsCurrent,
    detailLoading,
    refreshing,
    live,
    listError: listScopeIsCurrent ? listError : null,
    detailError:
      listScopeIsCurrent &&
      detailScopeIsCurrent &&
      detailError?.runId === selectedId
        ? detailError.message
        : null,
    reconciliationRefreshRevision,
    refresh,
    retry,
    requestSuggestions,
  }
}
