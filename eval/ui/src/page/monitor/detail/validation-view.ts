// Pure presentation logic of the E2E validation views (the attach dialog and
// the "E2E runs" panel): labels and values of the identity checks, the
// per-execution measures, the report notice. Unknown stays unknown, never 0.
import { formatDuration } from '@iii-dev/console-ui/format'
import { formatTokens, shortHash } from '../../../model'
import type { ComparabilityCheck, E2eExecution, E2eScenario, ValidationLink } from '../../../types'
import { formatCostShort } from './present'

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

// --- measures --------------------------------------------------------------

export interface MeasureCell {
  /** Absent when nothing was reported. */
  value?: string
  /** Quiet second line: how many runs the value covers, a conclusion… */
  sub?: string
  /** Amber line: some runs reported nothing. */
  warn?: string
}

export interface MeasureRow {
  key: string
  label: string
  baseline: MeasureCell
  candidate: MeasureCell
}

type MeasureKey = keyof E2eScenario['measures']

export function totalRuns(execution: E2eExecution): number {
  return execution.scenarios.reduce((sum, scenario) => sum + scenario.run_count, 0)
}

/**
 * The average of a measure over every scenario of the execution, weighted by
 * the runs that reported it, and how many runs that is. No sample, no value.
 */
export function aggregate(execution: E2eExecution, key: MeasureKey): { average?: number; samples: number } {
  let weighted = 0
  let samples = 0
  for (const scenario of execution.scenarios) {
    const measure = scenario.measures?.[key]
    if (measure?.average === undefined || measure.samples <= 0) continue
    weighted += measure.average * measure.samples
    samples += measure.samples
  }
  return samples > 0 ? { average: weighted / samples, samples } : { samples: 0 }
}

const NUMBER = new Intl.NumberFormat('en-US', { maximumFractionDigits: 1 })

const MEASURES: Array<{ key: MeasureKey; label: string; format: (average: number) => string }> = [
  { key: 'function_calls', label: 'Function calls · average', format: (average) => NUMBER.format(average) },
  { key: 'tokens', label: 'Tokens · average', format: (average) => formatTokens(Math.round(average)) },
  { key: 'duration_seconds', label: 'Duration · average', format: (average) => formatDuration(average * 1000) },
  { key: 'cost_usd', label: 'Cost · average', format: (average) => formatCostShort(average) },
]

function measureCell(execution: E2eExecution, key: MeasureKey, format: (average: number) => string): MeasureCell {
  const runs = totalRuns(execution)
  const { average, samples } = aggregate(execution, key)
  if (runs === 0) return {}
  const missing = runs - samples
  const warn = missing > 0 ? `Not reported in ${missing} of ${runs} runs` : undefined
  if (average === undefined) return { warn }
  return { value: format(average), sub: `${samples} of ${runs} runs`, warn }
}

function assessmentsCell(execution: E2eExecution): MeasureCell {
  const { assessments, conclusion } = execution
  return {
    value: assessments ? `${assessments.passed} / ${assessments.total}` : undefined,
    sub: conclusion ? `conclusion: ${conclusion}` : undefined,
  }
}

/** The rows of the "per execution" table: each execution on its own, no difference column. */
export function measureRows(baseline: E2eExecution, candidate: E2eExecution): MeasureRow[] {
  const runs = (execution: E2eExecution): MeasureCell => {
    const total = totalRuns(execution)
    return total > 0 ? { value: String(total) } : {}
  }
  return [
    { key: 'runs', label: 'Runs', baseline: runs(baseline), candidate: runs(candidate) },
    {
      key: 'assessments',
      label: 'Assessments passed',
      baseline: assessmentsCell(baseline),
      candidate: assessmentsCell(candidate),
    },
    ...MEASURES.map(({ key, label, format }) => ({
      key,
      label,
      baseline: measureCell(baseline, key, format),
      candidate: measureCell(candidate, key, format),
    })),
  ]
}

/** True when either execution carries anything to put in the table. */
export function hasMeasures(link: ValidationLink): boolean {
  return [link.baseline, link.candidate].some((run) => run.scenarios.length > 0 || run.assessments !== undefined)
}
