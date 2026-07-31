/**
 * How long a function call took, for the chat surface's card header.
 *
 * Milliseconds above a millisecond, exactly as before. Below one, the same
 * `μs` the traces surface uses (`pages/TracesV2/lib/traceUtils.ts`): a lot of
 * what an agent dispatches is engine-local — a trigger registration, a state
 * read — and finishes in microseconds, where rounding to `0ms` reads like the
 * call never happened.
 */
export function formatCallDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return '0μs'
  if (ms < 1) return `${Math.max(1, Math.round(ms * 1000))}μs`
  return `${Math.round(ms)}ms`
}
