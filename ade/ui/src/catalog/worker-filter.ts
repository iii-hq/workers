/**
 * The Observers page's one filter: a worker name. Nothing here is a fixed
 * list; the names are read from the catalogue itself, so a worker that
 * connects tomorrow is offered the moment it registers something.
 *
 * A worker is involved with a function group in two ways: it hosts the
 * observed function, or it registered one of the observers under it. The
 * filter keeps both, and narrows a group's observers to the chosen worker
 * unless the worker is the host (then every observer of its function
 * stays, because that is what the operator is looking at).
 */

interface GroupLike<O> {
  source: { worker_name: string }
  observers: readonly O[]
}

interface ObserverLike {
  worker: { worker_name?: string }
}

/** Function groups each worker is involved with, by name. */
export function workerCounts<O extends ObserverLike>(groups: readonly GroupLike<O>[]): Map<string, number> {
  const counts = new Map<string, number>()
  for (const group of groups) {
    const involved = new Set<string>([group.source.worker_name])
    for (const observer of group.observers) {
      if (observer.worker.worker_name) involved.add(observer.worker.worker_name)
    }
    for (const name of involved) counts.set(name, (counts.get(name) ?? 0) + 1)
  }
  return counts
}

/** The groups a worker is involved with; `null` keeps everything. */
export function narrowToWorker<O extends ObserverLike, G extends GroupLike<O>>(
  groups: readonly G[],
  worker: string | null,
): G[] {
  if (worker === null) return [...groups]
  const out: G[] = []
  for (const group of groups) {
    if (group.source.worker_name === worker) {
      out.push(group)
      continue
    }
    const own = group.observers.filter((o) => o.worker.worker_name === worker)
    if (own.length > 0) out.push({ ...group, observers: own })
  }
  return out
}
