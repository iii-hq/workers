/**
 * One relative-time formatter shared by the activity feed and the git graph.
 * Turns an elapsed-seconds delta into a short label, covering the union of both
 * former formatters' buckets: just-now (<5s) / s / m / h / d / mo / y.
 */
export function formatRelative(deltaSeconds: number): string {
  const s = Math.max(0, Math.floor(deltaSeconds))
  if (s < 5) return 'just now'
  if (s < 60) return `${s}s ago`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m ago`
  const h = Math.floor(m / 60)
  if (h < 24) return `${h}h ago`
  const d = Math.floor(h / 24)
  if (d < 30) return `${d}d ago`
  const mo = Math.floor(d / 30)
  if (mo < 12) return `${mo}mo ago`
  return `${Math.floor(d / 365)}y ago`
}
