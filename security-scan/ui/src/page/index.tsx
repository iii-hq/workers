import {
  Badge,
  Button,
  CodeHighlight,
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
  StatusPanel,
} from '@iii-dev/console-ui'
import {
  type ComponentType,
  type RefObject,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import {
  AlertIcon,
  ArrowLeftIcon,
  DownloadIcon,
  RefreshIcon,
  SearchIcon,
  SettingsIcon,
  ShieldIcon,
  WandIcon,
} from './icons'
import { SecuritySources } from './SecuritySources'
import {
  buildStatusOptions,
  categorizeFindings,
  categoryCoverageLabel,
  conciseReportTitle,
  countSeverities,
  emptyCategoryMessage,
  FINDING_CATEGORIES,
  githubBlobUrl,
  githubZipUrl,
  isUsefulRemediation,
  reportDownloadFilename,
  serializeSanitizedRun,
} from './security-dashboard.js'
import {
  formatLocation,
  formatRelativeTime,
  formatStatus,
  formatTimestamp,
  RUN_STATUSES,
  type RunFilters,
  type RunStatus,
  type RunSummary,
  type SecurityAssessments,
  type SecurityFinding,
  type SecurityRun,
  type Severity,
  shortSha,
} from './security-scan-data'
import { useSecurityReconciliation } from './useSecurityReconciliation'
import { useSecurityRunsLive } from './useSecurityRunsLive'
import {
  automaticFocusTarget,
  beginRetry,
  nextVisibleFindingCount,
  settleRetry,
} from './view-state.js'

const NARROW_BELOW = 760
/** Where a console without the configuration-dialog export sends the operator. */
const CONFIG_HASH = '#/workers/configuration/security-scan'
const INITIAL_FINDING_COUNT = 20
const FINDING_PAGE_SIZE = 20
const OVERVIEW_ROW_LIMIT = 5

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

const PIPELINE: ReadonlyArray<{ status: RunStatus; label: string }> = [
  { status: 'queued', label: 'queued' },
  { status: 'materializing', label: 'checkout' },
  { status: 'materialized', label: 'verified' },
  { status: 'dispatching', label: 'dispatch' },
  { status: 'analyzing', label: 'analysis' },
  { status: 'completed', label: 'report' },
]

const SEVERITY_ORDER: Record<Severity, number> = {
  critical: 0,
  high: 1,
  medium: 2,
  low: 3,
  info: 4,
}

const SEVERITIES: Severity[] = ['critical', 'high', 'medium', 'low', 'info']

function classNames(
  ...values: Array<string | false | null | undefined>
): string {
  return values.filter(Boolean).join(' ')
}

function useContainerNarrow(
  threshold: number,
): [(node: HTMLDivElement | null) => void, boolean] {
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

function statusIsActive(status: RunStatus): boolean {
  return !['completed', 'failed', 'cancelled'].includes(status)
}

function severityVariant(
  severity: Severity,
): 'default' | 'warn' | 'alert' | 'accent' {
  if (severity === 'critical' || severity === 'high') return 'alert'
  if (severity === 'medium') return 'warn'
  if (severity === 'low') return 'accent'
  return 'default'
}

function findingLabel(count: number): string {
  return `${count} ${count === 1 ? 'finding' : 'findings'}`
}

function downloadSanitizedReport(run: SecurityRun) {
  const blob = new Blob([serializeSanitizedRun(run)], {
    type: 'application/json;charset=utf-8',
  })
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = reportDownloadFilename(run)
  anchor.hidden = true
  document.body.append(anchor)
  anchor.click()
  anchor.remove()
  window.setTimeout(() => URL.revokeObjectURL(url), 0)
}

function FindingLocationLink({
  repository,
  targetSha,
  finding,
}: {
  repository: string
  targetSha: string
  finding: SecurityFinding
}) {
  const label = formatLocation(finding.location)
  const url = finding.location
    ? githubBlobUrl(
        repository,
        targetSha,
        finding.location.path,
        finding.location.line_start,
        finding.location.line_end,
      )
    : null
  return url ? (
    <a
      href={url}
      target="_blank"
      rel="noreferrer"
      title={`Open ${label} at ${shortSha(targetSha)} on GitHub`}
    >
      {label}
    </a>
  ) : (
    <span>{label}</span>
  )
}

function SecurityOverview({
  findings,
  assessments,
  repository,
  targetSha,
}: {
  findings: SecurityFinding[]
  assessments: SecurityAssessments
  repository: string
  targetSha: string
}) {
  const categories = useMemo(() => categorizeFindings(findings), [findings])

  return (
    <section
      className="security-scan-ui-overview"
      aria-labelledby="security-scan-overview-title"
    >
      <div className="security-scan-ui-overview-head">
        <div>
          <span className="security-scan-ui-section-label">Harness review</span>
          <h3 id="security-scan-overview-title">Harness review coverage</h3>
        </div>
        <span>{findingLabel(findings.length)}</span>
      </div>
      <div className="security-scan-ui-overview-grid">
        {FINDING_CATEGORIES.map((category) => {
          const categoryFindings = categories[category.id]
          const assessment = assessments[category.assessmentKey]
          const severityCounts = countSeverities(categoryFindings)
          const visibleRows = categoryFindings.slice(0, OVERVIEW_ROW_LIMIT)
          const remaining = categoryFindings.length - visibleRows.length
          return (
            <section
              className="security-scan-ui-overview-category"
              key={category.id}
            >
              <header>
                <h4>{category.label}</h4>
                <strong>{categoryFindings.length}</strong>
              </header>
              <div className="security-scan-ui-overview-severities">
                {SEVERITIES.filter(
                  (severity) => severityCounts[severity] > 0,
                ).map((severity) => (
                  <span key={severity} data-severity={severity}>
                    {severity} {severityCounts[severity]}
                  </span>
                ))}
                <span>
                  {categoryCoverageLabel(assessment, categoryFindings.length)}
                </span>
              </div>
              <div className="security-scan-ui-overview-table-wrap">
                <table>
                  <thead>
                    <tr>
                      <th scope="col">severity</th>
                      <th scope="col">finding</th>
                      <th scope="col">location</th>
                    </tr>
                  </thead>
                  <tbody>
                    {visibleRows.length === 0 ? (
                      <tr>
                        <td colSpan={3}>{emptyCategoryMessage(assessment)}</td>
                      </tr>
                    ) : (
                      visibleRows.map((finding, index) => (
                        <tr
                          key={`${finding.rule_id}:${finding.location?.path ?? ''}:${index}`}
                        >
                          <td>
                            <Badge variant={severityVariant(finding.severity)}>
                              {finding.severity}
                            </Badge>
                          </td>
                          <td title={finding.title}>{finding.title}</td>
                          <td className="security-scan-ui-overview-location">
                            <FindingLocationLink
                              repository={repository}
                              targetSha={targetSha}
                              finding={finding}
                            />
                          </td>
                        </tr>
                      ))
                    )}
                  </tbody>
                </table>
              </div>
              {remaining > 0 ? (
                <p>+{remaining} more in detailed findings</p>
              ) : null}
            </section>
          )
        })}
      </div>
    </section>
  )
}

function RunListRow({
  run,
  selected,
  onSelect,
  buttonRef,
}: {
  run: RunSummary
  selected: boolean
  onSelect(): void
  buttonRef(node: HTMLButtonElement | null): void
}) {
  return (
    <li>
      <button
        type="button"
        className={classNames(
          'security-scan-ui-run',
          selected && 'is-selected',
        )}
        aria-current={selected ? 'true' : undefined}
        aria-label={`${run.repository} ${shortSha(run.target_sha)}, ${formatStatus(run.status)}, ${findingLabel(run.finding_count)}`}
        data-run-id={run.run_id}
        ref={buttonRef}
        onClick={onSelect}
      >
        <span className="security-scan-ui-run-topline">
          <span className="security-scan-ui-run-repo" title={run.repository}>
            {run.repository}
          </span>
          <StatusDot
            tone={statusTone(run.status)}
            pulse={statusIsActive(run.status)}
            title={formatStatus(run.status)}
          />
        </span>
        <span className="security-scan-ui-run-meta">
          <code>{shortSha(run.target_sha)}</code>
          <span aria-hidden="true">·</span>
          <span title={formatTimestamp(run.updated_at)}>
            {formatRelativeTime(run.updated_at)}
          </span>
          <span className="security-scan-ui-run-status">
            {formatStatus(run.status)}
          </span>
          <span className="security-scan-ui-run-count">
            {findingLabel(run.finding_count)}
          </span>
        </span>
      </button>
    </li>
  )
}

function Progression({ run }: { run: RunSummary | SecurityRun }) {
  const activeIndex = PIPELINE.findIndex((step) => step.status === run.status)
  const completed = run.status === 'completed'
  const interrupted = run.status === 'failed' || run.status === 'cancelled'

  return (
    <section
      className="security-scan-ui-progress"
      aria-label={`Run status: ${formatStatus(run.status)}`}
    >
      <div className="security-scan-ui-section-label">progress</div>
      <ol>
        {PIPELINE.map((step, index) => {
          const state = completed
            ? 'done'
            : interrupted
              ? 'unknown'
              : index < activeIndex
                ? 'done'
                : index === activeIndex
                  ? 'current'
                  : 'future'
          return (
            <li key={step.status} data-state={state}>
              <span
                className="security-scan-ui-progress-mark"
                aria-hidden="true"
              />
              <span>{step.label}</span>
            </li>
          )
        })}
        {interrupted ? (
          <li data-state={run.status === 'failed' ? 'failed' : 'cancelled'}>
            <span
              className="security-scan-ui-progress-mark"
              aria-hidden="true"
            />
            <span>{formatStatus(run.status)}</span>
          </li>
        ) : null}
      </ol>
    </section>
  )
}

function ActiveRunPanel({ run }: { run: RunSummary | SecurityRun }) {
  const details: Partial<Record<RunStatus, string>> = {
    queued: 'Waiting for the durable scanner queue.',
    materializing: 'Creating an isolated checkout at the requested commit.',
    materialized: 'The exact checkout is verified and ready for dispatch.',
    dispatching: 'Starting the read-only Harness review.',
    analyzing: 'Harness is reviewing repository evidence.',
    cancelling: 'Cancellation is in progress.',
  }
  const detail = details[run.status]
  if (!detail) return null
  return (
    <StatusPanel
      variant={run.status === 'cancelling' ? 'warn' : 'info'}
      headline={formatStatus(run.status)}
      detail={detail}
    />
  )
}

function FindingCard({
  finding,
  index,
  repository,
  targetSha,
}: {
  finding: SecurityFinding
  index: number
  repository: string
  targetSha: string
}) {
  const [patchOpen, setPatchOpen] = useState(false)
  const usefulRemediation = isUsefulRemediation(finding.remediation)

  return (
    <article className="security-scan-ui-finding">
      <header>
        <span className="security-scan-ui-finding-number">
          {String(index + 1).padStart(2, '0')}
        </span>
        <div>
          <div className="security-scan-ui-finding-badges">
            <Badge variant={severityVariant(finding.severity)}>
              {finding.severity}
            </Badge>
            <code>{finding.rule_id}</code>
          </div>
          <h3>{finding.title}</h3>
          <div className="security-scan-ui-location">
            <FindingLocationLink
              repository={repository}
              targetSha={targetSha}
              finding={finding}
            />
          </div>
        </div>
      </header>

      <p className="security-scan-ui-finding-description">
        {finding.description}
      </p>

      <div
        className={classNames(
          'security-scan-ui-evidence-grid',
          !usefulRemediation && 'has-one-column',
        )}
      >
        <section>
          <h4>evidence</h4>
          <pre>{finding.evidence}</pre>
        </section>
        {usefulRemediation ? (
          <section>
            <h4>remediation</h4>
            <p>{finding.remediation}</p>
          </section>
        ) : null}
      </div>

      {finding.suggested_patch ? (
        <details
          className="security-scan-ui-patch"
          onToggle={(event) => setPatchOpen(event.currentTarget.open)}
        >
          <summary>suggested patch</summary>
          {patchOpen ? (
            <div className="security-scan-ui-patch-content">
              <CodeHighlight
                code={finding.suggested_patch}
                language="diff"
                wrap
              />
              <p>Suggestion only. The scanner did not apply this patch.</p>
            </div>
          ) : null}
        </details>
      ) : null}
    </article>
  )
}

function RunDetail({
  run,
  summary,
  loading,
  error,
  narrow,
  retrying,
  retryError,
  suggesting,
  suggestionError,
  suggestionMessage,
  reconciliation,
  backButtonRef,
  onBack,
  onRetry,
  onRequestSuggestions,
}: {
  run: SecurityRun | null
  summary: RunSummary
  loading: boolean
  error: string | null
  narrow: boolean
  retrying: boolean
  retryError: string | null
  suggesting: boolean
  suggestionError: string | null
  suggestionMessage: string | null
  reconciliation: ReturnType<typeof useSecurityReconciliation>
  backButtonRef: RefObject<HTMLButtonElement | null>
  onBack(): void
  onRetry(): void
  onRequestSuggestions(): void
}) {
  const current = run ?? summary
  const findings = useMemo(
    () =>
      [...(run?.report?.findings ?? [])].sort(
        (left, right) =>
          SEVERITY_ORDER[left.severity] - SEVERITY_ORDER[right.severity],
      ),
    [run?.report?.findings],
  )
  const [visibleFindingCount, setVisibleFindingCount] = useState(
    INITIAL_FINDING_COUNT,
  )
  const visibleFindings = findings.slice(0, visibleFindingCount)
  const remainingFindingCount = findings.length - visibleFindings.length
  const canRetry =
    current.status === 'failed' && current.error?.retryable === true
  const findingCount = run?.report?.findings.length ?? summary.finding_count
  const title = conciseReportTitle(
    run?.report?.summary,
    findingCount,
    current.status,
  )
  const sourceZipUrl = githubZipUrl(current.repository, current.target_sha)
  const canRequestSuggestions =
    current.mode === 'scan' &&
    current.status === 'completed' &&
    findings.some((finding) => !isUsefulRemediation(finding.remediation))

  return (
    <div className="security-scan-ui-detail">
      <div className="security-scan-ui-detail-head">
        <div className="security-scan-ui-detail-title">
          {narrow ? (
            <Button
              ref={backButtonRef}
              variant="ghost"
              size="sm"
              onClick={onBack}
            >
              <ArrowLeftIcon size={14} />
              history
            </Button>
          ) : null}
          <div className="security-scan-ui-detail-copy">
            <h2>{title}</h2>
            <div className="security-scan-ui-detail-kicker">
              <span>{current.repository}</span>
              <span aria-hidden="true">·</span>
              <code>{shortSha(current.target_sha)}</code>
            </div>
          </div>
        </div>
        <div className="security-scan-ui-detail-tools">
          <div className="security-scan-ui-detail-badges">
            <Badge variant={current.status === 'failed' ? 'alert' : 'default'}>
              {formatStatus(current.status)}
            </Badge>
            <Badge>{current.mode}</Badge>
            <span>attempt {current.attempt}</span>
          </div>
          <div
            className="security-scan-ui-detail-actions"
            role="toolbar"
            aria-label="Run downloads"
          >
            {sourceZipUrl ? (
              <Button asChild variant="ghost" size="sm">
                <a href={sourceZipUrl} target="_blank" rel="noreferrer">
                  <DownloadIcon size={14} />
                  source ZIP
                </a>
              </Button>
            ) : null}
            {run?.report ? (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => downloadSanitizedReport(run)}
              >
                <DownloadIcon size={14} />
                report JSON
              </Button>
            ) : null}
          </div>
        </div>
      </div>

      <dl className="security-scan-ui-facts">
        <div>
          <dt>commit</dt>
          <dd title={current.target_sha}>{current.target_sha}</dd>
        </div>
        <div>
          <dt>started</dt>
          <dd>{formatTimestamp(current.created_at)}</dd>
        </div>
        <div>
          <dt>updated</dt>
          <dd>{formatTimestamp(current.updated_at)}</dd>
        </div>
        <div>
          <dt>run id</dt>
          <dd title={current.run_id}>{current.run_id}</dd>
        </div>
      </dl>

      <Progression run={current} />

      {loading && !run ? (
        <div
          className="security-scan-ui-detail-skeleton"
          role="status"
          aria-label="Loading run report"
        >
          <span />
          <span />
          <span />
        </div>
      ) : null}

      {error ? (
        <div role="alert">
          <StatusPanel
            variant="alert"
            icon={<AlertIcon size={18} />}
            headline="failed to load run details"
            detail={error}
          />
        </div>
      ) : null}

      <ActiveRunPanel run={current} />

      {current.status === 'failed' ? (
        <div className="security-scan-ui-failure">
          <StatusPanel
            variant="alert"
            icon={<AlertIcon size={18} />}
            headline={current.error?.code ?? 'scan failed'}
            detail={
              current.error?.message ??
              'The scan failed without a structured error.'
            }
          />
          {canRetry ? (
            <Button
              variant="primary"
              size="sm"
              onClick={onRetry}
              disabled={retrying}
            >
              <RefreshIcon
                size={14}
                className={retrying ? 'is-spinning' : undefined}
              />
              {retrying ? 'retrying' : 'retry run'}
            </Button>
          ) : null}
          {retryError ? <p role="alert">{retryError}</p> : null}
        </div>
      ) : null}

      {current.status === 'cancelled' ? (
        <StatusPanel
          variant="warn"
          headline="scan cancelled"
          detail="No completed report is available for this run."
        />
      ) : null}

      {run?.report ? (
        <div className="security-scan-ui-report">
          <section className="security-scan-ui-summary">
            <div>
              <span className="security-scan-ui-section-label">
                Harness report summary
              </span>
              <h3>
                {findings.length} Harness{' '}
                {findings.length === 1 ? 'finding' : 'findings'}
              </h3>
            </div>
            <p>{run.report.summary}</p>
          </section>

          <SecuritySources
            runId={current.run_id}
            harnessFindingCount={findings.length}
            reconciliation={reconciliation.data}
            loading={reconciliation.loading}
            refreshing={reconciliation.refreshing}
            loadingMore={reconciliation.loadingMore}
            error={reconciliation.error}
            narrow={narrow}
            onRefresh={reconciliation.refresh}
            onLoadMore={reconciliation.loadMore}
          />

          <SecurityOverview
            findings={findings}
            assessments={run.report.assessments}
            repository={current.repository}
            targetSha={current.target_sha}
          />

          {findings.length === 0 ? (
            <StatusPanel
              variant="success"
              headline="no Harness findings reported"
              detail="This completed Harness review reported no findings. GitHub source alerts remain separate, and a zero is not proof that the code is vulnerability-free."
            />
          ) : (
            <>
              {canRequestSuggestions ? (
                <section className="security-scan-ui-suggestion-action">
                  <div>
                    <span className="security-scan-ui-section-label">
                      follow-up review
                    </span>
                    <strong>Request concrete patch suggestions</strong>
                    <p>
                      Run a separate suggestion-mode review. Suggestions stay
                      read-only and are never applied.
                    </p>
                    {suggestionError ? (
                      <p role="alert">{suggestionError}</p>
                    ) : null}
                    {suggestionMessage ? (
                      <p role="status">{suggestionMessage}</p>
                    ) : null}
                  </div>
                  <Button
                    variant="primary"
                    size="sm"
                    onClick={onRequestSuggestions}
                    disabled={suggesting}
                  >
                    <WandIcon size={14} />
                    {suggesting ? 'requesting' : 'request suggestions'}
                  </Button>
                </section>
              ) : null}

              <section
                className="security-scan-ui-detailed-findings"
                aria-labelledby="security-scan-detailed-findings-title"
              >
                <div className="security-scan-ui-detailed-findings-head">
                  <div>
                    <span className="security-scan-ui-section-label">
                      Harness evidence and guidance
                    </span>
                    <h3 id="security-scan-detailed-findings-title">
                      Detailed Harness findings
                    </h3>
                  </div>
                  <span>{findingLabel(findings.length)}</span>
                </div>
                <div className="security-scan-ui-findings">
                  {visibleFindings.map((finding, index) => (
                    <FindingCard
                      key={`${current.run_id}:${finding.rule_id}:${finding.location?.path ?? ''}:${index}`}
                      finding={finding}
                      index={index}
                      repository={current.repository}
                      targetSha={current.target_sha}
                    />
                  ))}
                  {remainingFindingCount > 0 ? (
                    <div className="security-scan-ui-findings-more">
                      <span aria-live="polite">
                        showing {visibleFindings.length} of {findings.length}
                      </span>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() =>
                          setVisibleFindingCount((currentCount) =>
                            nextVisibleFindingCount(
                              currentCount,
                              findings.length,
                              FINDING_PAGE_SIZE,
                            ),
                          )
                        }
                      >
                        show{' '}
                        {Math.min(FINDING_PAGE_SIZE, remainingFindingCount)}{' '}
                        more
                      </Button>
                    </div>
                  ) : null}
                </div>
              </section>
            </>
          )}
        </div>
      ) : current.status === 'completed' && !loading ? (
        <StatusPanel
          variant="warn"
          headline="completed without a report"
          detail="The run is terminal, but security-scan::read returned no report payload."
        />
      ) : null}
    </div>
  )
}

const EmptyShield = () => <ShieldIcon size={28} />

export function SecurityScanPage({
  host,
  panelSide = 'left',
  onRequestClose,
}: { host: Host } & Partial<PageRenderProps>) {
  const [filters, setFilters] = useState<RunFilters>({
    repository: '',
    status: '',
  })
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [narrowDetailOpen, setNarrowDetailOpen] = useState(false)
  const [retryStates, setRetryStates] = useState<RetryStates>({})
  const [suggestionState, setSuggestionState] =
    useState<SuggestionState | null>(null)
  const [pendingSuggestionRunId, setPendingSuggestionRunId] = useState<
    string | null
  >(null)
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
  const reconciliation = useSecurityReconciliation(
    host,
    selectedId,
    reconciliationRefreshRevision,
  )

  const statusOptions = useMemo(
    () =>
      buildStatusOptions(RUN_STATUSES, statusCounts, totalRuns) as Array<{
        value: StatusOptionValue
        label: string
      }>,
    [statusCounts, totalRuns],
  )

  const selected = useMemo(
    () => runs.find((run) => run.run_id === selectedId) ?? null,
    [runs, selectedId],
  )

  useEffect(() => {
    if (!pendingSuggestionRunId) return
    if (!runs.some((run) => run.run_id === pendingSuggestionRunId)) return
    setSelectedId(pendingSuggestionRunId)
    setPendingSuggestionRunId(null)
    if (narrow) setNarrowDetailOpen(true)
  }, [narrow, pendingSuggestionRunId, runs])

  useEffect(() => {
    if (loading) return
    if (runs.length === 0) {
      const focusTarget = automaticFocusTarget(narrow, narrowDetailOpen, null)
      if (focusTarget) restoreFocusTargetRef.current = focusTarget
      setSelectedId(null)
      setNarrowDetailOpen(false)
      return
    }
    if (!selectedId || !runs.some((run) => run.run_id === selectedId)) {
      const nextRunId = runs[0].run_id
      const focusTarget = automaticFocusTarget(
        narrow,
        narrowDetailOpen,
        nextRunId,
      )
      if (focusTarget) restoreFocusTargetRef.current = focusTarget
      setSelectedId(nextRunId)
      setNarrowDetailOpen(false)
    }
  }, [loading, narrow, narrowDetailOpen, runs, selectedId])

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
      if (result.deduplicated && result.status === 'failed') {
        retryError = 'Cleanup is still pending. Retry again shortly.'
      }
    } catch (error) {
      retryError = error instanceof Error ? error.message : String(error)
    } finally {
      setRetryStates((current) => settleRetry(current, runId, retryError))
    }
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
        error: error instanceof Error ? error.message : String(error),
        message: null,
      })
    }
  }

  const leaveNarrowDetail = () => {
    if (selectedId)
      restoreFocusTargetRef.current = { kind: 'run', runId: selectedId }
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
    <PageShell className="security-scan-ui-shell">
      <PageHeader
        icon={<ShieldIcon />}
        title="security scans"
        description={
          loading
            ? 'loading review history'
            : `${totalRuns} recent repository reviews`
        }
        actions={
          <>
            <span
              className="security-scan-ui-liveness"
              title={
                live
                  ? 'Run updates arrive through the security-scan stream.'
                  : 'Stream binding is unavailable. Runs refresh on reconnect, on tab focus, and on refresh.'
              }
            >
              <StatusDot tone={live ? 'accent' : 'ink'} pulse={live} />
              {live ? 'live' : 'offline'}
            </span>
            <Button
              variant="ghost"
              size="sm"
              aria-label="Configure security scans"
              title="Analysis budgets, operator model, and the repository allowlist"
              onClick={openConfiguration}
            >
              <SettingsIcon size={14} />
              configure
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={refresh}
              disabled={loading}
            >
              <RefreshIcon
                size={14}
                className={refreshing ? 'is-spinning' : undefined}
              />
              refresh
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
            // the page derives what it offers from the stored configuration.
            refresh()
          }}
        />
      ) : null}

      <div ref={bodyRef} className="security-scan-ui-body-observer">
        <PageBody
          side={panelSide}
          className={classNames('security-scan-ui-body', narrow && 'is-narrow')}
        >
          {showSidebar ? (
            <PageSidebar width={320} className="security-scan-ui-sidebar">
              <div className="security-scan-ui-history-head">
                <div>
                  <span className="security-scan-ui-section-label">
                    history
                  </span>
                  <strong>Scan runs</strong>
                </div>
                <span aria-live="polite">
                  {runs.length === totalRuns
                    ? totalRuns
                    : `${runs.length} of ${totalRuns}`}
                </span>
              </div>
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
                  <label htmlFor="security-scan-repository-filter">
                    repository
                  </label>
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
                  <div
                    className="security-scan-ui-list-skeleton"
                    role="status"
                    aria-label="Loading security runs"
                  >
                    <span />
                    <span />
                    <span />
                    <span />
                  </div>
                ) : runs.length === 0 ? (
                  <div className="security-scan-ui-list-empty">
                    <ShieldIcon size={22} />
                    <strong>no matching runs</strong>
                    <span>
                      Adjust the filters or request a scan through
                      security-scan::request.
                    </span>
                  </div>
                ) : (
                  <ul
                    className="security-scan-ui-run-list"
                    aria-label="Security scan runs"
                  >
                    {runs.map((run) => (
                      <RunListRow
                        key={run.run_id}
                        run={run}
                        selected={run.run_id === selectedId}
                        onSelect={() => selectRun(run.run_id)}
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
                <RunDetail
                  key={selected.run_id}
                  run={detail}
                  summary={selected}
                  loading={detailLoading}
                  error={detailError}
                  narrow={narrow}
                  retrying={retryStates[selected.run_id]?.pending ?? false}
                  retryError={retryStates[selected.run_id]?.error ?? null}
                  suggesting={
                    suggestionState?.runId === selected.run_id &&
                    suggestionState.pending
                  }
                  suggestionError={
                    suggestionState?.runId === selected.run_id
                      ? suggestionState.error
                      : null
                  }
                  suggestionMessage={
                    suggestionState?.runId === selected.run_id
                      ? suggestionState.message
                      : null
                  }
                  reconciliation={reconciliation}
                  backButtonRef={detailBackRef}
                  onBack={leaveNarrowDetail}
                  onRetry={performRetry}
                  onRequestSuggestions={performSuggestionRequest}
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
