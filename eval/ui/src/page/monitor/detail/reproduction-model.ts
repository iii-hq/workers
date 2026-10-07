// What a replay of the decision point says, computed from its replies: how
// often the signal showed, how the change moved it, what else moved, and which
// step of the flow the suggestion is at. Pure functions, so the card and the
// tests read the same numbers.
import type { Reply, Reproduction, SuggestionCheck } from '../../../types'
import { formatCostShort } from './present'

export const DEFAULT_SAMPLES = 20
export const MORE_SAMPLES = 30
/** Replies one reproduction may hold, extensions included (the backend's limit). */
export const MAX_SAMPLES = 100
/** Below this p-value a difference is called a difference. */
const SIGNIFICANCE = 0.05

export const FINAL_ANSWER = 'final answer'

export interface Tally {
  requested: number
  /** Replies that came back (with or without a signal). */
  ok: number
  failed: number
  positive: number
  negative: number
  unclear: number
}

export function tally(reproduction: Reproduction): Tally {
  const ok = reproduction.samples.filter((reply) => !reply.error)
  return {
    requested: reproduction.requested,
    ok: ok.length,
    failed: reproduction.samples.length - ok.length,
    positive: ok.filter((reply) => reply.signal === true).length,
    negative: ok.filter((reply) => reply.signal === false).length,
    unclear: ok.filter((reply) => reply.signal === undefined).length,
  }
}

export function percent(part: number, whole: number): string {
  if (whole === 0) return '—'
  return `${Math.round((part / whole) * 100)}%`
}

// ---------------------------------------------------------------------------
// Fisher's exact test, two-sided
// ---------------------------------------------------------------------------

const logFactorials: number[] = [0]
function logFactorial(n: number): number {
  for (let i = logFactorials.length; i <= n; i += 1) logFactorials[i] = logFactorials[i - 1] + Math.log(i)
  return logFactorials[n]
}

/**
 * p-value of the 2×2 table [[a, b], [c, d]] (rows: base and change; columns: signal and no signal), summing every
 * table with the same margins that is no more likely than the observed one.
 */
export function fisherTwoSided(a: number, b: number, c: number, d: number): number {
  const row1 = a + b
  const col1 = a + c
  const n = a + b + c + d
  if (n === 0) return 1
  const logP = (x: number) =>
    logFactorial(row1) +
    logFactorial(n - row1) +
    logFactorial(col1) +
    logFactorial(n - col1) -
    logFactorial(n) -
    logFactorial(x) -
    logFactorial(row1 - x) -
    logFactorial(col1 - x) -
    logFactorial(n - row1 - col1 + x)
  const observed = logP(a)
  let p = 0
  for (let x = Math.max(0, col1 - (n - row1)); x <= Math.min(row1, col1); x += 1) {
    const current = logP(x)
    if (current <= observed + 1e-9) p += Math.exp(current)
  }
  return Math.min(1, p)
}

/** `0.12`, `0.005`, `< 0.001`. */
export function formatP(p: number): string {
  if (p < 0.001) return '< 0.001'
  return p < 0.01 ? p.toFixed(3) : p.toFixed(2)
}

// ---------------------------------------------------------------------------
// One replay
// ---------------------------------------------------------------------------

/** The functions a reply called, once each, or `final answer` when it called none. */
export function actionsOf(reply: Reply): string[] {
  if (reply.calls.length === 0) return [FINAL_ANSWER]
  return [...new Set(reply.calls.map((call) => call.target))]
}

/** Replies that repeat a call of the original step exactly (same function, same payload). */
export function sameAsOriginal(reproduction: Reproduction): number {
  const original = reproduction.original
  if (!original || original.calls.length === 0) return 0
  const keys = new Set(original.calls.map((call) => `${call.target}\n${call.payload}`))
  return reproduction.samples.filter(
    (reply) => !reply.error && reply.calls.some((call) => keys.has(`${call.target}\n${call.payload}`)),
  ).length
}

/** `Reproduced in 3 of 20 (15%).` / `Not reproduced in 20 samples.` */
export function reproductionSentence(reproduction: Reproduction): string {
  const t = tally(reproduction)
  const measured = t.positive + t.negative
  if (measured === 0) return 'No reply could be read.'
  const head =
    t.positive === 0
      ? `Not reproduced in ${measured} ${measured === 1 ? 'sample' : 'samples'}.`
      : `Reproduced in ${t.positive} of ${measured} (${percent(t.positive, measured)}).`
  const same = sameAsOriginal(reproduction)
  const original = same > 0 ? ` The original call appeared ${same === 1 ? 'once' : `${same} times`}.` : ''
  const unclear = t.unclear > 0 ? ` ${t.unclear} unclear.` : ''
  return `${head}${original}${unclear}`
}

/** How often each action shows across the replies that came back. */
export function actionCounts(reproduction: Reproduction): Map<string, number> {
  const counts = new Map<string, number>()
  for (const reply of reproduction.samples) {
    if (reply.error) continue
    for (const action of actionsOf(reply)) counts.set(action, (counts.get(action) ?? 0) + 1)
  }
  return counts
}

export function callsPerSample(reproduction: Reproduction): number | null {
  const ok = reproduction.samples.filter((reply) => !reply.error)
  if (ok.length === 0) return null
  return ok.reduce((sum, reply) => sum + reply.calls.length, 0) / ok.length
}

// ---------------------------------------------------------------------------
// Base against a change
// ---------------------------------------------------------------------------

export interface SideEffect {
  action: string
  base: number
  change: number
  baseN: number
  changeN: number
  p: number
}

export interface Comparison {
  sentence: string
  base: { positive: number; measured: number }
  change: { positive: number; measured: number }
  p: number
  /** The difference may be chance. */
  inconclusive: boolean
  /** More replies per side that would settle it if the rates hold; null when no reachable number would. */
  moreNeeded: number | null
  sideEffects: SideEffect[]
  calls: { base: number | null; change: number | null }
}

/** The smallest extra number of replies per side at which the observed rates would differ significantly. */
export function moreSamplesNeeded(
  base: { positive: number; measured: number },
  change: { positive: number; measured: number },
  max = MAX_SAMPLES,
): number | null {
  if (base.measured === 0 || change.measured === 0) return null
  const rateBase = base.positive / base.measured
  const rateChange = change.positive / change.measured
  if (rateBase === rateChange) return null
  const start = Math.max(base.measured, change.measured)
  for (let n = start + 1; n <= max; n += 1) {
    const a = Math.round(rateBase * n)
    const c = Math.round(rateChange * n)
    if (fisherTwoSided(a, n - a, c, n - c) < SIGNIFICANCE) return n - start
  }
  return null
}

export function compare(base: Reproduction, change: Reproduction): Comparison {
  const b = tally(base)
  const c = tally(change)
  const baseCounts = { positive: b.positive, measured: b.positive + b.negative }
  const changeCounts = { positive: c.positive, measured: c.positive + c.negative }
  const p = fisherTwoSided(
    baseCounts.positive,
    baseCounts.measured - baseCounts.positive,
    changeCounts.positive,
    changeCounts.measured - changeCounts.positive,
  )
  const inconclusive = p >= SIGNIFICANCE
  const from = percent(baseCounts.positive, baseCounts.measured)
  const to = percent(changeCounts.positive, changeCounts.measured)
  const moved =
    changeCounts.positive / Math.max(1, changeCounts.measured) <
    baseCounts.positive / Math.max(1, baseCounts.measured)
      ? 'dropped'
      : changeCounts.positive / Math.max(1, changeCounts.measured) >
          baseCounts.positive / Math.max(1, baseCounts.measured)
        ? 'rose'
        : 'stayed'
  const moreNeeded = inconclusive ? moreSamplesNeeded(baseCounts, changeCounts) : null
  let sentence =
    moved === 'stayed' ? `The signal stayed at ${from}.` : `The signal ${moved} from ${from} to ${to} (p = ${formatP(p)}).`
  if (inconclusive && moved !== 'stayed') {
    sentence += moreNeeded
      ? ` This may still be chance; ${moreNeeded} more ${moreNeeded === 1 ? 'sample' : 'samples'} per side would settle it.`
      : ' This may still be chance.'
  }

  const baseActions = actionCounts(base)
  const changeActions = actionCounts(change)
  const sideEffects: SideEffect[] = []
  for (const action of new Set([...baseActions.keys(), ...changeActions.keys()])) {
    const x = baseActions.get(action) ?? 0
    const y = changeActions.get(action) ?? 0
    const effectP = fisherTwoSided(x, b.ok - x, y, c.ok - y)
    if (effectP < SIGNIFICANCE) sideEffects.push({ action, base: x, change: y, baseN: b.ok, changeN: c.ok, p: effectP })
  }
  sideEffects.sort((one, two) => one.p - two.p)
  return {
    sentence,
    base: baseCounts,
    change: changeCounts,
    p,
    inconclusive,
    moreNeeded,
    sideEffects,
    calls: { base: callsPerSample(base), change: callsPerSample(change) },
  }
}

// ---------------------------------------------------------------------------
// Where the suggestion stands
// ---------------------------------------------------------------------------

export type Stage =
  /** No check: the person names the decision point and the signal. */
  | 'setup'
  | 'ready'
  | 'reproducing'
  | 'reproduced'
  | 'not_reproduced'
  | 'testing'
  | 'compared'
  | 'failed'

export interface Standing {
  stage: Stage
  base?: Reproduction
  change?: Reproduction
  /** The reproduction that is running or failed last. */
  current?: Reproduction
}

/** The latest base replay, the latest change tested after it, and the step that follows from them. */
export function standing(check: SuggestionCheck | undefined, reproductions: Reproduction[] = []): Standing {
  const latest = reproductions.at(-1)
  const bases = reproductions.filter((reproduction) => reproduction.change_kind === 'none')
  const base = bases.at(-1)
  const change = reproductions.filter((reproduction) => reproduction.change_kind !== 'none').at(-1)
  if (latest?.state === 'running') {
    return { stage: latest.change_kind === 'none' ? 'reproducing' : 'testing', base, change, current: latest }
  }
  if (latest?.state === 'failed') return { stage: 'failed', base, change, current: latest }
  if (!base) return { stage: check ? 'ready' : 'setup', change }
  if (change && change.state === 'completed') return { stage: 'compared', base, change }
  return { stage: tally(base).positive > 0 ? 'reproduced' : 'not_reproduced', base, change }
}

/** `step 4` from `e_t_<turn>_4_assistant`. */
export function stepLabel(entryId: string): string {
  const match = /_(\d+)_assistant$/.exec(entryId)
  return match ? `step ${match[1]}` : entryId
}

/** What the replay of `samples` replies may cost, from what the original step cost. */
export function estimateCost(stepCostUsd: number | undefined, samples: number): number | undefined {
  if (stepCostUsd === undefined || !Number.isFinite(stepCostUsd)) return undefined
  return stepCostUsd * samples
}

/** `$0.10 + 10 not reported`: what a reproduction cost, with the samples that reported no cost named, never as zero. */
export function reproductionCost(reproduction: Reproduction): string {
  const { cost_usd: usd, cost_unknown_samples: unknown } = reproduction
  if (unknown === 0) return formatCostShort(usd)
  return [usd === undefined ? undefined : formatCostShort(usd), `${unknown} not reported`].filter(Boolean).join(' + ')
}

export function signalLabel(check: SuggestionCheck): string {
  switch (check.signal.rule) {
    case 'contract_rediscovery':
      return 'The reply asks again for a contract already in its context.'
    case 'repeated_error_call':
      return 'The reply repeats the last call that failed, with the same payload.'
    default:
      return check.signal.question ?? ''
  }
}

/** One line for the brief and the decision note. */
export function evidenceLine(standing: Standing): string | undefined {
  if (standing.base && standing.change && standing.stage === 'compared') {
    return `Replay at ${stepLabel(standing.base.check.decision_point)}: ${compare(standing.base, standing.change).sentence}`
  }
  if (standing.base && (standing.stage === 'reproduced' || standing.stage === 'not_reproduced')) {
    return `Replay at ${stepLabel(standing.base.check.decision_point)}: ${reproductionSentence(standing.base)}`
  }
  return undefined
}
