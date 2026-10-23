// Pure presentation helpers of the analysis detail: times, ids, the routing
// sentence, triage bars, entry lookups. No React, no host.
import { formatCost } from '../../../model'
import type {
  AnalysisRecord,
  ChoiceAnswer,
  CodeRef,
  Diagnostic,
  EntryRef,
  ExcludedProbe,
  MonitorLimits,
  Snapshot,
  Triage,
  ValidationLink,
} from '../../../types'

/** Suggestions the analyst may return: a contract constant, not a configured limit. */
export const MAX_SUGGESTIONS = 3

/** Code references one suggestion may cite: a contract constant, like `MAX_SUGGESTIONS`. */
export const MAX_CODE_REFS = 8

export const TRIAGE_QUESTION = 'investigation'
/** The one triage answer that sends a session to the analyst. */
const INVESTIGATED_CHOICE = 'needs_investigation'
const CHOICE_ORDER = [INVESTIGATED_CHOICE, 'insufficient_evidence', 'expected_behavior']

/** `1,204` */
export function count(value: number): string {
  return new Intl.NumberFormat('en-US').format(value)
}

export function plural(count: number, one: string, many = `${one}s`): string {
  return `${count} ${count === 1 ? one : many}`
}

/** `0.2 s`, `47.3 s`; whole seconds with `digits` 0 (`180 s`). */
export function seconds(ms: number, digits = 1): string {
  return `${(Math.max(0, ms) / 1000).toFixed(digits)} s`
}

/** `HH:MM:SS` in the viewer's time zone. */
export function clock(ms: number): string {
  const date = new Date(ms)
  return [date.getHours(), date.getMinutes(), date.getSeconds()].map((part) => String(part).padStart(2, '0')).join(':')
}

/** `t_1a2b3c4d…` for the 32-hex ids Harness mints; short ids stay whole. */
export function shortId(id: string, keep = 10): string {
  return id.length > keep + 2 ? `${id.slice(0, keep)}…` : id
}

/** `root + 2 descendants`; unknown until the capture exists. */
export function treeLabel(snapshot: Snapshot | undefined): string | undefined {
  if (!snapshot) return undefined
  const below = Math.max(0, snapshot.coverage.sessions_in_scope - 1)
  return below === 0 ? 'root only' : `root + ${plural(below, 'descendant')}`
}

/** `turn <short> · root + N descendants · captured HH:MM:SS`: the meta line after the session id. */
export function metaRest(record: AnalysisRecord, snapshot: Snapshot | undefined): string {
  return [
    `turn ${shortId(record.turn_id)}`,
    treeLabel(snapshot),
    snapshot ? `captured ${clock(snapshot.captured_at)}` : undefined,
  ]
    .filter(Boolean)
    .join(' · ')
}

export function capitalize(text: string): string {
  return text ? text[0].toUpperCase() + text.slice(1) : text
}

/** `Completed · end_turn`; the turn status the monitor observed. */
export function sourceLine(snapshot: Snapshot): string {
  return [capitalize(snapshot.source_status.replaceAll('_', ' ')), snapshot.source_stop_reason]
    .filter(Boolean)
    .join(' · ')
}

export function triageChoice(triage: Triage | undefined): ChoiceAnswer | undefined {
  const answer = triage?.answers[TRIAGE_QUESTION]
  return answer?.type === 'choice' ? answer : undefined
}

export interface ChoiceBar {
  key: string
  probability: number
  chosen: boolean
}

/** The three answers in a stable order, the chosen one flagged. */
export function choiceBars(answer: ChoiceAnswer): ChoiceBar[] {
  const keys = Object.keys(answer.probabilities)
  const ordered = [
    ...CHOICE_ORDER.filter((key) => keys.includes(key)),
    ...keys.filter((key) => !CHOICE_ORDER.includes(key)),
  ]
  return ordered.map((key) => ({
    key,
    probability: Math.min(1, Math.max(0, answer.probabilities[key] ?? 0)),
    chosen: key === answer.choice,
  }))
}

export function twoDecimals(value: number): string {
  return value.toFixed(2)
}

/**
 * `$0.91`: a total or an average to the cent, as the design shows it. A cost
 * under a cent keeps its precision (it is not zero) and an unknown one stays
 * `not reported`. A single analysis' Monitor cost keeps `formatCost`.
 */
export function formatCostShort(usd: number | undefined | null): string {
  if (usd === undefined || usd === null || !Number.isFinite(usd) || (usd > 0 && usd < 0.01)) return formatCost(usd)
  return `$${usd.toFixed(2)}`
}

function joinClauses(clauses: string[]): string {
  if (clauses.length <= 1) return clauses[0] ?? ''
  return `${clauses.slice(0, -1).join(', ')}, and ${clauses[clauses.length - 1]}`
}

/**
 * Why the session was investigated. Only `needs_investigation` is produced now; the other reasons read the analyses
 * recorded before a session was investigated on Jev's answer alone, in the past tense of what they were.
 */
export function routingClauses(record: AnalysisRecord, triage: Triage | undefined): string[] {
  const answer = triageChoice(triage)
  const clauses: string[] = []
  for (const reason of record.routing?.reasons ?? []) {
    switch (reason) {
      case 'needs_investigation':
      case 'insufficient_evidence':
        clauses.push(`Jev answered ${reason}`)
        break
      case 'diagnostics':
        clauses.push(record.counters.diagnostics === 1 ? 'a rule found a signal' : 'rules found signals')
        break
      case 'low_confidence':
        clauses.push(
          answer ? `confidence ${twoDecimals(answer.confidence)} was below its threshold` : 'confidence was low',
        )
        break
      case 'coverage_insufficient':
        clauses.push('the capture was insufficient')
        break
      case 'audit_sample':
        clauses.push('the session was drawn for the audit sample')
        break
      case 'manual_request':
        clauses.push('it was requested manually')
        break
    }
  }
  return clauses
}

export function routingSentence(record: AnalysisRecord, triage: Triage | undefined): string | undefined {
  if (!record.routing) return undefined
  if (record.routing.investigate) {
    const clauses = routingClauses(record, triage)
    return clauses.length ? `Investigated: ${joinClauses(clauses)}.` : 'Investigated.'
  }
  const answer = triageChoice(triage)
  return `Not investigated: ${answer ? `Jev answered ${answer.choice}; ` : ''}only ${INVESTIGATED_CHOICE} is investigated.`
}

/** `c2 · functions::info` tail of an entry id, the way chips label it. */
export function entryTail(entryId: string): string {
  const tail = entryId.replace(/^e_t_[0-9a-z]+_/i, '')
  return tail.length > 14 ? `${tail.slice(0, 12)}…` : tail
}

export function entryKey(ref: EntryRef): string {
  return `${ref.session_id}\u0000${ref.entry_id}`
}

export type EntryHome = 'preview' | 'signal'

/**
 * Where a cited entry can be looked at: in a session preview (what the models
 * saw), or only as evidence of a signal. `undefined` when neither holds it.
 */
export function locateEntry(snapshot: Snapshot | undefined, ref: EntryRef): EntryHome | undefined {
  if (!snapshot) return undefined
  const session = snapshot.sessions.find((candidate) => candidate.session_id === ref.session_id)
  const inPreview = session?.preview.some(
    (item) => typeof item === 'object' && item !== null && !Array.isArray(item) && item.entry_id === ref.entry_id,
  )
  if (inPreview) return 'preview'
  const inSignal = snapshot.diagnostics.some((diagnostic) =>
    diagnostic.evidence.some((e) => entryKey(e) === entryKey(ref)),
  )
  return inSignal ? 'signal' : undefined
}

/** The signal that holds an entry as evidence, for the jump from a chip. */
export function signalHolding(snapshot: Snapshot | undefined, ref: EntryRef): Diagnostic | undefined {
  return snapshot?.diagnostics.find((diagnostic) => diagnostic.evidence.some((e) => entryKey(e) === entryKey(ref)))
}

/** `fp 7c1d…e2 · <session> · fc_02 fc_05` */
export function signalFooter(diagnostic: Diagnostic, shortFingerprint: string): string {
  const calls = diagnostic.evidence.map((ref) => entryTail(ref.entry_id))
  return [`fp ${shortFingerprint}`, diagnostic.session_id, calls.join(' ')].filter(Boolean).join(' · ')
}

const PROBE_NOTE: Record<string, string> = {
  'engine::triggers::info': 'in the default namespace',
}

/** `engine::triggers::info → NOT_FOUND` (mono) and where it was expected. */
export function probeParts(probe: ExcludedProbe): { call: string; note?: string } {
  return { call: `${probe.target} → ${probe.code}`, note: PROBE_NOTE[probe.target] }
}

export function totalEntries(snapshot: Snapshot): number {
  return snapshot.sessions.filter((session) => session.in_scope).reduce((sum, session) => sum + session.entries, 0)
}

export function reducedEntries(snapshot: Snapshot): number {
  return snapshot.sessions.reduce((sum, session) => sum + session.reduced_entries, 0)
}

/** The newest E2E link attached to a suggestion (0-based index). */
export function latestValidation(links: ValidationLink[], suggestionIndex: number): ValidationLink | undefined {
  return links
    .filter((link) => link.suggestion_index === suggestionIndex)
    .sort((a, b) => b.attached_at - a.attached_at)[0]
}

/** Bytes out of the backend's `coverage_insufficient` message, when it carries them. */
export function parseSizes(message: string): { size: number; limit: number } | undefined {
  const match = /(\d+) bytes[\s\S]*?(\d+)-byte limit/.exec(message)
  return match ? { size: Number(match[1]), limit: Number(match[2]) } : undefined
}

/** What one investigation may spend: the code-access caps only when it has a directory. */
export function investigationCaps(limits: MonitorLimits, codeAccess: boolean): { steps: number; totalTokens: number } {
  return codeAccess
    ? { steps: limits.investigation_code_max_turns, totalTokens: limits.investigation_code_max_total_tokens }
    : { steps: limits.investigation_max_turns, totalTokens: limits.investigation_max_total_tokens }
}

function codeRefText(ref: CodeRef, dash: string): string {
  return ref.line_from === ref.line_to
    ? `${ref.path}:${ref.line_from}`
    : `${ref.path}:${ref.line_from}${dash}${ref.line_to}`
}

/** `harness/src/a.rs:120–148` (en dash) or `harness/src/a.rs:88` when both ends are one line. */
export function codeRefLabel(ref: CodeRef): string {
  return codeRefText(ref, '\u2013')
}

/** What a chip copies: the same with an ASCII hyphen, so it pastes into a terminal or an editor. */
export function codeRefCopy(ref: CodeRef): string {
  return codeRefText(ref, '-')
}

/** `Copy harness/src/a.rs lines 120 to 148` for assistive technology. */
export function codeRefAria(ref: CodeRef): string {
  return ref.line_from === ref.line_to
    ? `Copy ${ref.path} line ${ref.line_from}`
    : `Copy ${ref.path} lines ${ref.line_from} to ${ref.line_to}`
}

/** `['/home/', 'layon/', 'workspaces/', 'workers']`: where a long path may wrap, never inside a name. */
export function pathSegments(path: string): string[] {
  // The lookbehind keeps a leading slash with the first name instead of leaving it alone on a line.
  return path.split(/(?<=.\/)/)
}
