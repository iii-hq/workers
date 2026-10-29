/** Keys of List items (pure; no Solid, so it runs in Node tests too). */

/**
 * The items with one entry per key: the last one, at the place of its last
 * occurrence. A List's keys must be unique (a duplicate is a protocol
 * problem the host logs, and the test harness fails on), so pass worker
 * data that may repeat a key through this before `createList().set`.
 */
export function lastPerKey<T>(items: readonly T[], key: (keyof T & string) | ((item: T) => unknown)): T[] {
  const keyOf = typeof key === 'function' ? key : (item: T) => item[key]
  const seen = new Set<unknown>()
  const out: T[] = []
  for (let i = items.length - 1; i >= 0; i--) {
    const k = keyOf(items[i])
    if (seen.has(k)) continue
    seen.add(k)
    out.push(items[i])
  }
  return out.reverse()
}
