import type { SessionCronTask, SystemCronBinding } from '../lib/api'

/** What a schedule is doing, from the only facts a subscription carries:
    its fire count, its cap, and its expiry. There is no paused state in the
    harness surface and no per-fire outcome, so neither is invented here. */
export type TaskStatus = 'active' | 'ending' | 'finished'

export interface StatusView {
  status: TaskStatus
  label: string
  tone: 'accent' | 'warn' | 'ink'
}

/** A schedule about to stop deserves warning before it goes quiet: inside a
    day of expiry, or one fire from its cap. */
const ENDING_SOON_MS = 24 * 60 * 60 * 1000

export function taskStatus(task: SessionCronTask, now: number): TaskStatus {
  const capped = task.once || task.maxFires !== undefined
  const cap = task.once ? 1 : task.maxFires
  if (cap !== undefined && task.fires >= cap) return 'finished'
  if (task.expiresAt !== undefined && task.expiresAt <= now) return 'finished'
  if (task.expiresAt !== undefined && task.expiresAt - now <= ENDING_SOON_MS) {
    return 'ending'
  }
  if (capped && cap !== undefined && task.fires >= cap - 1) return 'ending'
  return 'active'
}

export function statusView(task: SessionCronTask, now: number): StatusView {
  const status = taskStatus(task, now)
  if (status === 'finished') return { status, label: 'Finished', tone: 'ink' }
  if (status === 'ending') return { status, label: 'Ending soon', tone: 'warn' }
  return { status, label: 'Active', tone: 'accent' }
}

export type Filter = 'all' | TaskStatus

export function countByStatus(tasks: readonly SessionCronTask[], now: number): Record<Filter, number> {
  const counts: Record<Filter, number> = {
    all: tasks.length,
    active: 0,
    ending: 0,
    finished: 0,
  }
  for (const task of tasks) counts[taskStatus(task, now)] += 1
  return counts
}

export function matchesFilter(task: SessionCronTask, filter: Filter, now: number): boolean {
  return filter === 'all' || taskStatus(task, now) === filter
}

/** Free-text match over what an operator can actually read in the row. */
export function matchesQuery(task: SessionCronTask, cadence: string, query: string): boolean {
  const needle = query.trim().toLowerCase()
  if (!needle) return true
  return [task.label, task.target, task.expression, cadence, task.subscriptionId].some((value) =>
    (value ?? '').toLowerCase().includes(needle),
  )
}

/** The same contract for the bindings tab: every field the row shows. */
export function matchesBindingQuery(binding: SystemCronBinding, query: string): boolean {
  const needle = query.trim().toLowerCase()
  if (!needle) return true
  return [binding.functionId, binding.workerName, binding.expression, binding.id].some((value) =>
    value.toLowerCase().includes(needle),
  )
}

/** Soonest next run first; a schedule with no computable next run sinks.
    Takes the run times already computed for the list, because parsing an
    expression inside a comparator repeats that work log n times per row. */
export function byNextRun(next: (task: SessionCronTask) => number): (a: SessionCronTask, b: SessionCronTask) => number {
  return (a, b) => {
    const left = next(a)
    const right = next(b)
    if (left !== right) return left - right
    return b.createdAt - a.createdAt
  }
}
