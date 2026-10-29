// The E2E runs the attach dialog offers, read from
// `e2e::dashboard::executions-list`. Only what the picker shows is kept: the
// list answers with full summaries (stack, metrics, progress) per run.
import type { SelectorGroup } from '@iii-dev/console-ui'

export interface E2eRun {
  id: string
  label: string
  status: string
  startedAt?: number
  model?: string
  provider?: string
  scenarios: string[]
  /** `1.8.42 · 57c6d33`, from the stack the run recorded. */
  harness?: string
}

type Json = Record<string, unknown>

const text = (value: unknown): string | undefined =>
  typeof value === 'string' && value.trim() ? value.trim() : undefined

const list = (value: unknown): Json[] => (Array.isArray(value) ? (value as Json[]) : [])

function harnessOf(stack: unknown): string | undefined {
  const entry = list(stack).find((item) => item?.name === 'harness')
  const version = text(entry?.observed) ?? text(entry?.requested)
  if (!version) return undefined
  const commit = text(entry?.commit)?.slice(0, 7)
  return [version, commit, entry?.dirty === true ? 'uncommitted changes' : undefined].filter(Boolean).join(' · ')
}

/** Plan runs carry `parameters`; older ones only `subjects` and `scenario_metrics`. */
function parseRun(run: Json): E2eRun | undefined {
  const id = text(run.id)
  if (!id) return undefined
  const parameters = (run.parameters ?? {}) as Json
  const subjects = list(run.subjects)
  const subject = subjects.length === 1 ? subjects[0] : undefined
  const field = (key: string) => text(parameters[key]) ?? text(subject?.[key])
  const scenarioSets = [
    list(parameters.scenarios).map((name) => text(name)),
    list(run.scenario_metrics).map((metric) => text(metric?.scenario_id)),
    subjects.flatMap((item) => list(item?.scenarios).map((scenario) => text(scenario?.id))),
  ].map((names) => [...new Set(names.filter((name): name is string => !!name))].sort())
  const started = Date.parse(text(run.started_at) ?? '')
  return {
    id,
    label: text(run.label) ?? 'Unnamed run',
    status: text(run.status) ?? 'unknown',
    startedAt: Number.isNaN(started) ? undefined : started,
    model: field('model'),
    provider: field('provider'),
    scenarios: scenarioSets.find((names) => names.length > 0) ?? [],
    harness: harnessOf(run.stack),
  }
}

export function e2eRuns(response: unknown): E2eRun[] {
  return list((response as Json | null)?.executions).flatMap((run) => parseRun(run) ?? [])
}

/** The same case: model, provider and scenarios. */
export function sameCase(a: E2eRun, b: E2eRun): boolean {
  return (
    !!a.model &&
    a.model === b.model &&
    a.provider === b.provider &&
    a.scenarios.length > 0 &&
    a.scenarios.join('\n') === b.scenarios.join('\n')
  )
}

const MINUTE = 60_000
const AGES: [unit: Intl.RelativeTimeFormatUnit, size: number][] = [
  ['year', 365 * 24 * 60 * MINUTE],
  ['month', 30 * 24 * 60 * MINUTE],
  ['day', 24 * 60 * MINUTE],
  ['hour', 60 * MINUTE],
  ['minute', MINUTE],
]

/** `3 hours ago`, `12 minutes ago`: the run list has room for words, `formatRelative` abbreviates. */
function ago(then: number, now: number): string {
  const span = Math.max(0, now - then)
  for (const [unit, size] of AGES) {
    if (span >= size)
      return new Intl.RelativeTimeFormat('en', { numeric: 'always' }).format(-Math.floor(span / size), unit)
  }
  return 'just now'
}

/** One line under the run's name: what tells two runs apart. */
export function runDescription(run: E2eRun, scenarioId: string | null, now = Date.now()): string {
  const first = scenarioId && run.scenarios.includes(scenarioId) ? scenarioId : run.scenarios[0]
  const scenarios = first && run.scenarios.length > 1 ? `${first} +${run.scenarios.length - 1}` : first
  return [
    run.status.replace(/_/g, ' '),
    run.harness ? `Harness ${run.harness}` : undefined,
    run.model,
    scenarios,
    run.startedAt ? ago(run.startedAt, now) : undefined,
  ]
    .filter(Boolean)
    .join(' · ')
}

function runName(run: E2eRun): string {
  return run.label === 'Unnamed run' ? `Unnamed run · ${run.id.slice(0, 13)}` : run.label
}

/** `passed`/`failed`: the runs a pair can be made of; the others are listed after them. */
const finished = (run: E2eRun): boolean => run.status === 'passed' || run.status === 'failed'

/**
 * Newest first, in up to three groups: runs of the same case as the run picked
 * on the other side (`other`: same model, provider and scenarios, the pairs
 * that can be compared), then runs that include the plan's scenario, then the
 * rest. Within a group the runs that did not finish (running, failed to run)
 * stay listed after the ones that did. The run picked on the other side is
 * shown but cannot be picked again.
 */
export function runGroups(
  runs: E2eRun[],
  scenarioId: string | null,
  other: E2eRun | undefined,
  otherSide: 'baseline' | 'candidate',
  now = Date.now(),
): SelectorGroup[] {
  const option = (run: E2eRun) => ({
    value: run.id,
    label: runName(run),
    description:
      run.id === other?.id
        ? [`Already chosen as the ${otherSide}`, run.harness ? `Harness ${run.harness}` : undefined]
            .filter(Boolean)
            .join(' · ')
        : runDescription(run, scenarioId, now),
    // The id is searchable by the prefix a person can read off another screen; a whole 32-hex id matches any short query.
    keywords: [run.id.slice(0, 13), run.harness, run.model, ...run.scenarios].filter((word): word is string => !!word),
    disabled: run.id === other?.id,
  })
  const newest = [...runs].sort((a, b) => (b.startedAt ?? 0) - (a.startedAt ?? 0))
  const sunk = (list: E2eRun[]) => [...list.filter(finished), ...list.filter((run) => !finished(run))]
  const used = new Set<E2eRun>()
  const groups: SelectorGroup[] = []
  const add = (label: string, pool: E2eRun[]) => {
    const fresh = pool.filter((run) => !used.has(run))
    for (const run of fresh) used.add(run)
    if (fresh.length) groups.push({ label, options: sunk(fresh).map(option) })
  }
  // A run of another case than the plan's scenario is no pair for this plan, so it is not put first.
  if (other && (!scenarioId || other.scenarios.includes(scenarioId))) {
    add(
      `Same scenarios and model as the ${otherSide}`,
      newest.filter((run) => sameCase(run, other)),
    )
  }
  if (scenarioId) {
    add(
      `${groups.length ? 'Other runs with' : 'Runs with'} ${scenarioId}`,
      newest.filter((run) => run.scenarios.includes(scenarioId)),
    )
  }
  add(groups.length ? 'Other runs' : 'Recent runs', newest)
  return groups
}

/** A picked id the list does not hold (the list was read again after the pick) still shows, by its id. */
export function withPicked(groups: SelectorGroup[], id: string): SelectorGroup[] {
  if (!id || groups.some((group) => group.options.some((option) => option.value === id))) return groups
  return [...groups, { label: '', options: [{ value: id, label: id }] }]
}
