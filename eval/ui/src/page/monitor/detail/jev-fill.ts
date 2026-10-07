// "Fill with Jev" in the attach dialog, as pure logic: what the answer of
// `eval::propose-validation` becomes on screen (the fill state, the notice
// copy, the quick-pick pairs) and how its failures read. Code decided which
// pairs were eligible before Jev chose; nothing here re-derives that. No
// React, no host: unit-tested.
import type { ProposeValidationResponse } from '../../../types'
import { plural } from './present'
import type { Side } from './validation-lookup'

/** Whole percent: never `6.999999%`; under one percent reads `<1%`. */
export function pct(probability: number): string {
  const whole = Math.round(Math.min(1, Math.max(0, probability)) * 100)
  return whole === 0 && probability > 0 ? '<1%' : `${whole}%`
}

export interface JevPair {
  baseline: string
  candidate: string
  probability: number
  stackNote: string
}

/** The counts the answer reports about what code offered Jev. */
export interface Tally {
  runsConsidered: number
  pairsConsidered: number
  pairsDropped: number
  excluded: Record<string, number>
}

export interface Proposed {
  kind: 'proposed'
  /** Jev's own pick first, then its alternatives, most likely first. */
  pairs: JevPair[]
  /** The pair on the pickers. */
  chosen: number
  lowConfidence: boolean
  tally: Tally
  /** Which pickers still hold what Jev put there. */
  marks: Record<Side, boolean>
}

export type JevFill =
  | { kind: 'idle' }
  | { kind: 'asking' }
  | Proposed
  /** A valid answer: Jev saw no pair that fits this suggestion. */
  | { kind: 'none_fits'; tally: Tally }
  /** Decided in code: no two runs of one case to pair, so Jev was not asked. */
  | { kind: 'no_comparable_pair'; tally: Tally }
  | { kind: 'failed'; message: string }

export function fillFromResponse(response: ProposeValidationResponse): JevFill {
  const tally: Tally = {
    runsConsidered: response.runs_considered,
    pairsConsidered: response.pairs_considered,
    pairsDropped: response.pairs_dropped,
    excluded: response.excluded ?? {},
  }
  if (response.outcome === 'no_comparable_pair') return { kind: 'no_comparable_pair', tally }
  if (response.outcome === 'none_fits') return { kind: 'none_fits', tally }
  const { proposal } = response
  if (!proposal) return { kind: 'failed', message: 'Jev answered without a pair.' }
  return {
    kind: 'proposed',
    pairs: [
      {
        baseline: proposal.baseline_execution_id,
        candidate: proposal.candidate_execution_id,
        probability: proposal.confidence,
        stackNote: proposal.stack_note,
      },
      ...(response.alternatives ?? []).map((alternative) => ({
        baseline: alternative.baseline_execution_id,
        candidate: alternative.candidate_execution_id,
        probability: alternative.probability,
        stackNote: alternative.stack_note,
      })),
    ],
    chosen: 0,
    lowConfidence: proposal.low_confidence,
    tally,
    marks: { baseline: true, candidate: true },
  }
}

/** Puts one of Jev's pairs on the pickers; both marks are Jev's again. */
export function choosePair(fill: Proposed, index: number): Proposed {
  return { ...fill, chosen: index, marks: { baseline: true, candidate: true } }
}

/**
 * The pickers now hold `picked`. Marks follow the ids, not the clicks: a pair
 * Jev offered (its pick or an alternative) is Jev's again, also when the user
 * typed it in; otherwise a field is Jev's only while it holds what Jev put there.
 */
export function settlePicked(fill: JevFill, picked: Pick<JevPair, 'baseline' | 'candidate'>): JevFill {
  if (fill.kind !== 'proposed') return fill
  const offered = fill.pairs.findIndex(
    (pair) => pair.baseline === picked.baseline && pair.candidate === picked.candidate,
  )
  if (offered >= 0) return choosePair(fill, offered)
  const own = fill.pairs[fill.chosen]
  return {
    ...fill,
    marks: { baseline: picked.baseline === own.baseline, candidate: picked.candidate === own.candidate },
  }
}

const intact = (fill: Proposed): boolean => fill.marks.baseline && fill.marks.candidate

/** Jev's low-confidence pick for a plan that asks for a new case: no recorded pair may test it, so E2E is the next step. */
export const needsNewCase = (fill: JevFill, scenarioId: string | null): boolean =>
  fill.kind === 'proposed' && fill.lowConfidence && scenarioId === null && (fill.marks.baseline || fill.marks.candidate)

// --- failures -------------------------------------------------------------------

export type FillFailure =
  /** The E2E service could not list runs: the dialog's run list is down too. */
  | { kind: 'e2e_unavailable' }
  /** Jev, or the call to it, failed; the message is the provider's, as returned. */
  | { kind: 'jev'; message: string }

// The backend words its errors with a stable code first (`jev_unavailable: Jev
// (typesafe) answered provider_error (HTTP 402): …`), after whatever the bus
// prefixed (`dependency error: `). The code is dropped; the rest is shown as is.
const JEV_CODES = ['jev_unavailable', 'jev_invalid_response', 'jev_invalid_request']

export function classifyFillFailure(message: string): FillFailure {
  if (message.includes('e2e_unavailable')) return { kind: 'e2e_unavailable' }
  const code = JEV_CODES.find((candidate) => message.includes(candidate))
  if (!code) return { kind: 'jev', message }
  const rest = message.slice(message.indexOf(code) + code.length).replace(/^\s*:\s*/, '')
  return { kind: 'jev', message: rest || message }
}

// --- copy -------------------------------------------------------------------------

/** The plan's one line above the pickers: what to pick, from `validation.scenario_id`. `code` spans are backticked. */
export function planHint(scenarioId: string | null): string {
  return scenarioId
    ? `Run \`${scenarioId}\` twice with the same model: the baseline on the current Harness, the candidate with this change.`
    : 'This plan needs a new E2E case. Pick two runs of the same scenarios and model: the baseline without this change, the candidate with it.'
}

const STATUS_WORDS: Record<string, [one: string, many: string]> = {
  technical_failed: ['technical failure', 'technical failures'],
  infra_failed: ['infra failure', 'infra failures'],
  running: ['still running', 'still running'],
  unknown_status: ['with no status', 'with no status'],
  other_scenario: ['on another scenario', 'on other scenarios'],
  no_id: ['without an id', 'without an id'],
}

function statusWords(status: string, count: number): string {
  const [one, many] = STATUS_WORDS[status] ?? [status.replace(/_/g, ' '), status.replace(/_/g, ' ')]
  return `${count} ${count === 1 ? one : many}`
}

const LAST = ['other_scenario', 'no_id']

/** `17 technical failures, 10 incomplete, 2 infra failures, 43 on other scenarios`: statuses by count, then the scenario and id counts. */
export function excludedSummary(excluded: Record<string, number>): string {
  const entries = Object.entries(excluded).filter(([, count]) => count > 0)
  const rank = (status: string) => (LAST.includes(status) ? LAST.indexOf(status) + 1 : 0)
  entries.sort(([a, x], [b, y]) => rank(a) - rank(b) || y - x || a.localeCompare(b))
  return entries.map(([status, count]) => statusWords(status, count)).join(', ')
}

const excludedTotal = (excluded: Record<string, number>): number =>
  Object.values(excluded).reduce((sum, count) => sum + Math.max(0, count), 0)

/** `Chosen among 6 comparable pairs from 4 runs; 3 runs excluded: 2 still running, 1 technical failure.` */
export function offeredLine(verb: 'Chosen among' | 'Looked at', tally: Tally): string {
  const excluded = excludedTotal(tally.excluded)
  const parts = [
    `${verb} ${plural(tally.pairsConsidered, 'comparable pair')} from ${plural(tally.runsConsidered, 'run')}`,
    excluded > 0 ? `${plural(excluded, 'run')} excluded: ${excludedSummary(tally.excluded)}` : '',
    tally.pairsDropped > 0 ? `${plural(tally.pairsDropped, 'older pair')} not offered` : '',
  ]
  return `${parts.filter(Boolean).join('; ')}.`
}

/** `Recorded stack differs:` in text, the stacks themselves in mono; a note without detail is all text. */
export function stackLine(note: string): { head: string; mono?: string } {
  const capital = (text: string) => text.charAt(0).toUpperCase() + text.slice(1)
  const at = note.indexOf(': ')
  return at < 0 ? { head: capital(note) } : { head: capital(note.slice(0, at + 1)), mono: note.slice(at + 2) }
}

export interface ProposedNotice {
  tone: 'neutral' | 'warn'
  headline: string
  detail: string
  /** What a low-confidence answer asks of the user beyond checking: look at the other pairs, or run the new case. */
  advice: string[]
  foot: string
  stack: { head: string; mono?: string }
}

/** One sentence: compare with the other pairs, and run the new case when no recorded pair may test the change. */
function lowConfidenceAdvice(others: boolean, newCase: boolean): string[] {
  if (others && newCase)
    return ['Compare with the other pairs below; if none tests this change, run the new case in E2E first.']
  if (others) return ['Compare with the other pairs below.']
  if (newCase) return ['If no recorded pair tests this change, run the new case in E2E first.']
  return []
}

/**
 * The notice under the pickers while both hold Jev's pair. Warn follows the
 * backend's `low_confidence` of Jev's own pick, also while one of its
 * alternatives is on the pickers.
 */
export function proposedNotice(fill: Proposed, scenarioId: string | null): ProposedNotice {
  const pair = fill.pairs[fill.chosen]
  const own = fill.chosen === 0
  const percent = pct(pair.probability)
  return {
    tone: fill.lowConfidence ? 'warn' : 'neutral',
    headline: own
      ? `${fill.lowConfidence ? "Jev isn't sure" : 'Jev picked this pair'} · ${percent} confidence`
      : `Jev's alternative · ${percent}`,
    detail:
      "Check both runs before attaching. Confidence describes Jev's choice among these pairs, not whether the change works.",
    advice: fill.lowConfidence ? lowConfidenceAdvice(fill.pairs.length > 1, scenarioId === null) : [],
    foot: offeredLine('Chosen among', fill.tally),
    stack: stackLine(pair.stackNote),
  }
}

/** The quiet line that replaces the notice once the user changed a picker Jev filled. */
export function editedNotice(fill: Proposed): { headline: string; detail: string } | undefined {
  if (intact(fill)) return undefined
  const both = !fill.marks.baseline && !fill.marks.candidate
  const changed = both ? 'both runs' : `the ${fill.marks.baseline ? 'candidate' : 'baseline'}`
  return {
    headline: `You changed ${changed} Jev picked`,
    detail: `Jev's ${pct(fill.pairs[fill.chosen].probability)} confidence was for the pair as Jev chose it, so it no longer applies.`,
  }
}

export interface QuickPick {
  index: number
  pair: JevPair
  /** `21% · recorded stacks identical`; Jev's own pick says so: `Jev's pick · 43% · …`. */
  quiet: string
}

/**
 * The pairs the user can switch to: while one of Jev's pairs is on the pickers,
 * the others; once a picker was edited, all of them, Jev's own pick too, so
 * there is a way back even when Jev offered no alternative. Nothing once the
 * user changed both (it is no longer Jev's fill).
 */
export function quickPicks(fill: Proposed): QuickPick[] {
  if (!fill.marks.baseline && !fill.marks.candidate) return []
  return fill.pairs.flatMap((pair, index) =>
    index === fill.chosen && intact(fill)
      ? []
      : [
          {
            index,
            pair,
            quiet: [index === 0 ? `Jev's pick · ${pct(pair.probability)}` : pct(pair.probability), pair.stackNote]
              .filter(Boolean)
              .join(' · '),
          },
        ],
  )
}

export interface PairlessNotice {
  headline: string
  detail: string
  foot: string
}

/** Jev answered and saw no pair that fits (state 6a). */
export function noFitNotice(tally: Tally): PairlessNotice {
  return {
    headline: 'Jev found no pair for this change',
    detail: `None of the ${plural(tally.pairsConsidered, 'comparable pair')} looks like this suggestion's baseline and candidate. Choose the runs yourself, or run the pair in E2E first.`,
    foot: offeredLine('Looked at', tally),
  }
}

/** Code found no two runs to pair (state 6b); the reason names the rule that failed. */
export function noPairNotice(tally: Tally, scenarioId: string | null): PairlessNotice {
  const n = tally.runsConsidered
  const finished = plural(n, 'finished run')
  const detail = scenarioId
    ? n === 0
      ? `No finished run includes ${scenarioId}.`
      : `${finished} ${n === 1 ? 'includes' : 'include'} ${scenarioId}, but no two used the same model and provider.`
    : n === 0
      ? 'No run has finished yet.'
      : `${finished}, but no two ran the same scenarios with the same model.`
  const counted = excludedSummary(tally.excluded)
  return {
    headline: 'No comparable pair yet',
    detail,
    foot: `${counted ? `Not counted: ${counted}. ` : ''}Run the ${scenarioId ? 'scenario' : 'case'} again in E2E with the changed Harness, then come back.`,
  }
}

/** The E2E service listed no executions at all (state 9a). */
export function noRunsNotice(scenarioId: string | null): { headline: string; detail: string } {
  return {
    headline: 'No E2E runs yet',
    detail: `The E2E service has no executions to attach. Run ${scenarioId ?? 'the case'} twice there, baseline first, then come back.`,
  }
}
