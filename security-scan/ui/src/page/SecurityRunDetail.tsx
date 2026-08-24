import {
  Badge,
  Button,
  CodeHighlight,
  StatusPanel,
} from '@iii-dev/console-ui'
import { type RefObject, useMemo, useState } from 'react'
import {
  canRequestPatchSuggestions,
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
import { SecurityFindingActions } from './SecurityFindingActions'
import { SecuritySources } from './SecuritySources'
import {
  canCancelRun,
  commitScopeLabel,
  formatLocation,
  formatStatus,
  formatTimestamp,
  type RunStatus,
  type RunSummary,
  type SecurityAssessments,
  type SecurityFinding,
  type SecurityRun,
  type Severity,
  shortSha,
} from './security-scan-data'
import type { useSecurityReconciliation } from './useSecurityReconciliation'
import type { SecurityActionsLive } from './useSecurityActions'
import { nextVisibleFindingCount } from './view-state.js'
import { AlertIcon, ArrowLeftIcon, DownloadIcon, RefreshIcon, WandIcon } from './icons'

const INITIAL_FINDING_COUNT = 20
const FINDING_PAGE_SIZE = 20
const OVERVIEW_ROW_LIMIT = 5

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
                  {categoryCoverageLabel(
                    assessment,
                    categoryFindings.length,
                  )}
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
          <li
            data-state={run.status === 'failed' ? 'failed' : 'cancelled'}
          >
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

function ActiveRunPanel({
  run,
  cancelling,
  cancelError,
  onCancel,
}: {
  run: RunSummary | SecurityRun
  cancelling: boolean
  cancelError: string | null
  onCancel(): void
}) {
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
    <div className="security-scan-ui-active-run">
      <StatusPanel
        variant={run.status === 'cancelling' ? 'warn' : 'info'}
        headline={formatStatus(run.status)}
        detail={detail}
      />
      {canCancelRun(run.status) ? (
        <Button
          variant="ghost"
          size="sm"
          onClick={onCancel}
          disabled={cancelling}
        >
          {cancelling ? 'stopping…' : 'stop scan'}
        </Button>
      ) : null}
      {cancelError ? <p role="alert">{cancelError}</p> : null}
    </div>
  )
}

function FindingCard({
  finding,
  index,
  repository,
  targetSha,
  runId,
  runMode,
  runStatus,
  githubConfigured,
  actions,
}: {
  finding: SecurityFinding
  index: number
  repository: string
  targetSha: string
  runId: string
  runMode: SecurityRun['mode'] | RunSummary['mode']
  runStatus: RunStatus
  githubConfigured: boolean
  actions: SecurityActionsLive
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
      <SecurityFindingActions
        actions={actions}
        runId={runId}
        findingIndex={index}
        runMode={runMode}
        runStatus={runStatus}
        hasPatch={Boolean(finding.suggested_patch?.trim())}
        githubConfigured={githubConfigured}
      />
    </article>
  )
}

export function SecurityRunDetail({
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
  onCancel,
  onRequestSuggestions,
  analysisSessionId,
  onOpenAnalysisChat,
  cancelling,
  cancelError,
  actions,
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
  onCancel(): void
  onRequestSuggestions(): void
  analysisSessionId?: string
  onOpenAnalysisChat(): void
  cancelling: boolean
  cancelError: string | null
  actions: SecurityActionsLive
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
  const sourceZipUrl = githubZipUrl(
    current.repository,
    current.target_sha,
  )
  const canRequestSuggestions = canRequestPatchSuggestions(
    current.mode,
    current.status,
    findings.length,
  )

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
              <code>{commitScopeLabel(current)}</code>
            </div>
          </div>
        </div>
        <div className="security-scan-ui-detail-tools">
          <div className="security-scan-ui-detail-badges">
            <Badge
              variant={current.status === 'failed' ? 'alert' : 'default'}
            >
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
            {analysisSessionId ? (
              <Button variant="ghost" size="sm" onClick={onOpenAnalysisChat}>
                open analysis chat
              </Button>
            ) : null}
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
            {canCancelRun(current.status) ? (
              <Button
                variant="ghost"
                size="sm"
                onClick={onCancel}
                disabled={cancelling}
              >
                {cancelling ? 'stopping…' : 'stop scan'}
              </Button>
            ) : null}
          </div>
        </div>
      </div>

      <dl className="security-scan-ui-facts">
        <div>
          <dt>commit</dt>
          <dd title={current.target_sha}>
            {current.resolved_from_head
              ? `HEAD (${current.target_sha})`
              : current.target_sha}
          </dd>
        </div>
        {current.model ? (
          <div>
            <dt>model</dt>
            <dd title={current.model}>{current.model}</dd>
          </div>
        ) : null}
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
      <ActiveRunPanel
        run={current}
        cancelling={cancelling}
        cancelError={cancelError}
        onCancel={onCancel}
      />
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
                    <strong>Generate recommended fixes</strong>
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
                    {suggesting ? 'requesting' : 'get recommended fixes'}
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
                      runId={current.run_id}
                      runMode={current.mode}
                      runStatus={current.status}
                      githubConfigured={Boolean(
                        reconciliation.data?.github_repository,
                      )}
                      actions={actions}
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
                        {Math.min(
                          FINDING_PAGE_SIZE,
                          remainingFindingCount,
                        )}{' '}
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
