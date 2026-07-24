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

/** Best-effort clipboard write — the copy affordance is a convenience. */
export function copyText(text: string): void {
  try {
    void navigator.clipboard?.writeText(text)
  } catch {
    // clipboard blocked/unavailable — nothing better to do here
  }
}
