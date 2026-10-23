// Pure logic of "Validate in E2E": what it will probably cost and take, from
// the executions the E2E already ran; which models Docker has run; the run
// count; and which field a refusal from `eval::start-validation` belongs
// under. No React.
import type { ResolvedCommit, ValidationResolution } from '../../../types'
import { MAX_RUNS } from './review-model'

export interface Estimate {
  /** Past Docker executions the figures come from. */
  executions: number
  /** USD for both sides together; absent when none of the past executions reported a cost. */
  cost?: { usd: number; from: number }
  /** Minutes both sides take together (they run in parallel), without building the Docker image. */
  minutes?: { value: number; from: number }
}

type Json = Record<string, unknown>

const rows = (value: unknown): Json[] => (Array.isArray(value) ? (value as Json[]) : [])
const num = (value: unknown): number | undefined =>
  typeof value === 'number' && Number.isFinite(value) ? value : undefined

/** Past executions needed before a figure is shown: one or two say nothing about the spread. */
export const MIN_PAST = 3

function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b)
  const middle = Math.floor(sorted.length / 2)
  return sorted.length % 2 === 1 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2
}

/**
 * The median for `runs` runs a side, from past Docker executions of the same scenario with the same model and
 * provider that finished their runs (`e2e::dashboard::executions-list`): local and GitHub executions run elsewhere
 * and say nothing about Docker. Cost is the mean per run the E2E reported, time is each execution's wall time per
 * planned run; each says how many executions it comes from. Fewer than `MIN_PAST` comparable executions, no
 * estimate: nothing is guessed.
 */
export function estimateFor(
  response: unknown,
  query: { scenarioId: string; model: string; provider: string; runs: number },
): Estimate | undefined {
  const costs: number[] = []
  const minutes: number[] = []
  let executions = 0
  for (const row of rows((response as Json | null)?.executions)) {
    const parameters = (row.parameters ?? {}) as Json
    if (parameters.where !== 'docker') continue
    if (parameters.model !== query.model || parameters.provider !== query.provider) continue
    if (row.status !== 'passed' && row.status !== 'failed') continue
    const metric = rows(row.scenario_metrics).find((item) => item.scenario_id === query.scenarioId)
    if (!metric) continue
    executions += 1
    const averages = (metric.averages ?? {}) as Json
    const samples = (metric.samples ?? {}) as Json
    const cost = num(averages.cost_usd)
    if (cost !== undefined && (num(samples.cost_usd) ?? 0) > 0) costs.push(cost)
    const started = Date.parse(String(row.started_at ?? ''))
    const finished = Date.parse(String(row.completed_at ?? ''))
    const planned = (num(parameters.runs) ?? 0) * Math.max(1, rows(parameters.scenarios).length)
    if (Number.isFinite(started) && Number.isFinite(finished) && finished > started && planned > 0) {
      minutes.push((finished - started) / planned / 60_000)
    }
  }
  if (executions < MIN_PAST) return undefined
  return {
    executions,
    cost: costs.length > 0 ? { usd: median(costs) * query.runs * 2, from: costs.length } : undefined,
    minutes: minutes.length > 0 ? { value: median(minutes) * query.runs, from: minutes.length } : undefined,
  }
}

/** `≈ $0.45`: money to the cent, cheaper than a cent to the hundredth of a cent. */
export function costApprox(usd: number): string {
  return `≈ $${usd < 0.01 ? usd.toFixed(4) : usd.toFixed(2)}`
}

/** `about 12 min`: whole minutes. */
export function minutesApprox(minutes: number): string {
  return `about ${Math.max(1, Math.round(minutes))} min`
}

export interface ModelChoice {
  model: string
  provider: string
}

/** What the E2E has run in Docker: the providers its passed executions used, and the newest model among them. */
export interface DockerHistory {
  providers: ReadonlySet<string>
  latest?: ModelChoice
}

export function dockerHistory(response: unknown): DockerHistory {
  const providers = new Set<string>()
  let latest: (ModelChoice & { at: number }) | undefined
  for (const row of rows((response as Json | null)?.executions)) {
    const parameters = (row.parameters ?? {}) as Json
    if (parameters.where !== 'docker' || row.status !== 'passed') continue
    const { model, provider } = parameters
    if (typeof model !== 'string' || typeof provider !== 'string') continue
    providers.add(provider)
    const at = Date.parse(String(row.started_at ?? '')) || 0
    if (!latest || at > latest.at) latest = { model, provider, at }
  }
  return { providers, latest: latest && { model: latest.model, provider: latest.provider } }
}

/**
 * The model to preselect. The observed session's own when Docker has run its provider, or when nothing is known about
 * Docker; otherwise the newest model Docker finished with: a provider the stack does not carry would spend a Docker
 * build and fail.
 */
export function defaultModel(
  observed: ModelChoice | undefined,
  history: DockerHistory | undefined,
): ModelChoice | undefined {
  if (!history || history.providers.size === 0) return observed
  if (observed && history.providers.has(observed.provider)) return observed
  return history.latest ?? observed
}

/** Runs per side: a whole number from 1 to 20 that is at least what the criterion needs. */
export function parseRuns(text: string, minimum: number): { runs: number } | { error: string } {
  const runs = Number(text.trim())
  if (!Number.isInteger(runs) || runs < 1 || runs > MAX_RUNS)
    return { error: `1 to ${MAX_RUNS}. The E2E advises at least 5.` }
  if (runs < minimum) return { error: `The criterion needs at least ${minimum} runs per side.` }
  return { runs }
}

/** The backend's clause as a sentence: a capital, and a full stop unless it ends in punctuation. */
export function sentence(text: string): string {
  const clause = text.trim()
  if (clause === '') return clause
  return `${clause[0].toUpperCase()}${clause.slice(1)}${/[.!?]$/.test(clause) ? '' : '.'}`
}

export type FailureField = 'candidate' | 'baseline' | 'runs' | 'criterion' | 'e2e' | 'other'

export interface StartFailure {
  field: FailureField
  /** The E2E is running something else: starting later works. */
  busy?: boolean
  /** The E2E never answered: an execution may have started, so starting again would duplicate it. */
  unconfirmed?: boolean
  /** The backend's own words, without its machine prefix. */
  text: string
}

const PREFIXES = [
  'e2e_busy:',
  'e2e_start_unconfirmed:',
  'e2e_unavailable:',
  'commit_not_pushed:',
  'git_ref_invalid:',
  'git_unavailable:',
  'code_repository_required:',
  'criterion_frozen:',
]

/** Which field an `eval::start-validation` refusal belongs under; refusals come before anything starts or costs. */
export function classifyStartFailure(message: string): StartFailure {
  const prefix = PREFIXES.find((candidate) => message.includes(candidate))
  const text = (prefix ? message.slice(message.indexOf(prefix) + prefix.length) : message).trim()
  const names = (word: string) => text.includes(word)
  switch (prefix) {
    case 'e2e_busy:':
      return { field: 'e2e', busy: true, text }
    case 'e2e_start_unconfirmed:':
      return { field: 'e2e', unconfirmed: true, text }
    case 'e2e_unavailable:':
      return { field: 'e2e', text }
    case 'criterion_frozen:':
      return { field: 'criterion', text }
    case 'commit_not_pushed:':
      return { field: names('baseline commit') ? 'baseline' : 'candidate', text }
    case 'git_ref_invalid:':
      // The same commit twice and a missing merge base are both the baseline's to fix.
      return {
        field: names('the baseline ref') || names('same commit') || names('merge base') ? 'baseline' : 'candidate',
        text,
      }
    case undefined:
      return { field: names('the criterion needs') || names('runs must be') ? 'runs' : 'other', text }
    default:
      return { field: 'other', text }
  }
}

// --- the refs check: "Resolved · <sha> · pushed to <branch>" ----------------------------------------------------

/** Quiet time after the last keystroke before the refs are asked about. */
export const REF_CHECK_DELAY_MS = 400

/** Everything the backend's dry run depends on: when any of it changes, an earlier answer says nothing. */
export interface RefInputs {
  scenario: string
  candidate: string
  /** `null` is the merge-base default. */
  baseline: string | null
  runs: number
}

/** The inputs to ask about, or `undefined` while the form lacks what a dry run needs. */
export function refInputs(form: {
  scenario: string
  candidate: string
  baseline: string | null
  runs: number | undefined
}): RefInputs | undefined {
  const candidate = form.candidate.trim()
  const baseline = form.baseline === null ? null : form.baseline.trim()
  if (form.scenario === '' || candidate === '' || baseline === '' || form.runs === undefined) return undefined
  return { scenario: form.scenario, candidate, baseline, runs: form.runs }
}

export const refKey = (inputs: RefInputs): string =>
  JSON.stringify([inputs.scenario, inputs.candidate, inputs.baseline, inputs.runs])

/** Where the check stands. Each answer carries the key of the inputs it was asked for. */
export type RefCheck =
  | { phase: 'idle' }
  | { phase: 'checking'; key: string }
  | { phase: 'resolved'; key: string; resolution: ValidationResolution }
  | { phase: 'refused'; key: string; field: 'candidate' | 'baseline' | 'other'; text: string }

/** An answer: what a dry run came back with. */
export type RefAnswer = Extract<RefCheck, { phase: 'resolved' | 'refused' }>

export const resolvedAnswer = (key: string, resolution: ValidationResolution): RefAnswer => ({
  phase: 'resolved',
  key,
  resolution,
})

/**
 * A refusal under the field it belongs to. What no ref owns (no codebase directory, git or the bus not answering) is
 * `other`: a general line that can be asked again, not an error of what was typed.
 */
export function refusedAnswer(key: string, message: string): RefAnswer {
  const { field, text } = classifyStartFailure(message)
  return { phase: 'refused', key, field: field === 'candidate' || field === 'baseline' ? field : 'other', text }
}

/** Whether to ask: not while the answer for these inputs is in hand or on its way. A refusal is asked again. */
export function needsCheck(check: RefCheck, key: string): boolean {
  return check.phase === 'idle' || check.key !== key || check.phase === 'refused'
}

/**
 * The answer is kept only if it is for the call in flight. Typing since has asked about other inputs, and an answer
 * for older ones must neither show nor open Start.
 */
export function settleCheck(current: RefCheck, answer: RefAnswer): RefCheck {
  return current.phase === 'checking' && current.key === answer.key ? answer : current
}

/** The check that speaks for the inputs now in the form; none while they differ from what it was asked. */
export function checkFor(check: RefCheck, key: string | undefined): Exclude<RefCheck, { phase: 'idle' }> | undefined {
  return check.phase !== 'idle' && check.key === key ? check : undefined
}

/** Start opens only on a resolution of exactly the current inputs. */
export const startAllowed = (check: RefCheck, key: string | undefined): boolean =>
  check.phase === 'resolved' && check.key === key

/** `Resolved · 3f9c2a1b7d4e · pushed to origin/feat` */
export function resolvedLine(commit: ResolvedCommit): string {
  return `Resolved · ${commit.short} · pushed to ${commit.branch}`
}

export interface Progress {
  state: string
  /** Runs the E2E finished of the `planned` ones. */
  finished: number
  planned: number
}

/** One execution's progress out of `e2e::dashboard::executions-list`; `undefined` when the list does not hold it. */
export function executionProgress(response: unknown, executionId: string | undefined): Progress | undefined {
  if (!executionId) return undefined
  const row = rows((response as Json | null)?.executions).find((item) => item.id === executionId)
  const plan = row?.plan_execution as Json | undefined
  const planned = num(plan?.planned)
  if (!plan || planned === undefined) return undefined
  return { state: String(plan.state ?? ''), finished: num(plan.finished) ?? num(plan.completed) ?? 0, planned }
}
