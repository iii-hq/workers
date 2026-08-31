import { Badge, Button } from '@iii-dev/console-ui'
import { useEffect, useMemo, useState } from 'react'
import { RefreshIcon } from './icons'
import {
  alertMatchLabel,
  buildSecuritySourceSummary,
  filterGithubAlerts,
  GITHUB_ALERT_FILTERS,
  githubCollectionState,
  githubCollectionStateCopy,
  githubCommitUrl,
  githubOpenAlertCount,
  nextVisibleAlertCount,
  overallGithubCollectionState,
  reconciliationScopeLabel,
  sourceCommitPresentation,
  sourceCountLabel,
} from './security-dashboard.js'
import {
  formatTimestamp,
  type GitHubAlertRecord,
  type GitHubSourceReconciliation,
  type SecurityReconciliation,
  type Severity,
} from './security-scan-data'

const INITIAL_ALERT_COUNT = 25
const ALERT_PAGE_SIZE = 25

export type GitHubAlertSource = 'dependabot' | 'code_scanning'
export type GitHubAlertFilter = 'all' | GitHubAlertSource
export type GitHubCollectionState =
  | 'not_collected'
  | 'not_configured'
  | 'auth'
  | 'permission'
  | 'disabled'
  | 'partial'
  | 'unavailable'
  | 'complete'

export interface GitHubAlertView {
  id: string
  source: GitHubAlertSource
  severity: Severity
  title: string
  lifecycle: string
  scope: string
  match: string
  url?: string
}

function severityVariant(severity: Severity): 'default' | 'warn' | 'alert' | 'accent' {
  if (severity === 'critical' || severity === 'high') return 'alert'
  if (severity === 'medium') return 'warn'
  if (severity === 'low') return 'accent'
  return 'default'
}

function sourceLabel(source: GitHubAlertSource): string {
  return source === 'dependabot' ? 'Dependabot' : 'Code scanning'
}

function alertLocation(record: GitHubAlertRecord): string | null {
  if (!record.path) return null
  if (record.start_line == null) return record.path
  if (record.end_line != null && record.end_line !== record.start_line) {
    return `${record.path}:${record.start_line}-${record.end_line}`
  }
  return `${record.path}:${record.start_line}`
}

function toAlertView(record: GitHubAlertRecord, reconciliation: SecurityReconciliation): GitHubAlertView {
  const scope = reconciliationScopeLabel(record.scope, reconciliation.target_sha)
  const location = alertLocation(record)
  return {
    id: `${record.source}:${record.number}`,
    source: record.source,
    severity: record.severity,
    title: record.title,
    lifecycle: record.lifecycle,
    scope: location ? `${scope} · ${location}` : scope,
    match: alertMatchLabel(reconciliation.matching.status),
    url: record.public_url || undefined,
  }
}

function safeGitHubHttpsUrl(url: string | undefined): string | undefined {
  if (!url) return undefined
  try {
    const parsed = new URL(url)
    if (parsed.protocol !== 'https:') return undefined
    if (parsed.username || parsed.password) return undefined
    if (parsed.hostname !== 'github.com' && parsed.hostname !== 'www.github.com') {
      return undefined
    }
    if (parsed.pathname.includes('\\') || parsed.pathname.includes('//')) {
      return undefined
    }
    return parsed.toString()
  } catch {
    return undefined
  }
}

function AlertLink({ alert }: { alert: GitHubAlertView }) {
  const url = safeGitHubHttpsUrl(alert.url)
  return url ? (
    <a href={url} target="_blank" rel="noreferrer">
      {alert.title}
    </a>
  ) : (
    <span>{alert.title}</span>
  )
}

function GitHubAlertsTable({ alerts }: { alerts: GitHubAlertView[] }) {
  return (
    <div className="security-scan-ui-source-table-wrap">
      <table className="security-scan-ui-source-table">
        <caption>Open alerts in the collected GitHub security snapshot</caption>
        <thead>
          <tr>
            <th scope="col">severity</th>
            <th scope="col">source</th>
            <th scope="col">alert</th>
            <th scope="col">lifecycle</th>
            <th scope="col">scope</th>
            <th scope="col">match</th>
          </tr>
        </thead>
        <tbody>
          {alerts.map((alert) => (
            <tr key={alert.id}>
              <td>
                <Badge variant={severityVariant(alert.severity)}>{alert.severity}</Badge>
              </td>
              <td>{sourceLabel(alert.source)}</td>
              <td className="security-scan-ui-source-alert-title">
                <AlertLink alert={alert} />
              </td>
              <td>{alert.lifecycle}</td>
              <td>{alert.scope}</td>
              <td>{alert.match}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

function GitHubAlertsList({ alerts }: { alerts: GitHubAlertView[] }) {
  return (
    <ul className="security-scan-ui-source-alert-list" aria-label="Open GitHub security alert records">
      {alerts.map((alert) => (
        <li key={alert.id}>
          <div className="security-scan-ui-source-alert-head">
            <Badge variant={severityVariant(alert.severity)}>{alert.severity}</Badge>
            <span>{sourceLabel(alert.source)}</span>
            <span>{alert.lifecycle}</span>
          </div>
          <strong>
            <AlertLink alert={alert} />
          </strong>
          <dl>
            <div>
              <dt>scope</dt>
              <dd>{alert.scope}</dd>
            </div>
            <div>
              <dt>match</dt>
              <dd>{alert.match}</dd>
            </div>
          </dl>
        </li>
      ))}
    </ul>
  )
}

function GitHubSourceCard({
  source,
  targetSha,
  githubRepository,
}: {
  source: GitHubSourceReconciliation
  targetSha: string
  githubRepository: string | null
}) {
  const state = githubCollectionState(source.status) as GitHubCollectionState
  const copy = githubCollectionStateCopy(state, source.record_count)
  const healthTool = source.health.tool ? ` · ${source.health.tool}` : ''
  const healthCommit = sourceCommitPresentation(source.health.commit_sha, targetSha)
  const healthCommitUrl = healthCommit ? githubCommitUrl(githubRepository ?? '', healthCommit.sha) : null
  return (
    <section className="security-scan-ui-source-card" data-state={state}>
      <header>
        <strong>{sourceLabel(source.source)}</strong>
        <span>
          {source.record_count == null
            ? 'count unavailable'
            : `${sourceCountLabel(source.record_count, source.status === 'complete')} open`}
        </span>
      </header>
      <p>
        <strong>{copy.label}.</strong> {copy.detail}
      </p>
      <dl>
        <div>
          <dt>scope</dt>
          <dd>{reconciliationScopeLabel(source.scope, targetSha)}</dd>
        </div>
        <div>
          <dt>snapshot time</dt>
          <dd>{source.collected_at == null ? 'Not collected' : formatTimestamp(source.collected_at)}</dd>
        </div>
        <div>
          <dt>source health</dt>
          <dd>
            {source.health.status.replace('_', ' ')}
            {healthTool}
          </dd>
        </div>
        {source.source === 'code_scanning' ? (
          <>
            <div>
              <dt>analysis commit</dt>
              <dd>
                {healthCommit ? (
                  <>
                    {healthCommitUrl ? (
                      <a href={healthCommitUrl} target="_blank" rel="noreferrer" title={healthCommit.sha}>
                        <code>{healthCommit.short}</code>
                      </a>
                    ) : (
                      <code title={healthCommit.sha}>{healthCommit.short}</code>
                    )}
                    {healthCommit.differsFromTarget
                      ? ` · differs from Harness target ${targetSha.slice(0, 8)}`
                      : ' · matches Harness target'}
                  </>
                ) : (
                  'Not reported'
                )}
              </dd>
            </div>
            <div>
              <dt>analysis observed</dt>
              <dd>
                {source.health.observed_at ? (
                  <time dateTime={source.health.observed_at}>{source.health.observed_at}</time>
                ) : (
                  'Not reported'
                )}
              </dd>
            </div>
          </>
        ) : null}
      </dl>
    </section>
  )
}

export function SecuritySources({
  runId,
  harnessFindingCount,
  reconciliation,
  loading,
  refreshing,
  loadingMore,
  error,
  narrow,
  onRefresh,
  onLoadMore,
}: {
  runId: string
  harnessFindingCount: number
  reconciliation: SecurityReconciliation | null
  loading: boolean
  refreshing: boolean
  loadingMore: boolean
  error: string | null
  narrow: boolean
  onRefresh(): void
  onLoadMore(): void
}) {
  const [filter, setFilter] = useState<GitHubAlertFilter>('all')
  const [visibleAlertCount, setVisibleAlertCount] = useState(INITIAL_ALERT_COUNT)
  const [revealAfterLoad, setRevealAfterLoad] = useState(false)
  const sources = reconciliation?.sources ?? []
  const count = githubOpenAlertCount(sources)
  const collectionState = overallGithubCollectionState(sources) as GitHubCollectionState
  const alerts = useMemo(
    () => reconciliation?.records.map((record) => toAlertView(record, reconciliation)) ?? [],
    [reconciliation],
  )
  const harnessVerifiedCount =
    reconciliation?.harness.status === 'not_available'
      ? null
      : (reconciliation?.harness.verified_count ?? harnessFindingCount)
  const summary = buildSecuritySourceSummary(
    harnessVerifiedCount,
    count.count,
    count.complete,
    reconciliation?.matching.status === 'available',
  )
  const stateCopy = githubCollectionStateCopy(collectionState, count.count)
  const filteredAlerts = useMemo(() => filterGithubAlerts(alerts, filter) as GitHubAlertView[], [alerts, filter])
  const visibleAlerts = filteredAlerts.slice(0, visibleAlertCount)
  const remainingAlertCount = filteredAlerts.length - visibleAlerts.length

  useEffect(() => {
    setFilter('all')
    setVisibleAlertCount(INITIAL_ALERT_COUNT)
    setRevealAfterLoad(false)
  }, [runId])

  useEffect(() => {
    setVisibleAlertCount(INITIAL_ALERT_COUNT)
    setRevealAfterLoad(false)
  }, [filter])

  useEffect(() => {
    if (!revealAfterLoad || loadingMore || filteredAlerts.length <= visibleAlertCount) return
    setVisibleAlertCount((current) => nextVisibleAlertCount(current, filteredAlerts.length, ALERT_PAGE_SIZE))
    setRevealAfterLoad(false)
  }, [filteredAlerts.length, loadingMore, revealAfterLoad, visibleAlertCount])

  const latestSnapshotAt = sources.reduce<number | null>(
    (latest, source) =>
      source.collected_at != null && (latest == null || source.collected_at > latest) ? source.collected_at : latest,
    null,
  )
  const matchingLabel =
    reconciliation?.matching.status === 'available'
      ? reconciliation.matching.matched_records == null
        ? 'Matching available · no matched records reported'
        : `${reconciliation.matching.matched_records} matched source records`
      : 'Matching unavailable'
  const liveMessage = refreshing
    ? 'Refreshing GitHub security sources.'
    : loading
      ? 'Loading GitHub security sources.'
      : `${stateCopy.label}. ${visibleAlerts.length} alerts shown.`

  return (
    <section className="security-scan-ui-sources" aria-labelledby="security-scan-sources-title">
      <div className="security-scan-ui-sources-head">
        <div>
          <span className="security-scan-ui-section-label">source reconciliation</span>
          <h3 id="security-scan-sources-title">Security sources</h3>
        </div>
        <Button variant="ghost" size="sm" onClick={onRefresh} disabled={loading || refreshing || loadingMore}>
          <RefreshIcon size={16} className={refreshing ? 'is-spinning' : undefined} />
          {refreshing ? 'refreshing' : stateCopy.action}
        </Button>
      </div>

      <div className="security-scan-ui-source-counts">
        <section>
          <span>Harness review</span>
          <strong>{summary.harness}</strong>
          <p>
            {reconciliation?.harness.status === 'not_available'
              ? 'Harness verification metadata is not available for this run.'
              : `Exact commit scope${
                  reconciliation?.harness.verified_at == null
                    ? '.'
                    : ` · verified ${formatTimestamp(reconciliation.harness.verified_at)}.`
                }`}
          </p>
        </section>
        <section data-state={collectionState}>
          <span>GitHub snapshot</span>
          <strong>{summary.github}</strong>
          <p>
            {stateCopy.label}. {stateCopy.detail}
          </p>
        </section>
      </div>

      <p className="security-scan-ui-source-qualification">{summary.qualification}</p>

      <dl className="security-scan-ui-source-facts">
        <div>
          <dt>latest GitHub snapshot</dt>
          <dd>{latestSnapshotAt == null ? 'Not collected' : formatTimestamp(latestSnapshotAt)}</dd>
        </div>
        <div>
          <dt>source completeness</dt>
          <dd>{stateCopy.label}</dd>
        </div>
        <div>
          <dt>cross-source matching</dt>
          <dd>{matchingLabel}</dd>
        </div>
      </dl>

      {sources.length > 0 ? (
        <div className="security-scan-ui-source-cards">
          {sources.map((source) => (
            <GitHubSourceCard
              key={source.source}
              source={source}
              targetSha={reconciliation?.target_sha ?? ''}
              githubRepository={reconciliation?.github_repository ?? null}
            />
          ))}
        </div>
      ) : null}

      {error ? <p className="security-scan-ui-source-error">{error}</p> : null}

      <div className="security-scan-ui-source-alerts-head">
        <div>
          <strong>GitHub open alert records</strong>
          <span>
            {count.count == null ? 'count unavailable' : `${sourceCountLabel(count.count, count.complete)} open`}
          </span>
        </div>
        <fieldset className="security-scan-ui-source-filters" aria-label="Filter GitHub alerts by source">
          {GITHUB_ALERT_FILTERS.map((option) => (
            <button
              type="button"
              key={option.id}
              aria-pressed={filter === option.id}
              onClick={() => setFilter(option.id as GitHubAlertFilter)}
            >
              {option.label}
            </button>
          ))}
        </fieldset>
      </div>

      {visibleAlerts.length === 0 ? (
        <p className="security-scan-ui-source-empty">
          {collectionState === 'complete' && count.count === 0
            ? 'No open alerts were returned by the collected GitHub sources.'
            : alerts.length > 0
              ? 'No alerts match this source filter.'
              : 'No GitHub alert rows are available for this snapshot.'}
        </p>
      ) : narrow ? (
        <GitHubAlertsList alerts={visibleAlerts} />
      ) : (
        <GitHubAlertsTable alerts={visibleAlerts} />
      )}

      <span className="security-scan-ui-source-live" aria-live="polite" aria-atomic="true">
        {liveMessage}
      </span>

      {remainingAlertCount > 0 || reconciliation?.next_cursor ? (
        <div className="security-scan-ui-source-more">
          <span>
            showing {visibleAlerts.length} of {filteredAlerts.length} loaded alerts
            {reconciliation?.next_cursor ? ' · more available' : ''}
          </span>
          <Button
            variant="ghost"
            size="sm"
            disabled={loadingMore}
            onClick={() => {
              if (remainingAlertCount > 0) {
                setVisibleAlertCount((current) =>
                  nextVisibleAlertCount(current, filteredAlerts.length, ALERT_PAGE_SIZE),
                )
                return
              }
              setRevealAfterLoad(true)
              onLoadMore()
            }}
          >
            {loadingMore
              ? 'loading'
              : remainingAlertCount > 0
                ? `show ${Math.min(ALERT_PAGE_SIZE, remainingAlertCount)} more`
                : 'load more'}
          </Button>
        </div>
      ) : null}
    </section>
  )
}
