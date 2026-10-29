/** The splices that turn one item list into another (pure; used by `createList().set`). */

/** `[start, deleteCount, items]`, applied in order like Array.prototype.splice. */
export type Splice<T> = [start: number, deleteCount: number, items: T[]]

/**
 * Splices from `prev` to `next`, by key. Items with the same key at the same
 * place are compared as JSON values (the same object is the same item, at no
 * cost); each run of changed ones is one splice.
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

/**
 * Whether two JSON values are equal as the host would read them: `===`
 * first (an app that keeps its unchanged items skips them outright), then
 * field by field, without building their JSON (a fresh copy of the worker's
 * data). Keys in another order are the same object; a field holding
 * undefined is a missing one, as in JSON; anything but a plain object or
 * an array (a Date) is compared by its JSON.
 */
export function sameJson(a: unknown, b: unknown): boolean {
  if (a === b) return true
  if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null) return false
  if (Array.isArray(a)) {
    if (!Array.isArray(b) || a.length !== b.length) return false
    for (let i = 0; i < a.length; i++) {
      const v = a[i]
      // Primitives inline: most fields are, and a call per field is what costs.
      if (v !== b[i] && (typeof v !== 'object' || v === null || !sameJson(v, b[i]))) return false
    }
    return true
  }
  if (Array.isArray(b)) return false
  // Not a plain object (a Date, a class with toJSON): what its JSON says.
  if ((a as object).constructor !== Object || (b as object).constructor !== Object) {
    return JSON.stringify(a) === JSON.stringify(b)
  }
  const x = a as Record<string, unknown>
  const y = b as Record<string, unknown>
  let count = 0
  for (const k in x) {
    const v = x[k]
    if (v === undefined) continue
    count++
    if (v !== y[k] && (typeof v !== 'object' || v === null || !sameJson(v, y[k]))) return false
  }
  // Every defined field of x is in y; y has no other one when it has no more keys.
  if (Object.keys(y).length === count) return true
  for (const k in y) if (y[k] !== undefined) count--
  return count === 0
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
    const changed = i < count && !sameJson(prev[from + i], next[at + i])
    if (changed && run < 0) run = i
    if (!changed && run >= 0) {
      out.push([at + run, i - run, next.slice(at + run, at + i)])
      run = -1
    }
  }
}
