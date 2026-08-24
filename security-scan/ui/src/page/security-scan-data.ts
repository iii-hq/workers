import type { Host } from '@iii-dev/console-ui'
import { withRpcTimeout } from './rpc-timeout.js'
import { assertCompleteGithubSourceSet } from './security-dashboard.js'

export const RUN_STATUSES = [
  'queued',
  'materializing',
  'materialized',
  'dispatching',
  'analyzing',
  'completed',
  'failed',
  'cancelling',
  'cancelled',
] as const

export type RunStatus = (typeof RUN_STATUSES)[number]
export type ScanMode = 'scan' | 'suggest'
export type Severity = 'critical' | 'high' | 'medium' | 'low' | 'info'
export type AssessmentStatus = 'assessed' | 'not_assessed' | 'unknown'

export interface RunError {
  code: string
  message: string
  retryable: boolean
}

export interface RunSummary {
  run_id: string
  repository: string
  target_sha: string
  mode: ScanMode
  status: RunStatus
  attempt: number
  finding_count: number
  error?: RunError
  created_at: number
  updated_at: number
  completed_at?: number
}

export interface FindingLocation {
  path: string
  line_start?: number
  line_end?: number
}

export interface SecurityFinding {
  rule_id: string
  severity: Severity
  title: string
  description: string
  evidence: string
  location?: FindingLocation
  remediation: string
  suggested_patch?: string
}

export interface SecurityAreaAssessment {
  status: AssessmentStatus
  reason?: string
}

export interface SecurityAssessments {
  vulnerabilities: SecurityAreaAssessment
  dependencies: SecurityAreaAssessment
  secrets: SecurityAreaAssessment
  supply_chain: SecurityAreaAssessment
}

export interface SecurityReport {
  summary: string
  assessments: SecurityAssessments
  findings: SecurityFinding[]
}

export interface SecurityRun extends Omit<RunSummary, 'finding_count'> {
  schema_version: string
  report?: SecurityReport
}

export interface RunFilters {
  repository: string
  status: RunStatus | ''
}

export interface RetryResult {
  run_id: string
  status: RunStatus
  deduplicated: boolean
}

export const GITHUB_ALERT_SOURCES = ['dependabot', 'code_scanning'] as const
export const GITHUB_SOURCE_STATUSES = [
  'complete',
  'partial',
  'unavailable',
  'authentication_required',
  'permission_denied',
  'disabled',
  'not_configured',
  'not_collected',
] as const
export const RECONCILIATION_SCOPES = [
  'repository_default_branch',
  'repository_snapshot',
  'exact_commit',
] as const

export type GitHubAlertSource = (typeof GITHUB_ALERT_SOURCES)[number]
export type GitHubSourceStatus = (typeof GITHUB_SOURCE_STATUSES)[number]
export type ReconciliationScope = (typeof RECONCILIATION_SCOPES)[number]
export type MatchingStatus = 'available' | 'unavailable'

export interface HarnessReconciliation {
  status: 'verified' | 'not_available'
  verified_count: number | null
  verified_at: number | null
  scope: 'exact_commit'
}

export interface GitHubSourceReconciliation {
  source: GitHubAlertSource
  status: GitHubSourceStatus
  scope: ReconciliationScope
  collected_at: number | null
  record_count: number | null
  health: {
    status: 'healthy' | 'warning' | 'error' | 'unknown'
    tool?: string
    commit_sha?: string
    observed_at?: string
  }
}

export interface GitHubAlertRecord {
  source: GitHubAlertSource
  number: number
  severity: Severity
  lifecycle: 'open'
  scope: ReconciliationScope
  title: string
  description: string
  public_url: string
  structured_ids: string[]
  path?: string
  start_line?: number
  end_line?: number
  observed_at?: string
}

export interface SecurityReconciliation {
  schema_version: string
  run_id: string
  repository: string
  target_sha: string
  harness: HarnessReconciliation
  github_repository: string | null
  sources: GitHubSourceReconciliation[]
  matching: {
    status: MatchingStatus
    matched_records: number | null
  }
  records: GitHubAlertRecord[]
  next_cursor: string | null
}

export interface ReconciliationRequestOptions {
  refresh?: boolean
  source?: GitHubAlertSource
  severity?: Severity
  lifecycle?: 'open'
  cursor?: string
  limit?: number
}

type JsonRecord = Record<string, unknown>

const STATUS_SET = new Set<string>(RUN_STATUSES)
const MODE_SET = new Set<string>(['scan', 'suggest'])
const SEVERITY_SET = new Set<string>([
  'critical',
  'high',
  'medium',
  'low',
  'info',
])
const ASSESSMENT_STATUS_SET = new Set<string>([
  'assessed',
  'not_assessed',
  'unknown',
])
const GITHUB_ALERT_SOURCE_SET = new Set<string>(GITHUB_ALERT_SOURCES)
const GITHUB_SOURCE_STATUS_SET = new Set<string>(GITHUB_SOURCE_STATUSES)
const RECONCILIATION_SCOPE_SET = new Set<string>(RECONCILIATION_SCOPES)
const MATCHING_STATUS_SET = new Set<string>(['available', 'unavailable'])
const SOURCE_HEALTH_STATUS_SET = new Set<string>([
  'healthy',
  'warning',
  'error',
  'unknown',
])

function record(value: unknown, label: string): JsonRecord {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${label} was not an object`)
  }
  return value as JsonRecord
}

function string(value: unknown, label: string): string {
  if (typeof value !== 'string') throw new Error(`${label} was not a string`)
  return value
}

function number(value: unknown, label: string): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    throw new Error(`${label} was not a number`)
  }
  return value
}

function optionalNumber(value: unknown, label: string): number | undefined {
  return value == null ? undefined : number(value, label)
}

function optionalString(value: unknown, label: string): string | undefined {
  return value == null ? undefined : string(value, label)
}

function nullableNumber(value: unknown, label: string): number | null {
  return value == null ? null : number(value, label)
}

function nullableString(value: unknown, label: string): string | null {
  return value == null ? null : string(value, label)
}

function runStatus(value: unknown, label: string): RunStatus {
  const parsed = string(value, label)
  if (!STATUS_SET.has(parsed)) throw new Error(`${label} had an unknown value`)
  return parsed as RunStatus
}

function scanMode(value: unknown, label: string): ScanMode {
  const parsed = string(value, label)
  if (!MODE_SET.has(parsed)) throw new Error(`${label} had an unknown value`)
  return parsed as ScanMode
}

function severity(value: unknown, label: string): Severity {
  const parsed = string(value, label)
  if (!SEVERITY_SET.has(parsed))
    throw new Error(`${label} had an unknown value`)
  return parsed as Severity
}

function enumValue<T extends string>(
  value: unknown,
  label: string,
  allowed: Set<string>,
): T {
  const parsed = string(value, label)
  if (!allowed.has(parsed)) throw new Error(`${label} had an unknown value`)
  return parsed as T
}

function assessmentStatus(value: unknown, label: string): AssessmentStatus {
  const parsed = string(value, label)
  if (!ASSESSMENT_STATUS_SET.has(parsed))
    throw new Error(`${label} had an unknown value`)
  return parsed as AssessmentStatus
}

function parseAssessment(
  value: unknown,
  label: string,
): SecurityAreaAssessment {
  if (value == null) return { status: 'unknown' }
  const item = record(value, label)
  return {
    status: assessmentStatus(item.status, `${label} status`),
    reason: optionalString(item.reason, `${label} reason`),
  }
}

function parseAssessments(value: unknown): SecurityAssessments {
  if (value == null) {
    return {
      vulnerabilities: { status: 'unknown' },
      dependencies: { status: 'unknown' },
      secrets: { status: 'unknown' },
      supply_chain: { status: 'unknown' },
    }
  }
  const item = record(value, 'security assessments')
  return {
    vulnerabilities: parseAssessment(
      item.vulnerabilities,
      'vulnerabilities assessment',
    ),
    dependencies: parseAssessment(item.dependencies, 'dependencies assessment'),
    secrets: parseAssessment(item.secrets, 'secrets assessment'),
    supply_chain: parseAssessment(item.supply_chain, 'supply-chain assessment'),
  }
}

function parseError(value: unknown): RunError | undefined {
  if (value == null) return undefined
  const item = record(value, 'run error')
  if (typeof item.retryable !== 'boolean') {
    throw new Error('run error retryable flag was not a boolean')
  }
  return {
    code: string(item.code, 'run error code'),
    message: string(item.message, 'run error message'),
    retryable: item.retryable,
  }
}

function parseLocation(value: unknown): FindingLocation | undefined {
  if (value == null) return undefined
  const item = record(value, 'finding location')
  return {
    path: string(item.path, 'finding location path'),
    line_start: optionalNumber(item.line_start, 'finding start line'),
    line_end: optionalNumber(item.line_end, 'finding end line'),
  }
}

function parseFinding(value: unknown): SecurityFinding {
  const item = record(value, 'finding')
  return {
    rule_id: string(item.rule_id, 'finding rule id'),
    severity: severity(item.severity, 'finding severity'),
    title: string(item.title, 'finding title'),
    description: string(item.description, 'finding description'),
    evidence: string(item.evidence, 'finding evidence'),
    location: parseLocation(item.location),
    remediation: string(item.remediation, 'finding remediation'),
    suggested_patch: optionalString(item.suggested_patch, 'suggested patch'),
  }
}

function parseReport(value: unknown): SecurityReport | undefined {
  if (value == null) return undefined
  const item = record(value, 'security report')
  if (!Array.isArray(item.findings))
    throw new Error('report findings was not an array')
  return {
    summary: string(item.summary, 'report summary'),
    assessments: parseAssessments(item.assessments),
    findings: item.findings.map(parseFinding),
  }
}

function parseRunBase(value: unknown): Omit<RunSummary, 'finding_count'> {
  const item = record(value, 'security scan run')
  return {
    run_id: string(item.run_id, 'run id'),
    repository: string(item.repository, 'repository'),
    target_sha: string(item.target_sha, 'target sha'),
    mode: scanMode(item.mode, 'scan mode'),
    status: runStatus(item.status, 'run status'),
    attempt: number(item.attempt, 'attempt'),
    error: parseError(item.error),
    created_at: number(item.created_at, 'created at'),
    updated_at: number(item.updated_at, 'updated at'),
    completed_at: optionalNumber(item.completed_at, 'completed at'),
  }
}

function parseSummary(value: unknown): RunSummary {
  const item = record(value, 'security scan summary')
  return {
    ...parseRunBase(item),
    finding_count: number(item.finding_count, 'finding count'),
  }
}

function parseRun(value: unknown): SecurityRun {
  const item = record(value, 'security scan run')
  return {
    ...parseRunBase(item),
    schema_version: string(item.schema_version, 'schema version'),
    report: parseReport(item.report),
  }
}

function parseHarnessReconciliation(value: unknown): HarnessReconciliation {
  const item = record(value, 'Harness reconciliation')
  const status = enumValue<'verified' | 'not_available'>(
    item.status,
    'Harness reconciliation status',
    new Set(['verified', 'not_available']),
  )
  const scope = string(item.scope, 'Harness reconciliation scope')
  if (scope !== 'exact_commit')
    throw new Error('Harness reconciliation scope had an unknown value')
  return {
    status,
    verified_count: nullableNumber(
      item.verified_count,
      'Harness verified count',
    ),
    verified_at: nullableNumber(item.verified_at, 'Harness verified at'),
    scope,
  }
}

function parseGitHubSource(value: unknown): GitHubSourceReconciliation {
  const item = record(value, 'GitHub source reconciliation')
  const health = record(item.health, 'GitHub source health')
  return {
    source: enumValue<GitHubAlertSource>(
      item.source,
      'GitHub alert source',
      GITHUB_ALERT_SOURCE_SET,
    ),
    status: enumValue<GitHubSourceStatus>(
      item.status,
      'GitHub source status',
      GITHUB_SOURCE_STATUS_SET,
    ),
    scope: enumValue<ReconciliationScope>(
      item.scope,
      'GitHub source scope',
      RECONCILIATION_SCOPE_SET,
    ),
    collected_at: nullableNumber(
      item.collected_at,
      'GitHub source collected at',
    ),
    record_count: nullableNumber(
      item.record_count,
      'GitHub source record count',
    ),
    health: {
      status: enumValue<'healthy' | 'warning' | 'error' | 'unknown'>(
        health.status,
        'GitHub source health status',
        SOURCE_HEALTH_STATUS_SET,
      ),
      tool: optionalString(health.tool, 'GitHub source tool'),
      commit_sha: optionalString(health.commit_sha, 'GitHub source commit sha'),
      observed_at: optionalString(
        health.observed_at,
        'GitHub source observed at',
      ),
    },
  }
}

function parseStringArray(value: unknown, label: string): string[] {
  if (!Array.isArray(value)) throw new Error(`${label} was not an array`)
  return value.map((item, index) => string(item, `${label} item ${index + 1}`))
}

function parseGitHubAlertRecord(value: unknown): GitHubAlertRecord {
  const item = record(value, 'GitHub alert record')
  const lifecycle = string(item.lifecycle, 'GitHub alert lifecycle')
  if (lifecycle !== 'open')
    throw new Error('GitHub alert lifecycle had an unknown value')
  return {
    source: enumValue<GitHubAlertSource>(
      item.source,
      'GitHub alert source',
      GITHUB_ALERT_SOURCE_SET,
    ),
    number: number(item.number, 'GitHub alert number'),
    severity: severity(item.severity, 'GitHub alert severity'),
    lifecycle,
    scope: enumValue<ReconciliationScope>(
      item.scope,
      'GitHub alert scope',
      RECONCILIATION_SCOPE_SET,
    ),
    title: string(item.title, 'GitHub alert title'),
    description: string(item.description, 'GitHub alert description'),
    public_url: string(item.public_url, 'GitHub alert public URL'),
    structured_ids: parseStringArray(
      item.structured_ids,
      'GitHub alert structured ids',
    ),
    path: optionalString(item.path, 'GitHub alert path'),
    start_line: optionalNumber(item.start_line, 'GitHub alert start line'),
    end_line: optionalNumber(item.end_line, 'GitHub alert end line'),
    observed_at: optionalString(item.observed_at, 'GitHub alert observed at'),
  }
}

function parseReconciliation(value: unknown): SecurityReconciliation {
  const item = record(value, 'security-scan::reconciliation response')
  if (!Array.isArray(item.sources))
    throw new Error('GitHub sources was not an array')
  if (!Array.isArray(item.records))
    throw new Error('GitHub alert records was not an array')
  const matching = record(item.matching, 'matching reconciliation')
  const sources = assertCompleteGithubSourceSet(
    item.sources.map(parseGitHubSource),
  )
  return {
    schema_version: string(
      item.schema_version,
      'reconciliation schema version',
    ),
    run_id: string(item.run_id, 'reconciliation run id'),
    repository: string(item.repository, 'reconciliation repository'),
    target_sha: string(item.target_sha, 'reconciliation target sha'),
    harness: parseHarnessReconciliation(item.harness),
    github_repository: nullableString(
      item.github_repository,
      'GitHub repository',
    ),
    sources,
    matching: {
      status: enumValue<MatchingStatus>(
        matching.status,
        'matching status',
        MATCHING_STATUS_SET,
      ),
      matched_records: nullableNumber(
        matching.matched_records,
        'matched records',
      ),
    },
    records: item.records.map(parseGitHubAlertRecord),
    next_cursor: nullableString(item.next_cursor, 'reconciliation next cursor'),
  }
}

export async function listRuns(
  host: Host,
  filters: RunFilters,
): Promise<RunSummary[]> {
  const request: Record<string, unknown> = { limit: 200 }
  const repository = filters.repository.trim()
  if (repository) request.repository = repository
  if (filters.status) request.status = filters.status

  const response = record(
    await withRpcTimeout(
      host.iii.trigger('security-scan::list', request),
      'security-scan::list',
    ),
    'security-scan::list response',
  )
  if (!Array.isArray(response.runs))
    throw new Error('run list was not an array')
  return response.runs.map(parseSummary)
}

export async function readRun(
  host: Host,
  runId: string,
): Promise<SecurityRun | null> {
  const response = record(
    await withRpcTimeout(
      host.iii.trigger('security-scan::read', { run_id: runId }),
      'security-scan::read',
    ),
    'security-scan::read response',
  )
  return response.run == null ? null : parseRun(response.run)
}

export async function readReconciliation(
  host: Host,
  runId: string,
  options: ReconciliationRequestOptions = {},
): Promise<SecurityReconciliation> {
  const request: Record<string, unknown> = { run_id: runId }
  if (options.refresh === true) request.refresh = true
  if (options.source) request.source = options.source
  if (options.severity) request.severity = options.severity
  if (options.lifecycle) request.lifecycle = options.lifecycle
  if (options.cursor) request.cursor = options.cursor
  if (options.limit != null) request.limit = options.limit
  const reconciliation = parseReconciliation(
    await withRpcTimeout(
      host.iii.trigger('security-scan::reconciliation', request),
      'security-scan::reconciliation',
    ),
  )
  if (reconciliation.run_id !== runId) {
    throw new Error('security-scan::reconciliation returned a different run id')
  }
  return reconciliation
}

export async function requestRunMode(
  host: Host,
  run: RunSummary | SecurityRun,
  mode: ScanMode,
): Promise<RetryResult> {
  const response = record(
    await withRpcTimeout(
      host.iii.trigger('security-scan::request', {
        repository: run.repository,
        target_sha: run.target_sha,
        mode,
      }),
      'security-scan::request',
    ),
    'security-scan::request response',
  )
  if (typeof response.deduplicated !== 'boolean') {
    throw new Error('retry deduplicated flag was not a boolean')
  }
  return {
    run_id: string(response.run_id, 'retry run id'),
    status: runStatus(response.status, 'retry run status'),
    deduplicated: response.deduplicated,
  }
}

export function retryRun(
  host: Host,
  run: RunSummary | SecurityRun,
): Promise<RetryResult> {
  return requestRunMode(host, run, run.mode)
}

export function isTerminal(status: RunStatus): boolean {
  return status === 'completed' || status === 'failed' || status === 'cancelled'
}

export function shortSha(sha: string): string {
  return sha.slice(0, 8)
}

export function formatStatus(status: RunStatus): string {
  return status.replace('_', ' ')
}

export function formatLocation(location?: FindingLocation): string {
  if (!location) return 'repository-wide'
  if (location.line_start == null) return location.path
  if (location.line_end != null && location.line_end !== location.line_start) {
    return `${location.path}:${location.line_start}-${location.line_end}`
  }
  return `${location.path}:${location.line_start}`
}

export function formatTimestamp(timestamp: number): string {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(timestamp))
}

export function formatRelativeTime(
  timestamp: number,
  now = Date.now(),
): string {
  const elapsed = Math.max(0, now - timestamp)
  const minutes = Math.floor(elapsed / 60_000)
  if (minutes < 1) return 'now'
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  if (days < 14) return `${days}d ago`
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
  }).format(new Date(timestamp))
}
