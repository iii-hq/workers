import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import { errText } from './errors.js'
import { shouldAutoCollectGithubSources } from './security-dashboard.js'
import {
  readReconciliation,
  type SecurityReconciliation,
} from './security-scan-data'
import { shouldReloadReconciliation } from './view-state.js'

const RECONCILIATION_PAGE_SIZE = 50

export interface SecurityReconciliationState {
  data: SecurityReconciliation | null
  loading: boolean
  refreshing: boolean
  loadingMore: boolean
  error: string | null
  refresh(): void
  loadMore(): void
}

export function useSecurityReconciliation(
  host: Host,
  runId: string | null,
  refreshRevision = 0,
): SecurityReconciliationState {
  const [data, setData] = useState<SecurityReconciliation | null>(null)
  const [loading, setLoading] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  const [error, setError] = useState<{ runId: string; message: string } | null>(
    null,
  )
  const runIdRef = useRef(runId)
  const dataRef = useRef(data)
  const requestEpochRef = useRef(0)
  const loadMorePendingRef = useRef(false)
  const autoRefreshAttemptedRef = useRef(new Set<string>())
  const refreshRevisionRef = useRef(refreshRevision)
  const loadFirstPageRef = useRef<(refresh: boolean) => Promise<void>>(
    async () => {},
  )
  runIdRef.current = runId
  dataRef.current = data

  const loadFirstPage = useCallback(
    async (refresh: boolean) => {
      const requestRunId = runIdRef.current
      if (!requestRunId) return
      const requestEpoch = ++requestEpochRef.current
      if (refresh) setRefreshing(true)
      else setLoading(true)
      setError(null)
      try {
        const next = await readReconciliation(host, requestRunId, {
          refresh,
          lifecycle: 'open',
          limit: RECONCILIATION_PAGE_SIZE,
        })
        if (
          requestRunId !== runIdRef.current ||
          requestEpoch !== requestEpochRef.current
        )
          return
        setData(next)
        const shouldCollect =
          !refresh &&
          shouldAutoCollectGithubSources(
            next.sources,
            autoRefreshAttemptedRef.current.has(requestRunId),
          )
        if (shouldCollect) {
          autoRefreshAttemptedRef.current.add(requestRunId)
          queueMicrotask(() => {
            if (requestRunId === runIdRef.current)
              void loadFirstPageRef.current(true)
          })
        }
      } catch (error) {
        if (
          requestRunId !== runIdRef.current ||
          requestEpoch !== requestEpochRef.current
        )
          return
        setError({ runId: requestRunId, message: errText(error) })
      } finally {
        if (
          requestRunId === runIdRef.current &&
          requestEpoch === requestEpochRef.current
        ) {
          setLoading(false)
          setRefreshing(false)
        }
      }
    },
    [host],
  )
  loadFirstPageRef.current = loadFirstPage

  useEffect(() => {
    refreshRevisionRef.current = refreshRevision
    requestEpochRef.current += 1
    loadMorePendingRef.current = false
    setData(null)
    setError(null)
    setRefreshing(false)
    setLoadingMore(false)
    if (!runId) {
      setLoading(false)
      return
    }
    void loadFirstPage(false)
  }, [loadFirstPage, runId])

  useEffect(() => {
    const previousRevision = refreshRevisionRef.current
    refreshRevisionRef.current = refreshRevision
    if (!shouldReloadReconciliation(previousRevision, refreshRevision, runId))
      return
    void loadFirstPage(false)
  }, [loadFirstPage, refreshRevision, runId])

  const refresh = useCallback(() => {
    void loadFirstPage(true)
  }, [loadFirstPage])

  const loadMore = useCallback(() => {
    const requestRunId = runIdRef.current
    const current = dataRef.current
    if (!requestRunId || !current?.next_cursor || loadMorePendingRef.current)
      return
    const requestEpoch = requestEpochRef.current
    const cursor = current.next_cursor
    loadMorePendingRef.current = true
    setLoadingMore(true)
    setError(null)
    void readReconciliation(host, requestRunId, {
      lifecycle: 'open',
      cursor,
      limit: RECONCILIATION_PAGE_SIZE,
    })
      .then((next) => {
        if (
          requestRunId !== runIdRef.current ||
          requestEpoch !== requestEpochRef.current
        )
          return
        setData((existing) => {
          if (!existing || existing.run_id !== requestRunId) return next
          const seen = new Set(
            existing.records.map(
              (record) => `${record.source}:${record.number}`,
            ),
          )
          const records = [
            ...existing.records,
            ...next.records.filter(
              (record) => !seen.has(`${record.source}:${record.number}`),
            ),
          ]
          return { ...next, records }
        })
      })
      .catch((error) => {
        if (
          requestRunId === runIdRef.current &&
          requestEpoch === requestEpochRef.current
        ) {
          setError({ runId: requestRunId, message: errText(error) })
        }
      })
      .finally(() => {
        loadMorePendingRef.current = false
        if (
          requestRunId === runIdRef.current &&
          requestEpoch === requestEpochRef.current
        ) {
          setLoadingMore(false)
        }
      })
  }, [host])

  const dataIsCurrent = data?.run_id === runId
  const errorIsCurrent = error?.runId === runId
  return {
    data: dataIsCurrent ? data : null,
    loading: runId && !dataIsCurrent && !errorIsCurrent ? true : loading,
    refreshing: dataIsCurrent ? refreshing : false,
    loadingMore: dataIsCurrent ? loadingMore : false,
    error: errorIsCurrent ? error.message : null,
    refresh,
    loadMore,
  }
}
