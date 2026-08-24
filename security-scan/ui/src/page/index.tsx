import {
  Button,
  EmptyState,
  type Host,
  Input,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  Select,
  StatusDot,
} from '@iii-dev/console-ui'
import { type ComponentType, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { errText } from './errors.js'
import { AlertIcon, ChatIcon, RefreshIcon, SearchIcon, SettingsIcon, ShieldIcon } from './icons'
import { ScanRequestForm } from './ScanRequestForm'
import { buildStatusOptions } from './security-dashboard.js'
import { SecurityRunDetail } from './SecurityRunDetail'
import {
  formatRelativeTime,
  formatStatus,
  formatTimestamp,
  ensureAnalysisConversation,
  isSafeGitHubHttpsUrl,
  isTerminal,
  listAnalysisConversations,
  loadComposerModel,
  loadScanFormDefaults,
  normalizeCommitSha,
  openAnalysisConversation,
  RUN_STATUSES,
  type RunFilters,
  type RunStatus,
  type RunSummary,
  cancelRun,
  canCancelRun,
  commitScopeLabel,
} from './security-scan-data'
import { useFollowAnalysisChat } from './useFollowAnalysisChat'
import { useSecurityActions } from './useSecurityActions'
import { useSecurityReconciliation } from './useSecurityReconciliation'
import { useSecurityRunsLive } from './useSecurityRunsLive'
import { automaticFocusTarget, beginRetry, scanHistoryDescription, settleRetry } from './view-state.js'

const NARROW_BELOW = 760
/** Where a console without the configuration-dialog export sends the operator. */
const CONFIG_HASH = '#/workers/configuration/security-scan'

type RetryStates = Record<string, { pending: boolean; error: string | null }>
type FocusTarget = { kind: 'run'; runId: string } | { kind: 'filter' }
// Console Select reserves the empty string for its placeholder state.
type StatusOptionValue = RunStatus | 'all'
type SuggestionState = {
  runId: string
  pending: boolean
  error: string | null
  message: string | null
}

function classNames(...values: Array<string | false | null | undefined>): string {
  return values.filter(Boolean).join(' ')
}

function useContainerNarrow(threshold: number): [(node: HTMLDivElement | null) => void, boolean] {
  const [narrow, setNarrow] = useState(false)
  const observerRef = useRef<ResizeObserver | null>(null)

  const ref = useCallback(
    (node: HTMLDivElement | null) => {
      observerRef.current?.disconnect()
      observerRef.current = null
      if (!node) return
      const width = node.getBoundingClientRect().width
      if (width > 0) setNarrow(width < threshold)
      const observer = new ResizeObserver((entries) => {
        const next = entries[0]?.contentRect.width
        if (typeof next === 'number' && next > 0) setNarrow(next < threshold)
      })
      observer.observe(node)
      observerRef.current = observer
    },
    [threshold],
  )

  return [ref, narrow]
}

function statusTone(status: RunStatus): 'accent' | 'alert' | 'warn' | 'ink' {
  if (status === 'failed') return 'alert'
  if (status === 'cancelling') return 'warn'
  if (status === 'completed') return 'accent'
  return status === 'cancelled' ? 'ink' : 'accent'
}

function findingLabel(count: number): string {
  return `${count} ${count === 1 ? 'finding' : 'findings'}`
}

function RunListRow({
  run,
  selected,
  cancelling,
  analysisSessionId,
  onSelect,
  onCancel,
  onOpenChat,
  buttonRef,
}: {
  run: RunSummary
  selected: boolean
  cancelling: boolean
  analysisSessionId?: string
  onSelect(): void
  onCancel(): void
  onOpenChat(): void
  buttonRef(node: HTMLButtonElement | null): void
}) {
  const stoppable = canCancelRun(run.status)
  return (
    <li className="security-scan-ui-run-item">
      <button
        type="button"
        className={classNames('security-scan-ui-run', selected && 'is-selected')}
        aria-current={selected ? 'true' : undefined}
        aria-label={`${run.repository} ${commitScopeLabel(run)}, ${formatStatus(run.status)}, ${findingLabel(run.finding_count)}`}
        data-run-id={run.run_id}
        ref={buttonRef}
        onClick={onSelect}
      >
        <span className="security-scan-ui-run-topline">
          <span className="security-scan-ui-run-repo" title={run.repository}>
            {run.repository}
          </span>
          <StatusDot tone={statusTone(run.status)} pulse={!isTerminal(run.status)} title={formatStatus(run.status)} />
        </span>
        <span className="security-scan-ui-run-context">
          <code>{commitScopeLabel(run)}</code>
          {run.model ? (
            <>
              <span aria-hidden="true">·</span>
              <span title={run.model}>{run.model}</span>
            </>
          ) : null}
        </span>
        <span className="security-scan-ui-run-meta">
          <span title={formatTimestamp(run.updated_at)}>{formatRelativeTime(run.updated_at)}</span>
          <span className="security-scan-ui-run-result">
            <span className="security-scan-ui-run-status">{formatStatus(run.status)}</span>
            <span className="security-scan-ui-run-count">{findingLabel(run.finding_count)}</span>
          </span>
        </span>
      </button>
      {analysisSessionId || stoppable || run.status === 'cancelling' ? (
        <div className="security-scan-ui-run-actions">
          {analysisSessionId ? (
            <Button
              variant="ghost"
              size="sm"
              className="security-scan-ui-run-chat"
              aria-label={`Open ${run.repository} analysis chat`}
              title="Open analysis chat"
              onClick={(event) => {
                event.preventDefault()
                event.stopPropagation()
                onOpenChat()
              }}
            >
              <ChatIcon size={14} />
            </Button>
          ) : null}
          {stoppable || run.status === 'cancelling' ? (
            <Button
              variant="ghost"
              size="sm"
              disabled={cancelling || run.status === 'cancelling'}
              aria-label={`Stop ${run.repository} scan`}
              onClick={(event) => {
                event.preventDefault()
                event.stopPropagation()
                onCancel()
              }}
            >
              {cancelling || run.status === 'cancelling' ? 'stopping' : 'stop'}
            </Button>
          ) : null}
        </div>
      ) : null}
    </li>
  )
}

const EmptyShield = () => <ShieldIcon size={28} />

export function SecurityScanPage({
  host,
  panelSide = 'left',
  onRequestClose,
  conversationId,
}: { host: Host } & Partial<PageRenderProps>) {
  const [filters, setFilters] = useState<RunFilters>({
    repository: '',
    status: '',
  })
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [narrowDetailOpen, setNarrowDetailOpen] = useState(false)
  const [retryStates, setRetryStates] = useState<RetryStates>({})
  const [suggestionState, setSuggestionState] = useState<SuggestionState | null>(null)
  const [pendingSuggestionRunId, setPendingSuggestionRunId] = useState<string | null>(null)
  const [pendingNewRunId, setPendingNewRunId] = useState<string | null>(null)
  const [followRunId, setFollowRunId] = useState<string | null>(null)
  const [followStartConversationId, setFollowStartConversationId] = useState<string | null>(null)
  const [cancelStates, setCancelStates] = useState<RetryStates>({})
  const [analysisSessionIds, setAnalysisSessionIds] = useState<Record<string, string>>({})
  const [bodyRef, narrow] = useContainerNarrow(NARROW_BELOW)
  const detailBackRef = useRef<HTMLButtonElement | null>(null)
  const repositoryFilterRef = useRef<HTMLInputElement | null>(null)
  const runButtonRefs = useRef(new Map<string, HTMLButtonElement>())
  const restoreFocusTargetRef = useRef<FocusTarget | null>(null)

  const {
    runs,
    totalRuns,
    statusCounts,
    detail,
    loading,
    detailLoading,
    refreshing,
    live,
    listError,
    detailError,
    reconciliationRefreshRevision,
    refresh,
    retry,
    requestSuggestions,
  } = useSecurityRunsLive(host, filters, selectedId)
  const reconciliation = useSecurityReconciliation(host, selectedId, reconciliationRefreshRevision)
  const actions = useSecurityActions(host)
  useFollowAnalysisChat(host, followRunId, followStartConversationId, conversationId, () => setFollowRunId(null))

  const statusOptions = useMemo(
    () =>
      buildStatusOptions(RUN_STATUSES, statusCounts, totalRuns) as Array<{
        value: StatusOptionValue
        label: string
      }>,
    [statusCounts, totalRuns],
  )

  const selected = useMemo(() => runs.find((run) => run.run_id === selectedId) ?? null, [runs, selectedId])
  const runIdsSignature = useMemo(
    () =>
      runs
        .map((run) => run.run_id)
        .sort()
        .join('\u0001'),
    [runs],
  )

  const beginFollow = (runId: string) => {
    setFollowStartConversationId(conversationId?.trim() || null)
    setFollowRunId(runId)
  }

  useEffect(() => {
    if (!followRunId) return
    const followed = runs.find((run) => run.run_id === followRunId)
    if (!followed) return
    if (followed.status === 'failed' || followed.status === 'cancelled') {
      setFollowRunId(null)
    }
  }, [followRunId, runs])

  useEffect(() => {
    let cancelled = false
    void listAnalysisConversations(host)
      .then((sessions) => {
        if (cancelled) return
        setAnalysisSessionIds(Object.fromEntries(sessions.map((session) => [session.runId, session.sessionId])))
      })
      .catch(() => undefined)
    return () => {
      cancelled = true
    }
  }, [host, runIdsSignature])

  useEffect(() => {
    if (!selectedId || analysisSessionIds[selectedId]) return
    let cancelled = false
    void ensureAnalysisConversation(host, selectedId)
      .then((sessionId) => {
        if (cancelled || !sessionId) return
        setAnalysisSessionIds((current) => ({ ...current, [selectedId]: sessionId }))
      })
      .catch(() => undefined)
    return () => {
      cancelled = true
    }
  }, [analysisSessionIds, host, selected?.status, selectedId])

  useEffect(() => {
    const pendingId = pendingNewRunId ?? pendingSuggestionRunId
    if (!pendingId) return
    if (!runs.some((run) => run.run_id === pendingId)) return
    setSelectedId(pendingId)
    setPendingNewRunId(null)
    setPendingSuggestionRunId(null)
    if (narrow) setNarrowDetailOpen(true)
  }, [narrow, pendingNewRunId, pendingSuggestionRunId, runs])

  useEffect(() => {
    if (loading) return
    if (runs.length === 0) {
      if (pendingNewRunId || pendingSuggestionRunId) return
      const focusTarget = automaticFocusTarget(narrow, narrowDetailOpen, null)
      if (focusTarget) restoreFocusTargetRef.current = focusTarget
      setSelectedId(null)
      setNarrowDetailOpen(false)
      return
    }
    if (!selectedId || !runs.some((run) => run.run_id === selectedId)) {
      const nextRunId = runs[0].run_id
      const focusTarget = automaticFocusTarget(narrow, narrowDetailOpen, nextRunId)
      if (focusTarget) restoreFocusTargetRef.current = focusTarget
      setSelectedId(nextRunId)
      setNarrowDetailOpen(false)
    }
  }, [loading, narrow, narrowDetailOpen, pendingNewRunId, pendingSuggestionRunId, runs, selectedId])

  useEffect(() => {
    if (!narrow) return
    const frame = window.requestAnimationFrame(() => {
      if (narrowDetailOpen) {
        detailBackRef.current?.focus()
        return
      }
      const target = restoreFocusTargetRef.current
      if (!target) return
      if (target.kind === 'run') {
        const row = runButtonRefs.current.get(target.runId)
        if (row) row.focus()
        else repositoryFilterRef.current?.focus()
      } else {
        repositoryFilterRef.current?.focus()
      }
      restoreFocusTargetRef.current = null
    })
    return () => window.cancelAnimationFrame(frame)
  }, [narrow, narrowDetailOpen])

  const selectRun = (runId: string) => {
    setSelectedId(runId)
    if (narrow) setNarrowDetailOpen(true)
  }

  const performRetry = async () => {
    if (!selected) return
    const runId = selected.run_id
    const retryTarget = detail?.run_id === runId ? detail : selected
    setRetryStates((current) => beginRetry(current, runId))
    let retryError: string | null = null
    try {
      const result = await retry(retryTarget)
      beginFollow(result.run_id)
      setPendingNewRunId(result.run_id)
      if (result.deduplicated && result.status === 'failed') {
        retryError = 'Cleanup is still pending. Retry again shortly.'
      }
    } catch (error) {
      retryError = errText(error)
    } finally {
      setRetryStates((current) => settleRetry(current, runId, retryError))
    }
  }

  const performCancel = async (runId: string) => {
    setCancelStates((current) => beginRetry(current, runId))
    let cancelError: string | null = null
    try {
      await cancelRun(host, runId)
      if (followRunId === runId) setFollowRunId(null)
    } catch (error) {
      cancelError = errText(error)
    } finally {
      setCancelStates((current) => settleRetry(current, runId, cancelError))
    }
  }

  const openAnalysisChat = (runId: string) => {
    const sessionId = analysisSessionIds[runId]
    if (sessionId) openAnalysisConversation(host, sessionId)
  }

  const performSuggestionRequest = async () => {
    if (!selected) return
    const runId = selected.run_id
    const requestTarget = detail?.run_id === runId ? detail : selected
    setSuggestionState({ runId, pending: true, error: null, message: null })
    try {
      const result = await requestSuggestions(requestTarget)
      setFilters((current) => ({ ...current, status: '' }))
      setPendingSuggestionRunId(result.run_id)
      beginFollow(result.run_id)
      setSuggestionState({
        runId,
        pending: false,
        error: null,
        message: result.deduplicated
          ? `Opening the existing ${formatStatus(result.status)} suggestion run.`
          : `Suggestion run ${formatStatus(result.status)}. Opening it when it appears in history.`,
      })
    } catch (error) {
      setSuggestionState({
        runId,
        pending: false,
        error: errText(error),
        message: null,
      })
    }
  }

  const leaveNarrowDetail = () => {
    if (selectedId) restoreFocusTargetRef.current = { kind: 'run', runId: selectedId }
    setNarrowDetailOpen(false)
  }

  // Configuration opens in the console's own editor dialog — schema fetch,
  // dirty guard and save are host-owned, shared with the workers tab rather
  // than duplicated here. Read off `host.components` at runtime, never
  // imported: a console predating the export degrades to navigation.
  const [configOpen, setConfigOpen] = useState(false)
  const HostConfigDialog = host.components?.WorkerConfigurationDialog as
    | ComponentType<{ configurationId: string | null; onClose: () => void }>
    | undefined
  const openConfiguration = () => {
    if (HostConfigDialog) setConfigOpen(true)
    else window.location.hash = CONFIG_HASH
  }

  const showSidebar = !narrow || !narrowDetailOpen
  const showMain = !narrow || narrowDetailOpen
  const filtersActive = Boolean(filters.repository.trim() || filters.status)

  return (
    <PageShell className={classNames('security-scan-ui-shell', narrow && 'is-narrow')}>
      <PageHeader
        icon={<ShieldIcon />}
        title="security scans"
        description={
          loading ? (narrow ? 'loading' : 'loading review history') : scanHistoryDescription(totalRuns, narrow)
        }
        actions={
          <>
            <span
              className="security-scan-ui-liveness"
              aria-label={live ? 'live updates' : 'reconnecting'}
              title={
                live
                  ? 'Run updates arrive through the security-scan stream.'
                  : 'Stream binding is unavailable. Runs refresh on reconnect, on tab focus, and on refresh.'
              }
            >
              <StatusDot tone={live ? 'accent' : 'ink'} pulse={live} />
              <span>{live ? 'live' : 'offline'}</span>
            </span>
            <Button
              variant="ghost"
              size="sm"
              className="security-scan-ui-configure"
              aria-label="Configure security scans"
              title="Analysis budgets, operator model, and the repository allowlist"
              onClick={openConfiguration}
            >
              <SettingsIcon size={14} />
              <span>configure</span>
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="security-scan-ui-refresh"
              aria-label="Refresh security scans"
              title="Refresh security scans"
              onClick={refresh}
              disabled={loading}
            >
              <RefreshIcon size={14} className={refreshing ? 'is-spinning' : undefined} />
              <span>refresh</span>
            </Button>
          </>
        }
        onClose={onRequestClose}
      />

      {HostConfigDialog ? (
        <HostConfigDialog
          configurationId={configOpen ? 'security-scan' : null}
          onClose={() => {
            setConfigOpen(false)
            // A save may have changed the allowlist or the operator model, and
            // the new-scan form derives both from the stored configuration.
            refresh()
          }}
        />
      ) : null}

      <div ref={bodyRef} className="security-scan-ui-body-observer">
        <PageBody side={panelSide} className={classNames('security-scan-ui-body', narrow && 'is-narrow')}>
          {showSidebar ? (
            <PageSidebar width={320} className="security-scan-ui-sidebar">
              <div className="security-scan-ui-history-head">
                <div>
                  <span className="security-scan-ui-section-label">history</span>
                  <strong>Scan runs</strong>
                </div>
                <span aria-live="polite">
                  {runs.length === totalRuns ? totalRuns : `${runs.length} of ${totalRuns}`}
                </span>
              </div>
              <ScanRequestForm
                host={host}
                conversationId={conversationId}
                onStarted={(runId) => {
                  setFilters({ repository: '', status: '' })
                  setPendingNewRunId(runId)
                  beginFollow(runId)
                  refresh()
                }}
              />
              <div className="security-scan-ui-filters">
                <div className="security-scan-ui-filter-head">
                  <span>filter history</span>
                  {filtersActive ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => {
                        setFilters({ repository: '', status: '' })
                        setNarrowDetailOpen(false)
                      }}
                    >
                      clear
                    </Button>
                  ) : null}
                </div>
                <div className="security-scan-ui-filter">
                  <label htmlFor="security-scan-repository-filter">repository</label>
                  <div className="security-scan-ui-input-wrap">
                    <SearchIcon size={14} />
                    <Input
                      ref={repositoryFilterRef}
                      id="security-scan-repository-filter"
                      value={filters.repository}
                      onChange={(repository) => {
                        setFilters((current) => ({ ...current, repository }))
                        setNarrowDetailOpen(false)
                      }}
                      placeholder="all repository IDs"
                      preserveCase
                      spellCheck={false}
                    />
                  </div>
                </div>
                <div className="security-scan-ui-filter">
                  <span id="security-scan-status-label">status</span>
                  <Select
                    value={filters.status || 'all'}
                    options={statusOptions}
                    onChange={(status) => {
                      setFilters((current) => ({
                        ...current,
                        status: status === 'all' ? '' : status,
                      }))
                      setNarrowDetailOpen(false)
                    }}
                    aria-label="Filter runs by status"
                  />
                </div>
              </div>

              <div className="security-scan-ui-run-scroll">
                {listError ? (
                  <div className="security-scan-ui-list-error" role="alert">
                    <AlertIcon size={18} />
                    <p>{listError}</p>
                    <Button variant="ghost" size="sm" onClick={refresh}>
                      retry
                    </Button>
                  </div>
                ) : loading && runs.length === 0 ? (
                  <div className="security-scan-ui-list-skeleton" role="status" aria-label="Loading security runs">
                    <span />
                    <span />
                    <span />
                    <span />
                  </div>
                ) : runs.length === 0 ? (
                  <div className="security-scan-ui-list-empty">
                    <ShieldIcon size={22} />
                    <strong>no matching runs</strong>
                    <span>Start a scan above, or adjust the filters.</span>
                  </div>
                ) : (
                  <ul className="security-scan-ui-run-list" aria-label="Security scan runs">
                    {runs.map((run) => (
                      <RunListRow
                        key={run.run_id}
                        run={run}
                        selected={run.run_id === selectedId}
                        cancelling={cancelStates[run.run_id]?.pending ?? false}
                        analysisSessionId={analysisSessionIds[run.run_id]}
                        onSelect={() => selectRun(run.run_id)}
                        onCancel={() => void performCancel(run.run_id)}
                        onOpenChat={() => openAnalysisChat(run.run_id)}
                        buttonRef={(node) => {
                          if (node) runButtonRefs.current.set(run.run_id, node)
                          else runButtonRefs.current.delete(run.run_id)
                        }}
                      />
                    ))}
                  </ul>
                )}
              </div>
            </PageSidebar>
          ) : null}

          {showMain ? (
            <PageMain className="security-scan-ui-main">
              {selected ? (
                <SecurityRunDetail
                  key={selected.run_id}
                  run={detail}
                  summary={selected}
                  loading={detailLoading}
                  error={detailError}
                  narrow={narrow}
                  retrying={retryStates[selected.run_id]?.pending ?? false}
                  retryError={retryStates[selected.run_id]?.error ?? null}
                  suggesting={suggestionState?.runId === selected.run_id && suggestionState.pending}
                  suggestionError={suggestionState?.runId === selected.run_id ? suggestionState.error : null}
                  suggestionMessage={suggestionState?.runId === selected.run_id ? suggestionState.message : null}
                  reconciliation={reconciliation}
                  actions={actions}
                  backButtonRef={detailBackRef}
                  onBack={leaveNarrowDetail}
                  onRetry={performRetry}
                  onCancel={() => void performCancel(selected.run_id)}
                  onRequestSuggestions={performSuggestionRequest}
                  analysisSessionId={analysisSessionIds[selected.run_id]}
                  onOpenAnalysisChat={() => openAnalysisChat(selected.run_id)}
                  cancelling={cancelStates[selected.run_id]?.pending ?? false}
                  cancelError={cancelStates[selected.run_id]?.error ?? null}
                />
              ) : runs.length === 0 ? (
                <EmptyState
                  icon={EmptyShield}
                  title="start a security scan"
                  description="Use the sidebar form with an allowlisted repository. Leave the SHA blank to analyze the entire repository at HEAD, or paste a 40-character commit SHA. The scan uses the open chat composer model when Console exposes that selection; otherwise it uses the operator default."
                />
              ) : (
                <EmptyState
                  icon={EmptyShield}
                  title="select a security run"
                  description="Choose a run to inspect its commit, progress, security overview, evidence, and available guidance."
                />
              )}
            </PageMain>
          ) : null}
        </PageBody>
      </div>
    </PageShell>
  )
}
