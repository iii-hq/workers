/**
 * Compute `part / total * 100`, guarded against the divide-by-zero and
 * non-finite inputs that occur for zero-duration traces (a single
 * instantaneous span, or a batch of spans sharing one timestamp, makes
 * `total_duration_ms === 0`). Returns 0 instead of NaN/Infinity and clamps
 * the result to [0, 100] so overlapping spans can't exceed the bar width.
 */
export function percentOfTotal(part: number, total: number): number {
  if (!Number.isFinite(part) || !Number.isFinite(total) || total <= 0) {
    return 0
  }
  const pct = (part / total) * 100
  return Math.min(100, Math.max(0, pct))
}
