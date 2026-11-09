/** Pure list helpers for the library column (grouping and search), kept
 * free of console imports so tests can load them. */

import type { BrowserRow } from './browser'

export interface LibraryGroup {
  id: string
  label: string
  rows: BrowserRow[]
}

/** Group rows by `row.group` (rows without one land in `fallbackLabel`),
 * keeping the adapter's group order, then the rest alphabetically. */
export function groupRows(rows: BrowserRow[], order: readonly string[] = [], fallbackLabel = 'Other'): LibraryGroup[] {
  const groups = new Map<string, LibraryGroup>()
  for (const row of rows) {
    const id = row.group ?? ''
    let group = groups.get(id)
    if (!group) {
      group = { id, label: row.groupLabel ?? (id || fallbackLabel), rows: [] }
      groups.set(id, group)
    }
    group.rows.push(row)
  }
  // Listed ids first, then named groups alphabetically, then the rows
  // without a group.
  const rank = (id: string) => {
    const i = order.indexOf(id)
    if (i !== -1) return i
    return id ? order.length : order.length + 1
  }
  return [...groups.values()].sort((a, b) => rank(a.id) - rank(b.id) || a.label.localeCompare(b.label))
}

/** Whether a group renders folded: only when headers show (more than one
 * group) and never while a search is filtering the list. */
export function isGroupCollapsed(
  id: string,
  { groupCount, filtering, collapsed }: { groupCount: number; filtering: boolean; collapsed: readonly string[] },
): boolean {
  return !filtering && groupCount > 1 && collapsed.includes(id)
}

/** The first row ON SCREEN — group order first, folded groups skipped — which
 * is what Enter in the search box opens (the same row ↓ lands on). */
export function firstShownRow(
  groups: LibraryGroup[],
  { filtering, collapsed }: { filtering: boolean; collapsed: readonly string[] },
): BrowserRow | undefined {
  const open = groups.find(
    (g) => g.rows.length > 0 && !isGroupCollapsed(g.id, { groupCount: groups.length, filtering, collapsed }),
  )
  return open?.rows[0]
}

/** Case-insensitive match over the key, title and description. */
export function matchesQuery(row: BrowserRow, needle: string): boolean {
  if (!needle) return true
  return (
    row.key.toLowerCase().includes(needle) ||
    row.title.toLowerCase().includes(needle) ||
    row.description.toLowerCase().includes(needle)
  )
}

const pluralRules = new Intl.PluralRules('en-US')

/** `1 skill` / `3 skills`: the whole phrase per plural category, never a
 * suffix glued onto a noun. */
export function countLabel(n: number, forms: { one: string; other: string }): string {
  return `${n} ${pluralRules.select(n) === 'one' ? forms.one : forms.other}`
}

/** Edit distance (insert, delete, substitute), for typo-tolerant hints. */
function editDistance(a: string, b: string): number {
  if (a === b) return 0
  let prev = Array.from({ length: b.length + 1 }, (_, i) => i)
  for (let i = 1; i <= a.length; i += 1) {
    const next = [i]
    for (let j = 1; j <= b.length; j += 1) {
      next[j] = Math.min(prev[j] + 1, next[j - 1] + 1, prev[j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1))
    }
    prev = next
  }
  return prev[b.length]
}

/** Lower-cased alphanumeric words of a name (`ade/console-ui` → ade, console, ui). */
const words = (text: string) =>
  text
    .toLowerCase()
    .split(/[^a-z0-9]+/)
    .filter(Boolean)

/** Length of the shared prefix of two strings. */
function commonPrefix(a: string, b: string): number {
  let i = 0
  while (i < a.length && i < b.length && a[i] === b[i]) i += 1
  return i
}

/**
 * When a search finds nothing, the rows it most likely meant: rows sharing a
 * word with the query (allowing a typo or two per word) first, then rows
 * whose name is a short edit away. `console-innegne` → console-injectable-ui,
 * console-ui-patterns…; `ochestration` → orchestration.
 */
export function closestMatches(rows: BrowserRow[], query: string, limit = 3): BrowserRow[] {
  const needle = query.trim().toLowerCase()
  if (!needle) return []
  const tokens = words(needle).filter((t) => t.length >= 3)
  const scored: { row: BrowserRow; hits: number; prefix: number; distance: number }[] = []
  for (const row of rows) {
    const last = row.key.slice(row.key.lastIndexOf('/') + 1)
    const names = [row.key, last, row.title].map((n) => n.toLowerCase())
    const rowWords = names.flatMap(words)
    const hits = tokens.filter((t) =>
      rowWords.some((w) => w.startsWith(t) || (t.length >= 5 && editDistance(t, w) <= Math.floor(t.length / 4))),
    ).length
    // How far each query word runs into some word of the row (`innegne` →
    // `injectable` shares "in"): ranks rows that tie on whole-word hits.
    const prefix = tokens.reduce((sum, t) => sum + Math.max(0, ...rowWords.map((w) => commonPrefix(t, w))), 0)
    const distance = Math.min(...names.map((n) => editDistance(needle, n) / Math.max(needle.length, n.length)))
    if (hits > 0 || distance <= 0.34) scored.push({ row, hits, prefix, distance })
  }
  scored.sort(
    (a, b) => b.hits - a.hits || b.prefix - a.prefix || a.distance - b.distance || a.row.key.localeCompare(b.row.key),
  )
  return scored.slice(0, limit).map((s) => s.row)
}
