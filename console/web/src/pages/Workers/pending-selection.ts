/**
 * A one-shot hand-off for "open the workers page, showing THIS worker".
 *
 * The workers page owns its filter state locally and takes no props, and the
 * palette attaches it as a workspace screen rather than navigating by hash, so
 * there is no route to carry a selection through. This is the smallest honest
 * contract instead: the caller leaves a term, the page takes it once on mount,
 * and it is gone — a later visit opens unfiltered.
 */

let pending: string | null = null

/** Ask the workers page to open filtered to this worker (or function id). */
export function setPendingWorkerSearch(term: string): void {
  pending = term.trim() || null
}

/** Read and clear. Returns null when nothing asked for a selection. */
export function takePendingWorkerSearch(): string | null {
  const term = pending
  pending = null
  return term
}
