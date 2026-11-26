/**
 * When turns that no user message started began, by harness turn id: the
 * turns `harness::retry` runs (the limit card's "Continue in", "Try again"),
 * timed by this tab as it asks for them. The transcript keeps no turn start,
 * so a re-run reply's duration reads it here; a re-run this tab didn't ask
 * for (another client, or before a reload) has none, and its reply shows no
 * duration rather than one measured from the failed turn's message.
 */

/** Enough for every re-run a tab sees; the oldest go first. */
const KEPT = 200
const starts = new Map<string, number>()

export function noteTurnStart(turnId: string, at: number): void {
  starts.delete(turnId)
  starts.set(turnId, at)
  while (starts.size > KEPT) {
    const oldest = starts.keys().next().value
    if (oldest === undefined) break
    starts.delete(oldest)
  }
}

export function turnStartedAt(turnId: string): number | undefined {
  return starts.get(turnId)
}

/** Tests: forget every start. */
export function clearTurnStartsForTests(): void {
  starts.clear()
}
