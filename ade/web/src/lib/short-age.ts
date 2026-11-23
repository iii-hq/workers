const DAY_MS = 86_400_000

/**
 * An age one short unit wide, the conversation list's words: `now`, `5m`,
 * `2h`, `3d`, then `2w`, `4mo`, `1y` so an old chat never reads as `143d`.
 * The list rows, the "by agent · 2m" pill and the context pane share it.
 */
export function shortAge(ts: number, now = Date.now()): string {
  const delta = now - ts
  if (delta < 60_000) return 'now'
  if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m`
  if (delta < DAY_MS) return `${Math.floor(delta / 3_600_000)}h`
  const days = Math.floor(delta / DAY_MS)
  if (days < 14) return `${days}d`
  if (days < 60) return `${Math.floor(days / 7)}w`
  if (days < 365) return `${Math.floor(days / 30)}mo`
  return `${Math.floor(days / 365)}y`
}
