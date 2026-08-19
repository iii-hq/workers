/** Draft-only heuristic preview and row reorder. Order is routing priority. */

export function winningHeuristicIndex(model: string, rows: { pattern: string; provider: string }[]): number | null {
  const needle = model.trim()
  if (!needle) return null
  for (let i = 0; i < rows.length; i++) {
    const pattern = rows[i].pattern
    if (!pattern || !rows[i].provider) continue
    try {
      if (new RegExp(pattern).test(needle)) return i
    } catch {
      // An invalid operator regex never takes the router down.
    }
  }
  return null
}

export function moveItem<T>(items: T[], from: number, to: number): T[] {
  if (to < 0 || to >= items.length || from === to) return items
  const next = [...items]
  const [row] = next.splice(from, 1)
  next.splice(to, 0, row)
  return next
}
