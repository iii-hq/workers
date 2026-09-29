/** The worker's wire contract, as the page sees it. */
import type { ExtensionIii } from '@iii-dev/console-ui'
import { FN, RPC_TIMEOUT_MS } from './shared'

export type ErrorSource = 'trace' | 'log' | 'harness-turn' | 'report'
export type GroupStatus =
  | 'new'
  | 'investigating'
  | 'diagnosed'
  | 'resolved'
  | 'regressed'
  | 'ignored'

export type IgnoreRule =
  | { kind: 'forever' }
  | { kind: 'occurrences'; count: number }
  | { kind: 'version_change' }

export interface GroupSummary {
  id: string
  fingerprint: string
  source: ErrorSource
  namespace: string
  service_name: string
  function_id?: string
  exception_type?: string
  title: string
  status: GroupStatus
  occurrence_count: number
  sessions_affected: number
  first_seen_ms: number
  last_seen_ms: number
  first_version?: string
  last_version?: string
  sparkline: number[]
  has_diagnosis: boolean
  ignore_rule?: IgnoreRule
  regressed_at_ms?: number
  resolved_at_ms?: number
  resolved_version?: string
  resolve_until_version_change?: boolean
}

export interface OccurrenceSummary {
  id: string
  at_ms: number
  source: ErrorSource
  trace_id?: string
  span_id?: string
  session_id?: string
  turn_id?: string
  worker_version?: string
  message: string
  has_evidence: boolean
  settled: boolean
  namespace_ambiguous: boolean
}

export interface DiagnosisEvidence {
  kind: 'code' | 'trace' | 'log'
  path?: string
  line?: number
  span_id?: string
  excerpt: string
  why: string
}

export interface Diagnosis {
  summary: string
  category: 'bug' | 'configuration' | 'dependency' | 'transient' | 'expected' | 'unknown'
  confidence: 'high' | 'medium' | 'low'
  root_cause: { description: string; evidence: DiagnosisEvidence[] }
  proposed_fix?: {
    description: string
    files: string[]
    risk: 'low' | 'medium' | 'high'
    steps: string[]
  }
  reproduction?: string
  missing_evidence?: string[]
  version_note?: string
  related_groups?: string[]
}

export interface DiagnosisRecord {
  id: string
  investigation_id: string
  group_id: string
  session_id: string
  turn_id?: string
  source: 'first_pass' | 'conversation'
  model: string
  created_ms: number
  valid: boolean
  diagnosis?: Diagnosis
  raw_result?: string
}

export type InvestigationStatus = 'running' | 'completed' | 'failed' | 'cancelled' | 'open'

export interface Investigation {
  id: string
  group_id: string
  occurrence_id: string
  session_id: string
  mode: 'assisted' | 'chat'
  first_pass_turn_id?: string
  model: string
  provider?: string
  repository_id?: string
  checkout_ref?: string
  investigated_version?: string
  status: InvestigationStatus
  error?: string
  turns?: number
  duration_ms?: number
  cost_usd?: number
  created_ms: number
  finished_ms?: number
}

export interface EvidenceSpan {
  span_id: string
  parent_span_id?: string
  name: string
  service_name: string
  function_id?: string
  start_time_unix_nano: number
  end_time_unix_nano: number
  status: string
  status_description?: string
  attributes: Record<string, string>
  events: { name: string; timestamp_unix_nano: number; attributes: Record<string, string> }[]
  depth: number
}

export interface EvidenceBundle {
  version: number
  captured_at_ms: number
  settled: boolean
  trace_id: string
  origin_span_id: string
  propagated_through: string[]
  trace_tags: Record<string, string>
  spans: EvidenceSpan[]
  logs: {
    timestamp_unix_nano: number
    severity_text: string
    body: string
    span_id?: string
    attributes: Record<string, string>
  }[]
  worker: { service_name: string; version?: string }
  truncated: { spans: number; logs: number; attributes: number }
}

export interface GroupTransition {
  id: string
  from_status?: GroupStatus
  to_status: GroupStatus
  reason?: string
  /** A role: `ingest`, `agent`, `investigation` or `console`. */
  actor: string
  at_ms: number
}

export interface GroupDetail {
  group: GroupSummary
  message_sample: string
  latest_occurrence?: OccurrenceSummary
  diagnosis?: DiagnosisRecord
  active_investigation?: Investigation
  latest_investigation?: Investigation
  trace_available: boolean
}

export interface StatusResponse {
  enabled: boolean
  engine: { trace_store: 'memory' | 'disabled' | 'unknown'; logs: boolean }
  sources: { trace: boolean; log: boolean }
  ingest: Record<string, number | undefined>
  groups: { open: number; regressed: number; ignored: number; resolved: number; last_seen_ms?: number }
  investigations: { running: number; open_sessions: number }
  repositories: { id: string; path: string; exists: boolean; workers: string[] }[]
  config_error?: string
}

export interface GroupsListRequest {
  status?: GroupStatus[]
  service_name?: string
  since_ms?: number
  search?: string
  offset?: number
  limit?: number
}

/** Typed calls into the worker. Nothing else in the page talks to `iii`. */
export function client(iii: ExtensionIii) {
  const call = <T,>(functionId: string, payload: Record<string, unknown> = {}) =>
    iii.trigger<T>(functionId, payload, { timeoutMs: RPC_TIMEOUT_MS })

  return {
    status: () => call<StatusResponse>(FN.status),
    groups: (request: GroupsListRequest) =>
      call<{ groups: GroupSummary[]; total: number }>(FN.groupsList, { ...request }),
    group: (group_id: string) => call<GroupDetail>(FN.groupsGet, { group_id }),
    occurrences: (group_id: string, limit = 50) =>
      call<{ occurrences: OccurrenceSummary[]; total: number }>(FN.occurrences, {
        group_id,
        limit,
      }),
    history: (group_id: string, limit = 50) =>
      call<{ transitions: GroupTransition[]; total: number }>(FN.history, { group_id, limit }),
    diagnoses: (group_id: string, limit = 20) =>
      call<{ diagnoses: DiagnosisRecord[]; total: number }>(FN.diagnoses, { group_id, limit }),
    evidence: (occurrence_id: string) =>
      call<{ evidence?: EvidenceBundle; pruned: boolean }>(FN.evidence, { occurrence_id }),
    resolve: (group_id: string, until_version_change: boolean) =>
      call<{ group_id: string; status: GroupStatus }>(FN.resolve, {
        group_id,
        until_version_change,
      }),
    ignore: (group_id: string, rule: IgnoreRule) =>
      call<{ group_id: string; status: GroupStatus }>(FN.ignore, { group_id, rule }),
    unignore: (group_id: string) =>
      call<{ group_id: string; status: GroupStatus }>(FN.unignore, { group_id }),
    reopen: (group_id: string) =>
      call<{ group_id: string; status: GroupStatus }>(FN.reopen, { group_id }),
    investigate: (group_id: string, mode: 'assisted' | 'chat', model?: string, provider?: string) =>
      call<{
        investigation_id: string
        session_id: string
        first_pass_turn_id?: string
        existing: boolean
      }>(FN.investigate, { group_id, mode, model, provider }),
    investigation: (investigation_id: string) =>
      call<{ investigation: Investigation; diagnoses: DiagnosisRecord[] }>(FN.investigationsGet, {
        investigation_id,
      }),
    investigations: (group_id: string) =>
      call<{ investigations: Investigation[]; total: number }>(FN.investigationsList, {
        group_id,
        limit: 20,
      }),
    cancel: (investigation_id: string) =>
      call<Investigation>(FN.cancel, { investigation_id }),
  }
}

export type Client = ReturnType<typeof client>
