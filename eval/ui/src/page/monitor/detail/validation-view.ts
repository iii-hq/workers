// Pure presentation logic of the E2E validation views (the attach dialog and
// the "E2E runs" panel): labels and values of the identity checks, the
// measures of each scenario with the difference between the two sides, the
// report notice. Unknown stays unknown, never 0; the difference is computed
// here, in code, and left blank when a side reports nothing.
import { formatDuration } from '@iii-dev/console-ui/format'
import { formatCost, formatTokens, shortHash } from '../../../model'
import type {
  ComparabilityCheck,
  Criterion,
  CriterionMetric,
  E2eExecution,
  E2eMeasure,
  E2eScenario,
  Evidence,
  ValidationLink,
} from '../../../types'

/** The runs a side the E2E advises: its robustness check wants at least this many. */
export const DEFAULT_MIN_RUNS = 5

const CHECK_LABEL: Record<string, { text: string; mono: boolean }> = {
  scenarios: { text: 'Scenario', mono: false },
  behavior_sha256: { text: 'behavior_sha256', mono: true },
  contract_fingerprint: { text: 'contract_fingerprint', mono: true },
  model: { text: 'Model', mono: false },
  provider: { text: 'Provider', mono: false },
  e2e_revision: { text: 'E2E revision', mono: false },
  engine_version: { text: 'Engine version', mono: false },
}

/** Field ids of the backend's identity checks → their label; machine fields stay mono. */
export function checkLabel(field: string): { text: string; mono: boolean } {
  return CHECK_LABEL[field] ?? { text: field, mono: true }
}

export const NOT_REPORTED = 'not reported'

// `scenarios`, `behavior_sha256` and `contract_fingerprint` are one entry per
// scenario, `<scenario_id>=<value>` joined by `, ` (the backend's format).
const PER_SCENARIO = new Set(['scenarios', 'behavior_sha256', 'contract_fingerprint'])

/** A check's value as shown: hashes shortened, per-scenario lists unpacked. */
export function checkValue(field: string, value: string | undefined): string {
  if (value === undefined || value.trim() === '') return NOT_REPORTED
  if (!PER_SCENARIO.has(field)) {
    // A git revision is long and meaningless past its first seven characters.
    return /^[0-9a-f]{12,}$/i.test(value) ? value.slice(0, 7) : value
  }
  const entries = value.split(', ').map((part) => {
    const at = part.indexOf('=')
    return at < 0 ? { id: part, value: '' } : { id: part.slice(0, at), value: part.slice(at + 1) }
  })
  if (field === 'scenarios') return entries.map((entry) => entry.id).join(', ')
  const shown = entries.map((entry) => (entry.value ? shortHash(entry.value) : NOT_REPORTED))
  if (entries.length === 1) return shown[0]
  return entries.map((entry, index) => `${entry.id}=${shown[index]}`).join(', ')
}

export function mismatches(checks: ComparabilityCheck[]): ComparabilityCheck[] {
  return checks.filter((check) => !check.matches)
}

export function scenarioIds(execution: E2eExecution): string[] {
  return [...new Set(execution.scenarios.map((scenario) => scenario.scenario_id).filter(Boolean))]
}

/** The tail of "Found · …": scenarios, Harness version, a report problem. */
export function foundSummary(execution: E2eExecution): string {
  const ids = scenarioIds(execution)
  return [
    ids.length > 0 ? ids.join(', ') : '',
    execution.harness_version ? `harness ${execution.harness_version}` : '',
    execution.reports_available ? '' : 'report unavailable',
  ]
    .filter(Boolean)
    .join(' · ')
}

export type HarnessNote =
  | { kind: 'differs'; baseline: string; candidate: string }
  | { kind: 'same'; version: string }
  | { kind: 'unknown' }

/** The one value that is meant to differ: the Harness version under test. */
export function harnessNote(baseline: E2eExecution, candidate: E2eExecution): HarnessNote {
  const b = baseline.harness_version
  const c = candidate.harness_version
  if (!b || !c) return { kind: 'unknown' }
  return b === c ? { kind: 'same', version: b } : { kind: 'differs', baseline: b, candidate: c }
}

/** `21:05`, local time. */
export function formatClock(ms: number): string {
  const date = new Date(ms)
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${pad(date.getHours())}:${pad(date.getMinutes())}`
}

/** The newest link (ties go to the later entry) and how many came before it. */
export function latestLink(links: ValidationLink[]): { latest: ValidationLink; earlier: number } | null {
  if (links.length === 0) return null
  let latest = links[0]
  for (const link of links) if (link.attached_at >= latest.attached_at) latest = link
  return { latest, earlier: links.length - 1 }
}

/** Runs of the link whose report the E2E service did not return, and why. */
export function reportProblem(link: ValidationLink): { ids: string[]; errors: string[] } | null {
  const runs = [link.baseline, link.candidate]
  const broken = runs.filter((run) => !run.reports_available || run.evidence_error)
  if (broken.length === 0) return null
  return {
    ids: [...new Set(broken.map((run) => run.execution_id))],
    errors: [...new Set(broken.flatMap((run) => (run.evidence_error ? [run.evidence_error] : [])))],
  }
}

export function reportsAvailable(link: ValidationLink): boolean {
  return link.baseline.reports_available && link.candidate.reports_available
}

// --- measures per scenario ---------------------------------------------------

export interface MeasureCell {
  /** Absent when nothing was reported. */
  value?: string
  /** Quiet second line: how many runs the value covers, which statistic… */
  sub?: string
  /** Amber line: some runs reported nothing. */
  warn?: string
}

export interface DiffRow {
  key: string
  label: string
  /** `median per run`: the statistic both sides share; absent when they report different ones. */
  qualifier?: string
  /** The measure the registered criterion judges. */
  primary?: boolean
  baseline: MeasureCell
  candidate: MeasureCell
  /** Candidate minus baseline; absent when either side reports nothing or they report different statistics. */
  delta?: { abs: string; pct?: string }
}

export interface ScenarioTable {
  scenarioId: string
  target: boolean
  /** `5 runs per side`, or each side's own count when they differ. */
  runs: string
  /** Set when a side has fewer runs than the E2E advises: the differences below are descriptive only. */
  caution?: string
  rows: DiffRow[]
}

const NUMBER = new Intl.NumberFormat('en-US', { maximumFractionDigits: 1 })

/** One statistic of a measure for one side; `samples` is how many runs it covers when the E2E says. */
interface Reading {
  qualifier: string
  value?: number
  samples?: number
}

interface Def {
  key: string
  label: string
  metric?: CriterionMetric
  /** Listed even when neither side reports it: an unreported cost must be seen. */
  always?: boolean
  /** The difference of two percentages is in points, not a relative change. */
  points?: boolean
  format: (value: number) => string
  /** Statistics in order of preference: both sides are compared on the first both report. */
  readings: (scenario: E2eScenario) => Reading[]
}

/** The mean of a measure over the runs that reported it; no sample, no value. */
const mean = (measure: E2eMeasure | undefined): number | undefined =>
  measure && measure.samples > 0 ? measure.average : undefined

const DEFS: Def[] = [
  {
    key: 'pass_rate',
    label: 'Pass rate',
    metric: 'pass_rate',
    points: true,
    format: (value) => `${Math.round(value)} %`,
    readings: (s) => [{ qualifier: '', value: s.pass_rate === undefined ? undefined : s.pass_rate * 100 }],
  },
  {
    key: 'function_calls',
    label: 'Function calls',
    metric: 'function_calls',
    format: (value) => NUMBER.format(value),
    readings: (s) => [
      { qualifier: 'median per run', value: s.p50_function_calls },
      { qualifier: 'mean per run', value: mean(s.measures?.function_calls) },
    ],
  },
  {
    key: 'function_call_errors',
    label: 'Function call errors',
    format: (value) => NUMBER.format(value),
    readings: (s) => [{ qualifier: 'mean per run', value: mean(s.measures?.function_call_errors) }],
  },
  {
    key: 'cost_usd',
    label: 'Cost',
    metric: 'cost_usd',
    always: true,
    format: formatCost,
    readings: (s) => {
      // The sample count belongs to the measure: a cost taken from the cohort has none to show.
      const measured = mean(s.measures?.cost_usd)
      return [
        {
          qualifier: 'per run',
          value: measured ?? s.cost_usd_per_run,
          samples: measured === undefined ? undefined : s.measures?.cost_usd?.samples,
        },
      ]
    },
  },
  {
    key: 'duration',
    label: 'Time',
    metric: 'duration',
    format: (seconds) => formatDuration(seconds * 1000),
    readings: (s) => [
      {
        qualifier: 'median per run',
        value: s.median_wall_time_ms === undefined ? undefined : s.median_wall_time_ms / 1000,
      },
      { qualifier: 'mean per run', value: mean(s.measures?.duration_seconds) },
    ],
  },
  {
    key: 'tokens',
    label: 'Tokens',
    metric: 'tokens',
    format: (value) => formatTokens(Math.round(value)),
    readings: (s) => [{ qualifier: 'per run', value: s.total_tokens_per_run ?? mean(s.measures?.tokens) }],
  },
]

/** `-27 %`, `+3`, `0`: the sign is the direction of the move, never a verdict. */
function signed(text: string, amount: number): string {
  return amount > 0 ? `+${text}` : amount < 0 ? `-${text}` : text
}

/** Candidate minus baseline, as absolute and relative change. The relative change has no baseline to start from at 0. */
export function difference(
  baseline: number,
  candidate: number,
  format: (value: number) => string,
  points = false,
): { abs: string; pct?: string } {
  const change = candidate - baseline
  if (points) return { abs: `${signed(String(Math.round(Math.abs(change))), Math.round(change))} pp` }
  const pct = baseline === 0 ? undefined : (change / baseline) * 100
  return {
    abs: signed(format(Math.abs(change)), change),
    pct: pct === undefined ? undefined : `${signed(String(Math.round(Math.abs(pct))), Math.round(pct))} %`,
  }
}

function cell(def: Def, reading: Reading | undefined, runs: number, statistic: boolean): MeasureCell {
  if (reading?.value === undefined) return { warn: 'Not reported' }
  const covered = reading.samples !== undefined && runs > 0
  return {
    value: def.format(reading.value),
    sub: covered ? `${reading.samples} of ${runs} runs` : statistic ? reading.qualifier : undefined,
    warn: covered && runs > (reading.samples ?? 0) ? `Not reported in ${runs - (reading.samples ?? 0)}` : undefined,
  }
}

function diffRow(def: Def, baseline: E2eScenario | undefined, candidate: E2eScenario | undefined, primary: boolean) {
  const forBaseline = baseline ? def.readings(baseline) : []
  const forCandidate = candidate ? def.readings(candidate) : []
  const shared = forBaseline.findIndex(
    (reading, at) => reading.value !== undefined && forCandidate[at]?.value !== undefined,
  )
  const first = (readings: Reading[]) => readings.find((reading) => reading.value !== undefined)
  const b = shared >= 0 ? forBaseline[shared] : first(forBaseline)
  const c = shared >= 0 ? forCandidate[shared] : first(forCandidate)
  if (b?.value === undefined && c?.value === undefined && !def.always) return undefined
  const row: DiffRow = {
    key: def.key,
    label: def.label,
    qualifier: shared >= 0 ? b?.qualifier || undefined : undefined,
    primary: primary || undefined,
    baseline: cell(def, b, baseline?.run_count ?? 0, shared < 0),
    candidate: cell(def, c, candidate?.run_count ?? 0, shared < 0),
  }
  if (shared >= 0 && b?.value !== undefined && c?.value !== undefined) {
    row.delta = difference(b.value, c.value, def.format, def.points)
  }
  return row
}

/** The signal the criterion counts, from the evidence the code computed over each run's transcript. */
export function signalRow(criterion: Criterion | undefined, evidence: Evidence | undefined): DiffRow | undefined {
  if (!criterion || criterion.metric !== 'signal_per_run' || !evidence) return undefined
  const { baseline, candidate } = evidence
  const side = (n: number, value: number | undefined): MeasureCell =>
    value === undefined ? {} : { value: NUMBER.format(value), sub: `n=${n}` }
  return {
    key: 'signal',
    label: criterion.pattern?.split(':')[0] ?? 'signal',
    qualifier: 'signals per run',
    primary: true,
    baseline: side(baseline.n, baseline.mean),
    candidate: side(candidate.n, candidate.mean),
    delta:
      baseline.mean !== undefined && candidate.mean !== undefined
        ? difference(baseline.mean, candidate.mean, (value) => NUMBER.format(value))
        : undefined,
  }
}

/**
 * One table per scenario the executions ran, the target first, each with the
 * measures the E2E reported for it. Scenarios are never averaged together: a
 * mean over different scenarios describes none of them.
 */
export function scenarioTables(
  link: ValidationLink,
  options: { target?: string; primary?: CriterionMetric; signal?: DiffRow },
): ScenarioTable[] {
  const ids = [
    ...new Set([...link.baseline.scenarios, ...link.candidate.scenarios].map((scenario) => scenario.scenario_id)),
  ]
  const target = options.target && ids.includes(options.target) ? options.target : ids[0]
  return [target, ...ids.filter((id) => id !== target)]
    .filter((id): id is string => id !== undefined)
    .map((id) => {
      const baseline = link.baseline.scenarios.find((scenario) => scenario.scenario_id === id)
      const candidate = link.candidate.scenarios.find((scenario) => scenario.scenario_id === id)
      const rows = DEFS.flatMap((def) => diffRow(def, baseline, candidate, def.metric === options.primary) ?? [])
      const counts = [baseline?.run_count, candidate?.run_count]
      const fewest = Math.min(counts[0] ?? 0, counts[1] ?? 0)
      return {
        scenarioId: id,
        target: id === target,
        runs:
          counts[0] !== undefined && counts[0] === counts[1]
            ? `${counts[0]} ${counts[0] === 1 ? 'run' : 'runs'} per side`
            : `${counts[0] ?? 0} baseline · ${counts[1] ?? 0} candidate runs`,
        caution:
          fewest < DEFAULT_MIN_RUNS
            ? `${counts[0] === counts[1] ? `n=${fewest} per side` : `n=${counts[0] ?? 0} and ${counts[1] ?? 0}`}; the E2E advises at least ${DEFAULT_MIN_RUNS}, so these differences are descriptive only`
            : undefined,
        rows: id === target && options.signal ? [options.signal, ...rows] : rows,
      }
    })
}

/** True when either execution carries a scenario to put in the table. */
export function hasMeasures(link: ValidationLink): boolean {
  return [link.baseline, link.candidate].some((run) => run.scenarios.length > 0)
}

/** The plan's one line above the pickers: what to pick, from `validation.scenario_id`. `code` spans are backticked. */
export function planHint(scenarioId: string | null): string {
  return scenarioId
    ? `Run \`${scenarioId}\` twice with the same model: the baseline on the current Harness, the candidate with this change.`
    : 'This plan needs a new E2E case. Pick two runs of the same scenarios and model: the baseline without this change, the candidate with it.'
}

/** The E2E service listed no executions at all. */
export function noRunsNotice(scenarioId: string | null): { headline: string; detail: string } {
  return {
    headline: 'No E2E runs yet',
    detail: `The E2E service has no executions to attach. Run ${scenarioId ?? 'the case'} twice there, baseline first, then come back.`,
  }
}
