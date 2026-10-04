// Pure logic of the suggestion review: how a lifecycle reads, the forms that
// move it, the criterion a validation is judged by, and the rules that refuse
// "validated improvement". Mirrors what `eval::review` enforces, so a refusal
// shows its reason before the call instead of after it. No React.
import { formatStamp, type Tone } from '../../../model'
import type {
  Criterion,
  CriterionInput,
  CriterionMetric,
  Direction,
  EvidenceSide,
  Lifecycle,
  LifecycleStatus,
  RecurrencePattern,
  SuggestionReview,
  ValidationLink,
  ValidationOutcome,
  ValidationPlan,
  ValidationRun,
} from '../../../types'
import { shortId } from './present'
import { checkLabel, DEFAULT_MIN_RUNS, formatClock, mismatches } from './validation-view'

// --- lifecycle ---------------------------------------------------------------

export interface StatusChoice {
  status: LifecycleStatus
  label: string
  hint: string
  /** The change needs a fact before it saves: a dialog asks for it. */
  asks: boolean
}

export const STATUS_CHOICES: StatusChoice[] = [
  { status: 'new', label: 'New', hint: 'Not looked at yet', asks: false },
  { status: 'accepted', label: 'Accepted', hint: 'Worth doing; nobody has started', asks: false },
  { status: 'in_progress', label: 'In progress', hint: 'Someone is implementing it', asks: false },
  { status: 'shipped', label: 'Shipped…', hint: 'Merged. Asks for the PR and the Harness version', asks: true },
  { status: 'rejected', label: 'Rejected…', hint: 'Not doing it. Asks for a reason', asks: true },
  { status: 'duplicate', label: 'Duplicate of…', hint: 'Already covered. Asks for the PR or suggestion', asks: true },
]

/** Why a move is not offered: `eval::review` refuses it. */
export function moveRefusal(current: LifecycleStatus, target: LifecycleStatus): string | undefined {
  if (target === current && current !== 'shipped') return undefined
  if (target === 'new') return "A suggestion can't go back to New"
  if (current === 'shipped' && target !== 'shipped') return 'A shipped suggestion stays shipped'
  return undefined
}

/** `#1292` for a number or a pull request URL; a `owner/repo#1292` stays as typed. */
export function prLabel(pr: string): string {
  const url = /\/pull\/(\d+)/.exec(pr)
  if (url) return `#${url[1]}`
  return /^#?\d+$/.test(pr) ? `#${pr.replace('#', '')}` : pr
}

/** `eval_9f839f40…`/`S2` from `<evaluation_id>:<index>`, else the pull request. */
export function duplicateLabel(duplicateOf: string): string {
  const own = /^(eval_[0-9a-z]+):(\d+)$/.exec(duplicateOf)
  return own ? `${shortId(own[1])} · S${Number(own[2]) + 1}` : `PR ${prLabel(duplicateOf)}`
}

export function lifecycleBadge(lifecycle: Lifecycle): { label: string; tone: Tone; strong?: boolean } {
  switch (lifecycle.status) {
    case 'new':
      return { label: 'New', tone: 'neutral' }
    case 'accepted':
      return { label: 'Accepted', tone: 'neutral', strong: true }
    case 'in_progress':
      return { label: 'In progress', tone: 'warn' }
    case 'shipped':
      return {
        label: [
          lifecycle.pr ? `Shipped in PR ${prLabel(lifecycle.pr)}` : 'Shipped',
          lifecycle.version ? `v${lifecycle.version}` : undefined,
        ]
          .filter(Boolean)
          .join(' · '),
        tone: 'ok',
      }
    case 'rejected':
      return { label: lifecycle.reason ? `Rejected · ${lifecycle.reason.toLowerCase()}` : 'Rejected', tone: 'neutral' }
    case 'duplicate':
      return {
        label: lifecycle.duplicate_of ? `Duplicate of ${duplicateLabel(lifecycle.duplicate_of)}` : 'Duplicate',
        tone: 'neutral',
      }
  }
}

/** A pull request as the person typed it: `#1292`, `owner/repo#1292` or a pull request URL. */
export function parsePr(input: string): string | undefined {
  const text = input.trim()
  if (/^#?\d+$/.test(text)) return `#${text.replace('#', '')}`
  if (/^[\w.-]+\/[\w.-]+#\d+$/.test(text) || /^https?:\/\/\S+\/pull\/\d+\S*$/.test(text)) return text
  return undefined
}

/**
 * What a suggestion duplicates: a pull request, or another suggestion
 * (`S2` of this analysis, `eval_… S2` of another), stored as
 * `<evaluation_id>:<index>`. A suggestion is not a duplicate of itself.
 */
export function parseDuplicateOf(input: string, own: { evaluationId: string; index: number }): string | undefined {
  const text = input.trim()
  const pr = parsePr(text)
  if (pr) return pr
  const match = /^(?:(eval_[0-9a-z]+)\s*[·:\s]\s*)?S(\d+)$/i.exec(text)
  if (!match || Number(match[2]) < 1) return undefined
  const evaluationId = match[1] ?? own.evaluationId
  const index = Number(match[2]) - 1
  return evaluationId === own.evaluationId && index === own.index ? undefined : `${evaluationId}:${index}`
}

/** A semantic version like `1.8.44`; a leading `v` is dropped. */
export function parseVersion(input: string): string | undefined {
  const text = input.trim().replace(/^v/i, '')
  return /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$/.test(text) ? text : undefined
}

export const REJECT_REASONS = [
  'Not reproducible',
  'Wrong cause',
  'Already fixed',
  'Not worth the cost',
  'Other',
] as const

export const OTHER_REASON = 'Other'

export function historyRows(lifecycle: Lifecycle): Array<{ label: string; by: string; at: string; note?: string }> {
  const words = new Map(STATUS_CHOICES.map((choice) => [choice.status, choice.label.replace('…', '')]))
  return [...lifecycle.history].reverse().map((event) => ({
    label: words.get(event.status) ?? event.status,
    by: event.by,
    at: formatStamp(event.at),
    note: event.note,
  }))
}

/** `Accepted by layon, 02 Oct 21:14`: the latest change of status, for the brief. */
export function statusSentence(lifecycle: Lifecycle): string | undefined {
  const last = lifecycle.history[lifecycle.history.length - 1]
  if (!last) return undefined
  const label = lifecycleBadge(lifecycle).label
  return `${label} by ${last.by}, ${formatStamp(last.at)}`
}

// --- criterion ---------------------------------------------------------------

/** The fewest runs per side the form accepts; `eval::review` itself allows 1. */
export const MIN_RUNS_FLOOR = 3
export const MAX_RUNS = 20

const MEASURE: Record<Exclude<CriterionMetric, 'signal_per_run'>, { label: string; description: string }> = {
  function_calls: { label: 'Function calls', description: 'Mean per completed run' },
  tokens: { label: 'Tokens', description: 'Mean per completed run' },
  cost_usd: { label: 'Cost', description: 'Mean USD per completed run' },
  duration: { label: 'Wall time', description: 'Mean per completed run' },
  pass_rate: { label: 'Pass rate', description: 'Share of runs that passed' },
}

/** `{rule, target}` of a `<rule_id>:<target>` pattern; the target may contain colons. */
export function patternParts(pattern: string): { rule: string; target?: string } {
  const at = pattern.indexOf(':')
  return at < 0 ? { rule: pattern } : { rule: pattern.slice(0, at), target: pattern.slice(at + 1) }
}

/** What the criterion measures, in words: `repeated_contract_discovery per run`, `Pass rate`. */
export function metricName(criterion: Pick<CriterionInput, 'metric' | 'pattern'>): string {
  if (criterion.metric === 'signal_per_run') {
    return `${criterion.pattern ? patternParts(criterion.pattern).rule : 'signal'} per run`
  }
  return MEASURE[criterion.metric].label
}

/** The threshold as it was typed: never rounded, since the backend compares against the exact number. */
const amount = (value: number, min: number) =>
  value.toLocaleString('en-US', { minimumFractionDigits: min, maximumFractionDigits: 20 })

interface EffectUnit {
  /** What the minimum effect is counted in: the metric's own unit, never a share of the baseline. */
  unit: string
  /** The value the form offers first. */
  start: string
  /** The effect as a sentence: `1.0 fewer signals per run`. */
  phrase: (value: number, lower: boolean) => string
}

const EFFECT: Record<CriterionMetric, EffectUnit> = {
  signal_per_run: {
    unit: 'signals per run',
    start: '1',
    phrase: (value, lower) => `${amount(value, 1)} ${lower ? 'fewer' : 'more'} signals per run`,
  },
  pass_rate: {
    unit: 'percentage points',
    start: '10',
    phrase: (value, lower) => `${amount(value, 0)} pp ${lower ? 'lower' : 'higher'} pass rate`,
  },
  cost_usd: {
    unit: 'USD',
    start: '0.01',
    phrase: (value, lower) => `$${amount(value, 2)} ${lower ? 'cheaper' : 'more expensive'} per run`,
  },
  duration: {
    unit: 'seconds',
    start: '5',
    phrase: (value, lower) => `${amount(value, 0)} s ${lower ? 'faster' : 'slower'} per run`,
  },
  tokens: {
    unit: 'tokens',
    start: '1000',
    phrase: (value, lower) => `${amount(value, 0)} ${lower ? 'fewer' : 'more'} tokens per run`,
  },
  function_calls: {
    unit: 'function calls',
    start: '1',
    phrase: (value, lower) => `${amount(value, 0)} ${lower ? 'fewer' : 'more'} function calls per run`,
  },
}

/** What the minimum effect of the chosen metric is counted in: `signals per run`, `percentage points`, `USD`. */
export function effectUnit(choice: string): string {
  return EFFECT[choiceParts(choice).metric].unit
}

/** `1.0 fewer signals per run`, `5 pp higher pass rate`, `$0.01 cheaper per run`: what "at least" is said of. */
export function effectPhrase(criterion: Pick<CriterionInput, 'metric' | 'direction' | 'min_effect'>): string {
  return EFFECT[criterion.metric].phrase(criterion.min_effect, criterion.direction === 'decrease')
}

/** `repeated_contract_discovery per run · lower is better · at least 1.0 fewer signals per run · 5 runs per side` */
export function criterionSummary(criterion: CriterionInput): string {
  const lower = criterion.direction === 'decrease'
  return [
    metricName(criterion),
    `${lower ? 'lower' : 'higher'} is better`,
    `at least ${effectPhrase(criterion)}`,
    `${criterion.min_runs} runs per side`,
  ].join(' · ')
}

/** The registered criterion with its author and time, for the brief and the dialogs. */
export function criterionSentence(criterion: Criterion | undefined): string | undefined {
  return criterion
    ? `${criterionSummary(criterion)} (registered ${formatClock(criterion.registered_at)} by ${criterion.registered_by})`
    : undefined
}

export interface MetricChoice {
  value: string
  label: string
  description: string
}

/** One choice per pattern the suggestion cites, then the E2E measures. */
export function metricChoices(patterns: string[]): MetricChoice[] {
  return [
    ...patterns.map((pattern) => ({
      value: `signal_per_run:${pattern}`,
      label: pattern.replace(':', ' · '),
      description: "Signals per run, counted by the monitor in each run's transcript",
    })),
    ...(Object.keys(MEASURE) as Array<keyof typeof MEASURE>).map((metric) => ({
      value: metric,
      label: MEASURE[metric].label,
      description: MEASURE[metric].description,
    })),
  ]
}

export function choiceParts(value: string): Pick<CriterionInput, 'metric' | 'pattern'> {
  if (value.startsWith('signal_per_run:')) {
    return { metric: 'signal_per_run', pattern: value.slice('signal_per_run:'.length) }
  }
  return { metric: value as CriterionMetric }
}

export interface CriterionDraft {
  choice: string
  direction: Direction
  /** The smallest difference of the means, as typed, in the unit of the chosen metric (`effectUnit`). */
  effect: string
  runs: string
  scenario: string
}

/** The signal the suggestion cites first, else function calls; a drop of one, five runs. */
export function defaultDraft(patterns: string[], scenario: string | undefined): CriterionDraft {
  const choice = patterns[0] ? `signal_per_run:${patterns[0]}` : 'function_calls'
  return {
    choice,
    direction: 'decrease',
    effect: EFFECT[choiceParts(choice).metric].start,
    runs: String(DEFAULT_MIN_RUNS),
    scenario: scenario ?? '',
  }
}

/**
 * The draft with another metric. An effect still at the old metric's starting value moves to the new one's: `1` signal
 * must not turn into `1` USD without being asked.
 */
export function withChoice(draft: CriterionDraft, choice: string): CriterionDraft {
  const untouched = draft.effect.trim() === EFFECT[choiceParts(draft.choice).metric].start
  return { ...draft, choice, effect: untouched ? EFFECT[choiceParts(choice).metric].start : draft.effect }
}

/** A draft that shows an existing criterion, to edit. */
export function draftOf(criterion: Criterion): CriterionDraft {
  return {
    choice: criterion.metric === 'signal_per_run' ? `signal_per_run:${criterion.pattern}` : criterion.metric,
    direction: criterion.direction,
    effect: String(criterion.min_effect),
    runs: String(criterion.min_runs),
    scenario: criterion.scenario_id,
  }
}

export type CriterionParse =
  | { ok: true; input: CriterionInput; scenarioId: string }
  | { ok: false; errors: { effect?: string; runs?: string; scenario?: string } }

/** The form's text as a criterion `eval::review` accepts, or what to say under each field. */
export function parseCriterion(draft: CriterionDraft): CriterionParse {
  const errors: { effect?: string; runs?: string; scenario?: string } = {}
  const { metric } = choiceParts(draft.choice)
  const effect = Number(draft.effect.trim().replace(',', '.'))
  if (draft.effect.trim() === '' || !Number.isFinite(effect) || effect <= 0) {
    errors.effect = `Enter a number above 0, in ${EFFECT[metric].unit}.`
  } else if (metric === 'pass_rate' && effect > 100) {
    errors.effect = "A pass rate can't move by more than 100 percentage points."
  }
  const runs = Number(draft.runs.trim())
  if (!Number.isInteger(runs) || runs < MIN_RUNS_FLOOR) {
    errors.runs = `At least ${MIN_RUNS_FLOOR} runs per side. The E2E advises ${DEFAULT_MIN_RUNS}.`
  } else if (runs > MAX_RUNS) {
    errors.runs = `At most ${MAX_RUNS} runs per side.`
  }
  if (draft.scenario.trim() === '') errors.scenario = 'Choose the scenario whose runs are measured.'
  if (Object.keys(errors).length > 0) return { ok: false, errors }
  return {
    ok: true,
    scenarioId: draft.scenario.trim(),
    input: {
      ...choiceParts(draft.choice),
      direction: draft.direction,
      min_effect: effect,
      min_runs: runs,
    },
  }
}

// --- when the criterion was registered ----------------------------------------

/** The links attached to this suggestion. */
export function linksOf(review: SuggestionReview, links: ValidationLink[]): ValidationLink[] {
  return links.filter((link) => link.suggestion_index === review.suggestion_index)
}

/**
 * When results first existed: results cannot predate the start of an execution, so each attached one counts from its
 * own start (an old pair attached today was not unseen until today), besides the attach, the executions this worker
 * requested and the ones a restart replaced.
 */
export function firstDataAt(review: SuggestionReview, links: ValidationLink[]): number | undefined {
  const times = linksOf(review, links).flatMap((link) => [
    link.attached_at,
    link.baseline.started_at,
    link.candidate.started_at,
  ])
  const run = review.run
  if (run && (run.baseline_execution_id || run.candidate_execution_id)) times.push(run.started_at)
  times.push(review.first_run_at)
  const known = times.filter((at): at is number => at !== undefined)
  return known.length > 0 ? Math.min(...known) : undefined
}

/** A criterion cannot change once results exist; the same one can be sent again. */
export function criterionFrozen(review: SuggestionReview, links: ValidationLink[]): boolean {
  return review.criterion !== undefined && firstDataAt(review, links) !== undefined
}

/** `late`: the criterion was registered after the first result, so it cannot prove an improvement. */
export function registeredLate(review: SuggestionReview, links: ValidationLink[]): number | undefined {
  const first = firstDataAt(review, links)
  return review.criterion && first !== undefined && review.criterion.registered_at > first ? first : undefined
}

// --- the verdict ---------------------------------------------------------------

export const OUTCOMES: ValidationOutcome[] = ['validated_improvement', 'no_improvement', 'regression', 'inconclusive']

export const OUTCOME_LABEL: Record<ValidationOutcome, string> = {
  validated_improvement: 'Validated improvement',
  no_improvement: 'No improvement',
  regression: 'Regression',
  inconclusive: 'Inconclusive',
}

export const OUTCOME_HINT: Record<ValidationOutcome, string> = {
  validated_improvement: 'The criterion was met and the controls hold.',
  no_improvement: "The runs are sound; the criterion wasn't met.",
  regression: 'Something got worse in the candidate.',
  inconclusive: "The runs can't tell. Say what is missing.",
}

/** What the code proposes reads shorter than what a person records. */
export const PROPOSAL_LABEL: Record<ValidationOutcome, string> = {
  ...OUTCOME_LABEL,
  validated_improvement: 'Improvement',
}

export const OUTCOME_TONE: Record<ValidationOutcome, Tone> = {
  validated_improvement: 'ok',
  no_improvement: 'neutral',
  regression: 'alert',
  inconclusive: 'warn',
}

const FIXED_CONTROLS = [
  'The candidate exercised the changed path',
  'Baseline and candidate differ only in the Harness build',
  'No infrastructure failures in the runs I counted',
]

/** The plan's invariants and controls, then the three every validation needs. */
export function verdictControls(plan: ValidationPlan): string[] {
  return [...new Set([...plan.invariants, ...plan.non_regression_controls, ...FIXED_CONTROLS])]
}

function shortOfRuns(side: 'baseline' | 'candidate', evidence: EvidenceSide, minimum: number): string {
  const lost = evidence.runs.findIndex((run) => !run.completed)
  if (lost >= 0) {
    const run = evidence.runs[lost]
    const what =
      run.technical && run.technical !== 'valid'
        ? `had an infrastructure failure (${run.technical.replaceAll('_', ' ')})`
        : `was ${(run.completion ?? 'not completed').replaceAll('_', ' ')}`
    return `Not available: the ${side}'s run ${lost + 1} ${what}. ${evidence.n} of ${evidence.runs.length} runs finished.`
  }
  return `Not available: ${evidence.n} counted run${evidence.n === 1 ? '' : 's'} on the ${side}, below the criterion's minimum of ${minimum}.`
}

/**
 * Why "validated improvement" cannot be recorded, or `undefined` when it can.
 * `eval::review` refuses the same first five cases; the comparability check is
 * the monitor's own (runs that differ in more than the Harness prove nothing).
 */
export function improvementRefusal(review: SuggestionReview, links: ValidationLink[]): string | undefined {
  const criterion = review.criterion
  if (!criterion) return 'Validated improvement needs a criterion. Register it, then run again.'
  const first = firstDataAt(review, links)
  if (first === undefined) return 'Not available: no E2E runs are attached or running for this suggestion.'
  if (criterion.registered_at > first) {
    return `Not available: the criterion was registered at ${formatClock(criterion.registered_at)}, after the first run (${formatClock(first)}). A criterion chosen after seeing results can't validate an improvement.`
  }
  const evidence = review.evidence
  if (!evidence) return 'Not available: no evidence was computed from the attached runs.'
  if (evidence.scenario_id !== criterion.scenario_id) {
    return `Not available: the evidence is of ${evidence.scenario_id}, the criterion measures ${criterion.scenario_id}.`
  }
  for (const side of ['baseline', 'candidate'] as const) {
    if (evidence[side].n < criterion.min_runs) return shortOfRuns(side, evidence[side], criterion.min_runs)
  }
  const pair = linksOf(review, links).sort((a, b) => b.attached_at - a.attached_at)[0]
  if (pair && !pair.comparability.comparable) {
    const fields = mismatches(pair.comparability.checks).map((check) => checkLabel(check.field).text)
    return `Not available: the runs aren't comparable (${fields.join(', ')} differ).`
  }
  return undefined
}

/** The person recorded something other than what the code computed. */
export function disagrees(review: SuggestionReview): boolean {
  return (
    review.verdict !== undefined &&
    review.evidence !== undefined &&
    review.verdict.outcome !== review.evidence.computed_outcome
  )
}

// --- after release -----------------------------------------------------------

type Reading = 'fell' | 'still' | 'unseen'

/** `unseen`: the pattern was not there before, so there is nothing to fall from. `fell`: at most half of what it was. */
export function readPattern(before: RecurrencePattern | undefined, after: RecurrencePattern | undefined): Reading {
  const was = before?.per_analysis ?? 0
  if (was === 0) return 'unseen'
  return (after?.per_analysis ?? 0) <= was / 2 ? 'fell' : 'still'
}

// --- an execution this worker started -------------------------------------------

/** The validation is still moving: the sweep advances it and the card follows. */
export function runActive(run: ValidationRun | undefined): boolean {
  return run !== undefined && (run.state === 'starting' || run.state === 'running' || run.state === 'finished')
}

/** The scenarios the criterion form can measure: the plan's, the run's, the criterion's. */
export function scenarioChoices(review: SuggestionReview, planScenario: string | null): string[] {
  return [
    ...new Set([review.criterion?.scenario_id, review.scenario_id, planScenario, review.run?.scenario_id]),
  ].filter((id): id is string => Boolean(id))
}
