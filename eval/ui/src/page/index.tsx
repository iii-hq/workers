import { useCallback, useEffect, useMemo, useState } from 'react'
import { EmptyState, Skeleton, StatusPanel, type Host } from '@iii-dev/console-ui'
import { createEvalApi, errorMessage, isTerminal } from '../api'
import { useEvalCompleted } from '../events'
import type { CatalogModel, EvalRequest, EvalResultResponse, EvalStatusResponse, EvalSummary } from '../types'
import { EvaluationDetail } from './EvaluationDetail'
import { History } from './History'
import { NewEvaluationForm } from './NewEvaluationForm'
import { SessionComparison } from './SessionComparison'

export function EvalPage({ host }: { host: Host }) {
  const api = useMemo(() => createEvalApi(host), [host])
  const [surface, setSurface] = useState<'sessions' | 'experiments'>('sessions')
  const [evaluations, setEvaluations] = useState<EvalSummary[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)
  const [historyLoading, setHistoryLoading] = useState(false)
  const [historyError, setHistoryError] = useState<string | null>(null)
  const [models, setModels] = useState<CatalogModel[]>([])
  const [modelCatalogError, setModelCatalogError] = useState<string | null>(null)
  const [status, setStatus] = useState<EvalStatusResponse | null>(null)
  const [result, setResult] = useState<EvalResultResponse | null>(null)
  const [detailLoading, setDetailLoading] = useState(false)
  const [detailError, setDetailError] = useState<string | null>(null)
  const [actionPending, setActionPending] = useState(false)

  const loadHistory = useCallback(async () => {
    setHistoryLoading(true)
    try {
      const next = await api.list()
      setEvaluations(next)
      setHistoryError(null)
      setSelectedId((current) => {
        if (current && next.some((item) => item.evaluation_id === current)) {
          return current
        }
        return next[0]?.evaluation_id ?? null
      })
      if (next.length === 0) setCreating(true)
    } catch (error) {
      setHistoryError(errorMessage(error))
    } finally {
      setHistoryLoading(false)
    }
  }, [api])

  const loadDetail = useCallback(
    async (evaluationId: string, quiet = false) => {
      if (!quiet) setDetailLoading(true)
      try {
        const [nextStatus, nextResult] = await Promise.all([api.status(evaluationId), api.result(evaluationId)])
        setStatus(nextStatus)
        setResult(nextResult)
        setDetailError(null)
        if (nextStatus) {
          setEvaluations((current) =>
            current.map((item) =>
              item.evaluation_id === evaluationId
                ? {
                    ...item,
                    status: nextStatus.status,
                    total_runs: nextStatus.total_runs,
                    terminal_runs: nextStatus.terminal_runs,
                    passed_runs: nextStatus.passed_runs,
                    failed_runs: nextStatus.failed_runs,
                    updated_at: nextStatus.updated_at,
                    completed_at: nextStatus.completed_at,
                    error: nextStatus.error,
                    eligible: nextResult?.report?.eligible ?? item.eligible,
                  }
                : item,
            ),
          )
        }
      } catch (error) {
        setDetailError(errorMessage(error))
      } finally {
        if (!quiet) setDetailLoading(false)
      }
    },
    [api],
  )

  useEffect(() => {
    if (surface !== 'experiments') return
    void loadHistory()
    api
      .models()
      .then((catalog) => {
        setModels(catalog)
        setModelCatalogError(null)
      })
      .catch((error: unknown) => {
        setModels([])
        setModelCatalogError(errorMessage(error))
      })
  }, [api, loadHistory, surface])

  useEffect(() => {
    if (surface !== 'experiments') return
    if (!selectedId || creating) {
      setStatus(null)
      setResult(null)
      return
    }
    void loadDetail(selectedId)
  }, [creating, loadDetail, selectedId, surface])

  useEffect(() => {
    if (surface !== 'experiments') return
    if (!selectedId || creating || !status || isTerminal(status.status)) return
    const timer = window.setInterval(() => {
      void loadDetail(selectedId, true)
    }, 2_000)
    return () => window.clearInterval(timer)
  }, [creating, loadDetail, selectedId, status, surface])

  const onCompleted = useCallback(
    (event: { evaluation_id: string }) => {
      if (surface !== 'experiments') return
      void loadHistory()
      if (event.evaluation_id === selectedId) {
        void loadDetail(event.evaluation_id, true)
      }
    },
    [loadDetail, loadHistory, selectedId, surface],
  )
  useEvalCompleted(host, onCompleted)

  const create = useCallback(
    async (request: EvalRequest) => {
      const started = await api.start(request)
      setCreating(false)
      setSelectedId(started.evaluation_id)
      await loadHistory()
      await loadDetail(started.evaluation_id)
    },
    [api, loadDetail, loadHistory],
  )

  const cancel = useCallback(async () => {
    if (!selectedId) return
    setActionPending(true)
    try {
      await api.cancel(selectedId)
      await Promise.all([loadHistory(), loadDetail(selectedId)])
    } catch (error) {
      setDetailError(errorMessage(error))
    } finally {
      setActionPending(false)
    }
  }, [api, loadDetail, loadHistory, selectedId])

  const remove = useCallback(async () => {
    if (!selectedId) return
    if (!window.confirm('Delete this evaluation and its stored report?')) return
    setActionPending(true)
    try {
      await api.delete(selectedId)
      setSelectedId(null)
      setStatus(null)
      setResult(null)
      await loadHistory()
    } catch (error) {
      setDetailError(errorMessage(error))
    } finally {
      setActionPending(false)
    }
  }, [api, loadHistory, selectedId])

  const rerun = useCallback(async (reverseOrder: boolean) => {
    if (!selectedId) return
    setActionPending(true)
    try {
      const started = await api.rerun(selectedId, reverseOrder)
      setSelectedId(started.evaluation_id)
      await loadHistory()
      await loadDetail(started.evaluation_id)
    } catch (error) {
      setDetailError(errorMessage(error))
    } finally {
      setActionPending(false)
    }
  }, [api, loadDetail, loadHistory, selectedId])

  const selected = evaluations.find((item) => item.evaluation_id === selectedId) ?? null

  return (
    <div className="eval-ui-container">
      <div className="eval-ui-tabs" role="tablist" aria-label="Eval views">
        <button className={surface === 'sessions' ? 'active' : ''} type="button" role="tab" aria-selected={surface === 'sessions'} onClick={() => setSurface('sessions')}>Sessions</button>
        <button className={surface === 'experiments' ? 'active' : ''} type="button" role="tab" aria-selected={surface === 'experiments'} onClick={() => setSurface('experiments')}>Prompt experiments</button>
      </div>
      <div className={`eval-ui${creating && surface === 'experiments' ? ' creating' : ''}${surface === 'sessions' ? ' sessions-mode' : ''}`}>
        {surface === 'experiments' ? <History
          evaluations={evaluations}
          selectedId={creating ? null : selectedId}
          loading={historyLoading}
          onSelect={(evaluationId) => {
            setCreating(false)
            setSelectedId(evaluationId)
          }}
          onNew={() => setCreating(true)}
          onRefresh={() => void loadHistory()}
        /> : null}
        <main className="eval-ui-main">
          {surface === 'sessions' ? <SessionComparison host={host} /> : null}
          {surface === 'experiments' && historyError ? (
            <StatusPanel variant="alert" headline="evaluation history unavailable" detail={historyError} />
          ) : null}
          {surface === 'experiments' && creating ? (
            <NewEvaluationForm
              models={models}
              modelCatalogError={modelCatalogError}
              onLoadDefaultSystemPrompt={(provider) => api.systemPrompt(provider)}
              onCreate={create}
            />
          ) : surface === 'experiments' && selected ? (
            <EvaluationDetail
              summary={selected}
              status={status}
              result={result}
              loading={detailLoading}
              error={detailError}
              actionPending={actionPending}
              onCancel={() => void cancel()}
              onDelete={() => void remove()}
              onRerun={(reverseOrder) => void rerun(reverseOrder)}
            />
          ) : surface === 'experiments' && historyLoading ? (
            <div className="eval-ui-loading">
              <Skeleton />
              <Skeleton />
              <Skeleton />
            </div>
          ) : surface === 'experiments' ? (
            <EmptyState
              title="select an evaluation"
              description="choose a recent report or start a new prompt comparison."
              action={{ label: 'new evaluation', onClick: () => setCreating(true) }}
            />
          ) : null}
        </main>
      </div>
    </div>
  )
}
