/** The splices that turn one item list into another (pure; used by `createList().set`). */

/** `[start, deleteCount, items]`, applied in order like Array.prototype.splice. */
export type Splice<T> = [start: number, deleteCount: number, items: T[]]

/**
 * Splices from `prev` to `next`, by key. Items with the same key at the same
 * place are compared by their JSON; each run of changed ones is one splice.
 * Whatever differs between the common head and the common tail of keys is
 * one splice (a reorder of everything is `[0, prev.length, next]`).
 */
export function diffItems<T>(prev: readonly T[], next: readonly T[], keyOf: (item: T) => unknown): Splice<T>[] {
  let head = 0
  while (head < prev.length && head < next.length && keyOf(prev[head]) === keyOf(next[head])) head++
  let tail = 0
  while (
    tail < prev.length - head &&
    tail < next.length - head &&
    keyOf(prev[prev.length - 1 - tail]) === keyOf(next[next.length - 1 - tail])
  ) {
    tail++
  }
  const out: Splice<T>[] = []
  changedRuns(prev, next, 0, 0, head, out)
  const prevMiddle = prev.length - head - tail
  const nextMiddle = next.length - head - tail
  if (prevMiddle || nextMiddle) out.push([head, prevMiddle, next.slice(head, head + nextMiddle)])
  changedRuns(prev, next, prev.length - tail, next.length - tail, tail, out)
  return out
}

/** One splice per run of items (same keys, same places) whose JSON changed. */
function changedRuns<T>(
  prev: readonly T[],
  next: readonly T[],
  from: number,
  at: number,
  count: number,
  out: Splice<T>[],
) {
  let run = -1
  for (let i = 0; i <= count; i++) {
    const changed = i < count && JSON.stringify(prev[from + i]) !== JSON.stringify(next[at + i])
    if (changed && run < 0) run = i
    if (!changed && run >= 0) {
      out.push([at + run, i - run, next.slice(at + run, at + i)])
      run = -1
    }
  }
}
