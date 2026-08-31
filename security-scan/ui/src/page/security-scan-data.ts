import type { Host } from '@iii-dev/console-ui'
import { withRpcTimeout } from './rpc-timeout.js'
import { assertCompleteGithubSourceSet } from './security-dashboard.js'
import { analysisConversationFromSession } from './view-state.js'

interface OptionalChatHost {
  chat?: {
    selectConversation?(sessionId: string): void
    composerModel?(conversationId?: string | null): string | null
  }
}

function optionalChat(host: Host): OptionalChatHost['chat'] {
  return (host as Host & OptionalChatHost).chat
}

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
  resolved_from_head?: boolean
  mode: ScanMode
  model?: string
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

export const ACTION_KINDS = ['issue', 'fix_pr'] as const
export const ACTION_STATUSES = ['queued', 'preparing', 'awaiting_approval', 'completed', 'failed', 'cancelled'] as const

export type ActionKind = (typeof ACTION_KINDS)[number]
export type ActionStatus = (typeof ACTION_STATUSES)[number]

export interface ActionResult {
  url: string
  kind: string
  branch?: string
  commit_sha?: string
  draft?: boolean
  validation?: string
}

export interface SecurityAction {
  schema_version: string
  action_id: string
  run_id: string
  finding_index: number
  action: ActionKind
  repository: string
  target_sha: string
  status: ActionStatus
  attempt: number
  result?: ActionResult
  error?: RunError
  created_at: number
  updated_at: number
  completed_at?: number
}

export interface ActionRequestResult {
  action_id: string
  run_id: string
  finding_index: number
  action: ActionKind
  status: ActionStatus
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
export const RECONCILIATION_SCOPES = ['repository_default_branch', 'repository_snapshot', 'exact_commit'] as const

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
const SEVERITY_SET = new Set<string>(['critical', 'high', 'medium', 'low', 'info'])
const ASSESSMENT_STATUS_SET = new Set<string>(['assessed', 'not_assessed', 'unknown'])
const GITHUB_ALERT_SOURCE_SET = new Set<string>(GITHUB_ALERT_SOURCES)
const GITHUB_SOURCE_STATUS_SET = new Set<string>(GITHUB_SOURCE_STATUSES)
const RECONCILIATION_SCOPE_SET = new Set<string>(RECONCILIATION_SCOPES)
const MATCHING_STATUS_SET = new Set<string>(['available', 'unavailable'])
const SOURCE_HEALTH_STATUS_SET = new Set<string>(['healthy', 'warning', 'error', 'unknown'])

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
  if (!SEVERITY_SET.has(parsed)) throw new Error(`${label} had an unknown value`)
  return parsed as Severity
}

function enumValue<T extends string>(value: unknown, label: string, allowed: Set<string>): T {
  const parsed = string(value, label)
  if (!allowed.has(parsed)) throw new Error(`${label} had an unknown value`)
  return parsed as T
}

function assessmentStatus(value: unknown, label: string): AssessmentStatus {
  const parsed = string(value, label)
  if (!ASSESSMENT_STATUS_SET.has(parsed)) throw new Error(`${label} had an unknown value`)
  return parsed as AssessmentStatus
}

function parseAssessment(value: unknown, label: string): SecurityAreaAssessment {
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
    vulnerabilities: parseAssessment(item.vulnerabilities, 'vulnerabilities assessment'),
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
  if (!Array.isArray(item.findings)) throw new Error('report findings was not an array')
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
    resolved_from_head: item.resolved_from_head === true,
    mode: scanMode(item.mode, 'scan mode'),
    model: optionalString(item.model, 'analysis model'),
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
  if (scope !== 'exact_commit') throw new Error('Harness reconciliation scope had an unknown value')
  return {
    status,
    verified_count: nullableNumber(item.verified_count, 'Harness verified count'),
    verified_at: nullableNumber(item.verified_at, 'Harness verified at'),
    scope,
  }
}

function parseGitHubSource(value: unknown): GitHubSourceReconciliation {
  const item = record(value, 'GitHub source reconciliation')
  const health = record(item.health, 'GitHub source health')
  return {
    source: enumValue<GitHubAlertSource>(item.source, 'GitHub alert source', GITHUB_ALERT_SOURCE_SET),
    status: enumValue<GitHubSourceStatus>(item.status, 'GitHub source status', GITHUB_SOURCE_STATUS_SET),
    scope: enumValue<ReconciliationScope>(item.scope, 'GitHub source scope', RECONCILIATION_SCOPE_SET),
    collected_at: nullableNumber(item.collected_at, 'GitHub source collected at'),
    record_count: nullableNumber(item.record_count, 'GitHub source record count'),
    health: {
      status: enumValue<'healthy' | 'warning' | 'error' | 'unknown'>(
        health.status,
        'GitHub source health status',
        SOURCE_HEALTH_STATUS_SET,
      ),
      tool: optionalString(health.tool, 'GitHub source tool'),
      commit_sha: optionalString(health.commit_sha, 'GitHub source commit sha'),
      observed_at: optionalString(health.observed_at, 'GitHub source observed at'),
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
  if (lifecycle !== 'open') throw new Error('GitHub alert lifecycle had an unknown value')
  return {
    source: enumValue<GitHubAlertSource>(item.source, 'GitHub alert source', GITHUB_ALERT_SOURCE_SET),
    number: number(item.number, 'GitHub alert number'),
    severity: severity(item.severity, 'GitHub alert severity'),
    lifecycle,
    scope: enumValue<ReconciliationScope>(item.scope, 'GitHub alert scope', RECONCILIATION_SCOPE_SET),
    title: string(item.title, 'GitHub alert title'),
    description: string(item.description, 'GitHub alert description'),
    public_url: string(item.public_url, 'GitHub alert public URL'),
    structured_ids: parseStringArray(item.structured_ids, 'GitHub alert structured ids'),
    path: optionalString(item.path, 'GitHub alert path'),
    start_line: optionalNumber(item.start_line, 'GitHub alert start line'),
    end_line: optionalNumber(item.end_line, 'GitHub alert end line'),
    observed_at: optionalString(item.observed_at, 'GitHub alert observed at'),
  }
}

function parseReconciliation(value: unknown): SecurityReconciliation {
  const item = record(value, 'security-scan::reconciliation response')
  if (!Array.isArray(item.sources)) throw new Error('GitHub sources was not an array')
  if (!Array.isArray(item.records)) throw new Error('GitHub alert records was not an array')
  const matching = record(item.matching, 'matching reconciliation')
  const sources = assertCompleteGithubSourceSet(item.sources.map(parseGitHubSource))
  return {
    schema_version: string(item.schema_version, 'reconciliation schema version'),
    run_id: string(item.run_id, 'reconciliation run id'),
    repository: string(item.repository, 'reconciliation repository'),
    target_sha: string(item.target_sha, 'reconciliation target sha'),
    harness: parseHarnessReconciliation(item.harness),
    github_repository: nullableString(item.github_repository, 'GitHub repository'),
    sources,
    matching: {
      status: enumValue<MatchingStatus>(matching.status, 'matching status', MATCHING_STATUS_SET),
      matched_records: nullableNumber(matching.matched_records, 'matched records'),
    },
    records: item.records.map(parseGitHubAlertRecord),
    next_cursor: nullableString(item.next_cursor, 'reconciliation next cursor'),
  }
}

export async function listRuns(host: Host, filters: RunFilters): Promise<RunSummary[]> {
  const request: Record<string, unknown> = { limit: 200 }
  const repository = filters.repository.trim()
  if (repository) request.repository = repository
  if (filters.status) request.status = filters.status

  const response = record(
    await withRpcTimeout(host.iii.trigger('security-scan::list', request), 'security-scan::list'),
    'security-scan::list response',
  )
  if (!Array.isArray(response.runs)) throw new Error('run list was not an array')
  return response.runs.map(parseSummary)
}

export async function readRun(host: Host, runId: string): Promise<SecurityRun | null> {
  const response = record(
    await withRpcTimeout(host.iii.trigger('security-scan::read', { run_id: runId }), 'security-scan::read'),
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
    await withRpcTimeout(host.iii.trigger('security-scan::reconciliation', request), 'security-scan::reconciliation'),
  )
  if (reconciliation.run_id !== runId) {
    throw new Error('security-scan::reconciliation returned a different run id')
  }
  return reconciliation
}

const COMMIT_SHA = /^[0-9a-f]{40}$/i

export function normalizeCommitSha(value: string): string | null {
  const sha = value.trim().toLowerCase()
  return COMMIT_SHA.test(sha) ? sha : null
}

export interface ScanFormDefaults {
  repositories: string[]
  analysisModel: string | null
}

export async function loadScanFormDefaults(host: Host): Promise<ScanFormDefaults> {
  try {
    const response = record(
      await withRpcTimeout(host.iii.trigger('configuration::get', { id: 'security-scan' }), 'configuration::get'),
      'configuration::get response',
    )
    if (response.value == null) return { repositories: [], analysisModel: null }
    const config = record(response.value, 'security-scan config')
    const repositories = Array.isArray(config.repositories)
      ? config.repositories.map((item, index) => {
          const repository = record(item, `configured repository ${index + 1}`)
          return string(repository.id, 'configured repository id')
        })
      : []
    const analysis = config.analysis == null ? null : record(config.analysis, 'security-scan analysis')
    const analysisModel =
      analysis == null ? null : optionalString(analysis.model, 'operator analysis model')?.trim() || null
    return { repositories, analysisModel: analysisModel || null }
  } catch {
    return { repositories: [], analysisModel: null }
  }
}

export async function loadComposerModel(host: Host, conversationId: string | null | undefined): Promise<string | null> {
  const live = optionalChat(host)?.composerModel?.(conversationId)
  if (typeof live === 'string' && live.trim()) return live.trim()
  const sessionId = conversationId?.trim()
  if (!sessionId) return null
  try {
    const response = await withRpcTimeout(
      host.iii.trigger<{ meta?: { metadata?: Record<string, unknown> } } | null>('session::get', {
        session_id: sessionId,
      }),
      'session::get',
    )
    if (response == null) return null
    const model = response.meta?.metadata?.model
    return typeof model === 'string' && model.trim() ? model.trim() : null
  } catch {
    return null
  }
}

export function supportsLiveComposerModel(host: Host): boolean {
  return typeof optionalChat(host)?.composerModel === 'function'
}

export interface CatalogModel {
  /** `provider::id` — the key `security-scan::request` splits back apart. */
  key: string
  id: string
  provider: string
  label: string
}

const MODELS_CHANGED_FN = 'security-scan-ui::models-changed'

/**
 * The router catalog behind the analysis-model picker. An unreachable or
 * empty router yields an empty list, and the form falls back to the operator
 * default rather than blocking the scan.
 */
export async function loadModelCatalog(host: Host): Promise<CatalogModel[]> {
  try {
    const response = await withRpcTimeout(
      host.iii.trigger<{ models?: unknown }>('router::models::list', {}),
      'router::models::list',
    )
    const rows = response?.models
    if (!Array.isArray(rows)) return []
    const models: CatalogModel[] = []
    for (const raw of rows) {
      if (!raw || typeof raw !== 'object') continue
      const row = raw as Record<string, unknown>
      const id = typeof row.id === 'string' ? row.id.trim() : ''
      const provider = typeof row.provider === 'string' ? row.provider.trim() : ''
      if (!id || !provider) continue
      const displayName = typeof row.display_name === 'string' && row.display_name.trim() ? row.display_name.trim() : id
      models.push({
        key: `${provider}::${id}`,
        id,
        provider,
        label: `${provider} · ${displayName}`,
      })
    }
    models.sort((left, right) => left.key.localeCompare(right.key))
    return models
  } catch {
    return []
  }
}

/**
 * Re-read the catalog when llm-router announces a provider reconcile
 * (credential added or removed, `refresh_models`, provider worker added or
 * removed). The trigger is the router's own fan-out type, so the picker never
 * polls for a catalog that changes only on operator action.
 */
export function subscribeModelCatalog(host: Host, onChange: () => void): () => void {
  let offHandler: (() => void) | undefined
  let offTrigger: (() => void) | undefined
  try {
    offHandler = host.iii.on(MODELS_CHANGED_FN, () => onChange())
    offTrigger = host.iii.registerTrigger({
      type: 'router::models::changed',
      function_id: `${MODELS_CHANGED_FN}::${host.iii.browserId}`,
      config: {},
    })
  } catch {
    offTrigger?.()
    offHandler?.()
    return () => {}
  }
  let disposed = false
  return () => {
    if (disposed) return
    disposed = true
    offTrigger?.()
    offHandler?.()
  }
}

function parseRequestResponse(value: unknown, label: string): RetryResult {
  const response = record(value, `${label} response`)
  if (typeof response.deduplicated !== 'boolean') {
    throw new Error(`${label} deduplicated flag was not a boolean`)
  }
  return {
    run_id: string(response.run_id, `${label} run id`),
    status: runStatus(response.status, `${label} run status`),
    deduplicated: response.deduplicated,
  }
}

export async function requestNewRun(
  host: Host,
  request: { repository: string; target_sha?: string; mode: ScanMode; model?: string },
): Promise<RetryResult> {
  return parseRequestResponse(
    await withRpcTimeout(
      host.iii.trigger('security-scan::request', {
        repository: request.repository,
        mode: request.mode,
        ...(request.target_sha ? { target_sha: request.target_sha } : {}),
        ...(request.model ? { model: request.model } : {}),
      }),
      'security-scan::request',
    ),
    'security-scan::request',
  )
}

export async function requestRunMode(host: Host, run: RunSummary | SecurityRun, mode: ScanMode): Promise<RetryResult> {
  return requestNewRun(host, {
    repository: run.repository,
    target_sha: run.target_sha,
    mode,
    ...(run.model ? { model: run.model } : {}),
  })
}

export function retryRun(host: Host, run: RunSummary | SecurityRun): Promise<RetryResult> {
  return requestRunMode(host, run, run.mode)
}

export async function cancelRun(host: Host, runId: string): Promise<RetryResult> {
  return parseRequestResponse(
    await withRpcTimeout(host.iii.trigger('security-scan::cancel', { run_id: runId }), 'security-scan::cancel'),
    'security-scan::cancel',
  )
}

const ACTION_KIND_SET = new Set<string>(ACTION_KINDS)
const ACTION_STATUS_SET = new Set<string>(ACTION_STATUSES)

export async function requestFindingAction(
  host: Host,
  runId: string,
  findingIndex: number,
  action: ActionKind,
): Promise<ActionRequestResult> {
  const response = record(
    await withRpcTimeout(
      host.iii.trigger('security-scan::action', {
        run_id: runId,
        finding_index: findingIndex,
        action,
      }),
      'security-scan::action',
    ),
    'security-scan::action response',
  )
  if (typeof response.deduplicated !== 'boolean') {
    throw new Error('action deduplicated flag was not a boolean')
  }
  return {
    action_id: string(response.action_id, 'action id'),
    run_id: string(response.run_id, 'action run id'),
    finding_index: number(response.finding_index, 'action finding index'),
    action: enumValue<ActionKind>(response.action, 'action kind', ACTION_KIND_SET),
    status: enumValue<ActionStatus>(response.status, 'action status', ACTION_STATUS_SET),
    deduplicated: response.deduplicated,
  }
}

export async function readFindingAction(host: Host, actionId: string): Promise<SecurityAction | null> {
  const response = record(
    await withRpcTimeout(
      host.iii.trigger('security-scan::action-read', { action_id: actionId }),
      'security-scan::action-read',
    ),
    'security-scan::action-read response',
  )
  return response.action == null ? null : parseAction(response.action)
}

function parseAction(value: unknown): SecurityAction {
  const item = record(value, 'security-scan action')
  return {
    schema_version: string(item.schema_version, 'action schema version'),
    action_id: string(item.action_id, 'action id'),
    run_id: string(item.run_id, 'action run id'),
    finding_index: number(item.finding_index, 'action finding index'),
    action: enumValue<ActionKind>(item.action, 'action kind', ACTION_KIND_SET),
    repository: string(item.repository, 'action repository'),
    target_sha: string(item.target_sha, 'action target sha'),
    status: enumValue<ActionStatus>(item.status, 'action status', ACTION_STATUS_SET),
    attempt: number(item.attempt, 'action attempt'),
    result: item.result == null ? undefined : parseActionResult(item.result),
    error: parseError(item.error),
    created_at: number(item.created_at, 'action created at'),
    updated_at: number(item.updated_at, 'action updated at'),
    completed_at: item.completed_at == null ? undefined : number(item.completed_at, 'action completed at'),
  }
}

function parseActionResult(value: unknown): ActionResult {
  const item = record(value, 'action result')
  const url = string(item.url, 'action result URL')
  if (!isSafeGitHubHttpsUrl(url)) {
    throw new Error('action result URL was not a github.com https URL')
  }
  return {
    url,
    kind: string(item.kind, 'action result kind'),
    branch: optionalString(item.branch, 'action result branch'),
    commit_sha: optionalString(item.commit_sha, 'action result commit sha'),
    draft: item.draft == null ? undefined : booleanValue(item.draft, 'action draft'),
    validation: optionalString(item.validation, 'action validation'),
  }
}

export function isSafeGitHubHttpsUrl(url: string): boolean {
  try {
    const parsed = new URL(url)
    if (parsed.protocol !== 'https:') return false
    if (parsed.username || parsed.password) return false
    if (parsed.hostname !== 'github.com' && parsed.hostname !== 'www.github.com') {
      return false
    }
    return !parsed.pathname.includes('\\') && !parsed.pathname.includes('//')
  } catch {
    return false
  }
}

function booleanValue(value: unknown, label: string): boolean {
  if (typeof value !== 'boolean') throw new Error(`${label} was not a boolean`)
  return value
}

export function isTerminal(status: RunStatus): boolean {
  return status === 'completed' || status === 'failed' || status === 'cancelled'
}

export function canCancelRun(status: RunStatus): boolean {
  return !isTerminal(status) && status !== 'cancelling'
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

export function commitScopeLabel(run: { target_sha: string; resolved_from_head?: boolean }): string {
  const sha = shortSha(run.target_sha)
  return run.resolved_from_head ? `HEAD -> ${sha}` : sha
}

export const ANALYSIS_SESSION_PREFIX = 'security-scan-analysis-'
export const ANALYSIS_SESSION_TITLE = 'Security review'
const ANALYSIS_SESSION_LIMIT = 200

export interface AnalysisConversation {
  sessionId: string
  runId: string
}

export async function listAnalysisConversations(host: Host, runId?: string): Promise<AnalysisConversation[]> {
  const id = runId?.trim()
  const response = record(
    await withRpcTimeout(
      host.iii.trigger('session::list', {
        limit: ANALYSIS_SESSION_LIMIT,
        order: 'updated_desc',
        metadata: {
          security_scan: true,
          ...(id ? { security_scan_run_id: id } : {}),
        },
      }),
      'session::list',
    ),
    'session::list response',
  )
  if (!Array.isArray(response.sessions)) throw new Error('session list was not an array')
  return response.sessions
    .map(analysisConversationFromSession)
    .filter((session): session is AnalysisConversation => session !== null)
}

export async function analysisConversationRunId(host: Host, sessionId: string): Promise<string | null> {
  const id = sessionId.trim()
  if (!id) return null
  const response = await withRpcTimeout(
    host.iii.trigger<{ meta?: unknown } | null>('session::get', { session_id: id }),
    'session::get',
  )
  if (!response?.meta) return null
  return analysisConversationFromSession(response.meta)?.runId ?? null
}

export async function ensureAnalysisConversation(host: Host, runId: string): Promise<string | null> {
  const id = runId.trim()
  if (!id) return null
  const existing = await listAnalysisConversations(host, id)
  if (existing[0]) return existing[0].sessionId
  const response = record(
    await withRpcTimeout(
      host.iii.trigger('security-scan::analysis-chat', { run_id: id }),
      'security-scan::analysis-chat',
    ),
    'security-scan::analysis-chat response',
  )
  if (response.available !== true) return null
  return (await listAnalysisConversations(host, id))[0]?.sessionId ?? null
}

export function isSecurityAnalysisSession(event: { session_id?: string; title?: string }): boolean {
  const sessionId = typeof event.session_id === 'string' ? event.session_id.trim() : ''
  const title = typeof event.title === 'string' ? event.title.trim() : ''
  return sessionId.startsWith(ANALYSIS_SESSION_PREFIX) && title === ANALYSIS_SESSION_TITLE
}

export function openAnalysisConversation(host: Host, sessionId: string): boolean {
  const id = sessionId.trim()
  const select = optionalChat(host)?.selectConversation
  if (!id || typeof select !== 'function') return false
  select(id)
  return true
}

export function formatRelativeTime(timestamp: number, now = Date.now()): string {
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
