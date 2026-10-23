export function formatDate(timestamp?: number): string {
  if (!timestamp) return '—'
  return new Date(timestamp).toLocaleString()
}
