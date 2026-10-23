// What changed between two analyses of the same observed turn, computed in code: which suggestions stayed, which
// went away and which are new, and the same three counts for the signals by fingerprint. It never says which
// analysis is right, and it does not read meaning: a suggestion stays when its title says the same or when it cites
// mostly the same evidence, never because two texts sound alike.
import { shortHash } from '../../../model'
import type { AnalysisRecord, Diagnostic, Suggestion } from '../../../types'
import { codeRefLabel } from './present'

/** The same Harness area and the same title once case, punctuation and spacing are set aside. */
export function suggestionKey(suggestion: Pick<Suggestion, 'harness_component' | 'title'>): string {
  const title = suggestion.title
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, ' ')
    .trim()
  return `${suggestion.harness_component}\u0000${title}`
}

/** A suggestion with its place (1-based) in its analysis. */
export interface Placed {
  index: number
  suggestion: Suggestion
}

export interface Kept {
  before: Placed
  now: Placed
  /** Paired by the evidence both cite, not by title: the same idea in other words. */
  reworded?: boolean
}

export interface SuggestionDiff {
  kept: Kept[]
  /** In the earlier analysis only. */
  dropped: Placed[]
  /** In the later analysis only. */
  added: Placed[]
}

/** Evidence entries both cite, out of the smaller of the two lists. */
const REWORDED_OVERLAP = 0.5

const cited = (suggestion: Suggestion) =>
  new Set(suggestion.evidence.map((ref) => `${ref.session_id}\u0000${ref.entry_id}`))

/** Share of the smaller evidence list that the other one cites as well; 0 when either cites nothing. */
export function evidenceOverlap(a: Suggestion, b: Suggestion): number {
  const [left, right] = [cited(a), cited(b)]
  const smaller = Math.min(left.size, right.size)
  if (smaller === 0) return 0
  return [...left].filter((entry) => right.has(entry)).length / smaller
}

/**
 * Matches by `suggestionKey` (two suggestions of one analysis with the same key pair off in order), then pairs what is
 * left by the evidence they cite: each leftover of the later analysis takes the earlier one it shares the most
 * evidence with, if that is at least half of the smaller list.
 */
export function diffSuggestions(before: readonly Suggestion[], now: readonly Suggestion[]): SuggestionDiff {
  const waiting = new Map<string, Placed[]>()
  before.forEach((suggestion, at) => {
    const key = suggestionKey(suggestion)
    waiting.set(key, [...(waiting.get(key) ?? []), { index: at + 1, suggestion }])
  })
  const kept: Kept[] = []
  const added: Placed[] = []
  now.forEach((suggestion, at) => {
    const placed = { index: at + 1, suggestion }
    const match = waiting.get(suggestionKey(suggestion))?.shift()
    if (match) kept.push({ before: match, now: placed })
    else added.push(placed)
  })
  const dropped = [...waiting.values()].flat().sort((a, b) => a.index - b.index)
  for (const placed of [...added]) {
    let best: { placed: Placed; share: number } | undefined
    for (const candidate of dropped) {
      const share = evidenceOverlap(candidate.suggestion, placed.suggestion)
      if (share >= REWORDED_OVERLAP && share > (best?.share ?? 0)) best = { placed: candidate, share }
    }
    if (!best) continue
    dropped.splice(dropped.indexOf(best.placed), 1)
    added.splice(added.indexOf(placed), 1)
    kept.push({ before: best.placed, now: placed, reworded: true })
  }
  kept.sort((a, b) => a.now.index - b.now.index)
  return { kept, dropped, added }
}

/** `Code references 0 → 8 · plan scenario unchanged tool_contract_recovery`. */
export function keptNote({ before, now, reworded }: Kept): string {
  const [was, is] = [before.suggestion, now.suggestion]
  const [from, to] = [was.validation.scenario_id, is.validation.scenario_id]
  const scenario =
    from === to
      ? `plan scenario unchanged ${to ?? 'new case needed'}`
      : `plan scenario ${from ?? 'new case needed'} → ${to ?? 'new case needed'}`
  const references = `Code references ${was.code_refs.length} → ${is.code_refs.length} · ${scenario}`
  return reworded ? `Reworded, was “${was.title}” · ${references}` : references
}

/** What a new suggestion points at: the first code it read, or that it read none. */
export function readsNote(suggestion: Suggestion): string {
  const first = suggestion.code_refs[0]
  return first ? `Reads ${codeRefLabel(first)}` : 'Reads no code'
}

export interface SignalDiff {
  kept: number
  dropped: Diagnostic[]
  added: Diagnostic[]
}

/** The signals of two captures by fingerprint: the same observation keeps the same one. */
export function diffSignals(before: readonly Diagnostic[], now: readonly Diagnostic[]): SignalDiff {
  const had = new Set(before.map((diagnostic) => diagnostic.fingerprint))
  const has = new Set(now.map((diagnostic) => diagnostic.fingerprint))
  return {
    kept: [...has].filter((fingerprint) => had.has(fingerprint)).length,
    dropped: before.filter((diagnostic) => !has.has(diagnostic.fingerprint)),
    added: now.filter((diagnostic) => !had.has(diagnostic.fingerprint)),
  }
}

/** `repeated_tool_error · state::get · fp 2b90…a4` */
export function signalLabel(diagnostic: Diagnostic): string {
  return `${diagnostic.rule_id} · ${diagnostic.target} · fp ${shortHash(diagnostic.fingerprint)}`
}

/** The analyses of the open one's turn, newest first, without the open one. */
export function otherAnalyses(records: readonly AnalysisRecord[], evaluationId: string): AnalysisRecord[] {
  return records.filter((record) => record.evaluation_id !== evaluationId).sort((a, b) => b.created_at - a.created_at)
}

/** Compare needs suggestions and signals to read: only an analysis that ran to the end has them. */
export function canCompare(record: Pick<AnalysisRecord, 'status'>): boolean {
  return record.status === 'completed'
}

/** How the open analysis relates to a listed one: it replaced it, or was replaced by it. */
export function relation(
  open: Pick<AnalysisRecord, 'evaluation_id' | 'supersedes'>,
  other: AnalysisRecord,
): string | undefined {
  if (open.supersedes === other.evaluation_id) return 'Replaced by this analysis'
  if (other.supersedes === open.evaluation_id) return 'Replaces this analysis'
  return undefined
}

/** The open analysis in the story of its turn: what it is a reanalysis of, or what replaced it. */
export interface TurnRelation {
  /** `Reanalysis of` or `Replaced by`. */
  label: string
  evaluationId: string
  /** When that analysis started; unknown while the turn's list is not read, or after it was deleted. */
  at?: number
}

/** `supersedes` points back, so a replaced analysis finds its replacement among the others: the newest one. */
export function turnRelation(
  open: Pick<AnalysisRecord, 'evaluation_id' | 'supersedes'>,
  rows: readonly AnalysisRecord[] | null,
): TurnRelation | undefined {
  if (open.supersedes) {
    const earlier = rows?.find((record) => record.evaluation_id === open.supersedes)
    return { label: 'Reanalysis of', evaluationId: open.supersedes, at: earlier?.created_at }
  }
  const newest = otherAnalyses(rows ?? [], open.evaluation_id).find(
    (record) => record.supersedes === open.evaluation_id,
  )
  return newest ? { label: 'Replaced by', evaluationId: newest.evaluation_id, at: newest.created_at } : undefined
}
