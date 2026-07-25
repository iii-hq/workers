/** Cell-value helpers shared by the result grid and the row inspector. */

/** Copyable string form of a cell value. */
export function cellText(value: unknown): string {
  if (value === null || value === undefined) return 'NULL'
  if (typeof value === 'object') {
    try {
      return JSON.stringify(value)
    } catch {
      return String(value)
    }
  }
  return String(value)
}

/** Best-effort clipboard write — the copy affordance is a convenience.
 *  clipboard may be missing (non-secure context) and writeText may reject
 *  (permission denied); the `?.catch` swallows both. */
export function copyText(text: string): void {
  void navigator.clipboard?.writeText(text)?.catch(() => {})
}
