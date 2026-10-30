import type { ListGroup, ListItem } from '../model'

/** A half-open `[start, end)` span of the name the query matched. */
export type Range = readonly [number, number]

export type WorkerHit = {
  item: ListItem
  group: ListGroup['id']
  groupLabel: string
  score: number
  /** Matched spans of the name, merged and in order, for highlighting. */
  nameRanges: Range[]
  /** Matched spans of the detail line (version, checkout, error). */
  detailRanges: Range[]
}

const WORD_BREAK = /[-_./:@ ]/

/** Where `needle` starts a word inside `haystack` (after a separator). */
function wordStart(haystack: string, needle: string): number {
  let from = 0
  for (;;) {
    const at = haystack.indexOf(needle, from)
    if (at === -1) return -1
    if (at === 0 || WORD_BREAK.test(haystack[at - 1] ?? '')) return at
    from = at + 1
  }
}

/**
 * The characters of `needle` in order inside `haystack`, not necessarily
 * adjacent: `pgc` finds provider-github-copilot. Prefers word starts, and
 * scores contiguity so a tight match outranks a scattered one.
 */
function subsequence(
  haystack: string,
  needle: string,
): { score: number; ranges: Range[] } | null {
  const ranges: Range[] = []
  let score = 0
  let from = 0
  let previous = -2
  for (const char of needle) {
    let at = haystack.indexOf(char, from)
    if (at === -1) return null
    // A word-start occurrence a little further on beats the first hit.
    const boundary = wordStart(haystack.slice(from), char)
    if (boundary !== -1 && boundary + from < at + 4) at = boundary + from
    if (at === previous + 1) score += 3
    else if (at === 0 || WORD_BREAK.test(haystack[at - 1] ?? '')) score += 2
    else score += 1
    const last = ranges[ranges.length - 1]
    if (last && last[1] === at) ranges[ranges.length - 1] = [last[0], at + 1]
    else ranges.push([at, at + 1])
    previous = at
    from = at + 1
  }
  return { score, ranges }
}

/** One token against one field: the strongest kind of match it makes. */
function matchField(
  field: string,
  token: string,
  weights: { exact: number; prefix: number; word: number; inside: number },
  fuzzy: boolean,
): { score: number; ranges: Range[] } | null {
  const value = field.toLowerCase()
  if (!value) return null
  if (value === token)
    return { score: weights.exact, ranges: [[0, token.length]] }
  if (value.startsWith(token))
    return { score: weights.prefix, ranges: [[0, token.length]] }
  const word = wordStart(value, token)
  if (word !== -1)
    return { score: weights.word, ranges: [[word, word + token.length]] }
  const inside = value.indexOf(token)
  if (inside !== -1)
    return { score: weights.inside, ranges: [[inside, inside + token.length]] }
  if (!fuzzy || token.length < 2) return null
  const loose = subsequence(value, token)
  // Scattered single characters are noise; ask for some shape.
  return loose && loose.score >= token.length * 1.5
    ? {
        score: Math.min(weights.inside - 10, loose.score * 2),
        ranges: loose.ranges,
      }
    : null
}

const NAME = { exact: 100, prefix: 80, word: 70, inside: 50 }
const DETAIL = { exact: 40, prefix: 34, word: 30, inside: 24 }
const STATE = { exact: 36, prefix: 30, word: 26, inside: 20 }
const GROUP = { exact: 18, prefix: 14, word: 12, inside: 10 }

function merge(ranges: Range[]): Range[] {
  const sorted = [...ranges].sort((a, b) => a[0] - b[0])
  const out: Range[] = []
  for (const range of sorted) {
    const last = out[out.length - 1]
    if (last && range[0] <= last[1])
      out[out.length - 1] = [last[0], Math.max(last[1], range[1])]
    else out.push([range[0], range[1]])
  }
  return out
}

/**
 * Ranks every worker against the query. Each whitespace-separated token must
 * match somewhere — the name first (exact, prefix, a word start, anywhere,
 * then as a loose subsequence), else the state, the detail line or the
 * group — so `ready 0.2` finds every ready worker pinned at 0.2.x and
 * `pgc` still lands on provider-github-copilot. The result is ordered best
 * first, ties broken by name so the list never jumps between keystrokes.
 */
export function searchWorkers(
  groups: readonly ListGroup[],
  query: string,
): WorkerHit[] {
  const tokens = query.trim().toLowerCase().split(/\s+/).filter(Boolean)
  if (tokens.length === 0) return []
  const hits: WorkerHit[] = []
  for (const group of groups) {
    for (const item of group.items) {
      let score = 0
      const nameRanges: Range[] = []
      const detailRanges: Range[] = []
      let every = true
      for (const token of tokens) {
        const byName = matchField(item.name, token, NAME, true)
        if (byName) {
          score += byName.score
          nameRanges.push(...byName.ranges)
          continue
        }
        const byDetail = matchField(item.detail, token, DETAIL, false)
        if (byDetail) {
          score += byDetail.score
          detailRanges.push(...byDetail.ranges)
          continue
        }
        const byState = matchField(item.state, token, STATE, false)
        if (byState) {
          score += byState.score
          continue
        }
        const byGroup = matchField(group.label, token, GROUP, false)
        if (byGroup) {
          score += byGroup.score
          continue
        }
        every = false
        break
      }
      if (!every) continue
      // A failing worker is what the operator is most likely looking for.
      if (item.failed) score += 5
      hits.push({
        item,
        group: group.id,
        groupLabel: group.label,
        score,
        nameRanges: merge(nameRanges),
        detailRanges: merge(detailRanges),
      })
    }
  }
  return hits.sort(
    (a, b) => b.score - a.score || a.item.name.localeCompare(b.item.name),
  )
}

/**
 * Nothing matched: the closest worker name by edit distance, offered as a
 * "did you mean". Only names within a third of the query's length away.
 */
export function suggestWorker(
  groups: readonly ListGroup[],
  query: string,
): string | null {
  const needle = query.trim().toLowerCase()
  if (needle.length < 3) return null
  let best: { name: string; distance: number } | null = null
  for (const group of groups) {
    for (const { name } of group.items) {
      const distance = levenshtein(
        needle,
        name.toLowerCase().slice(0, needle.length + 2),
      )
      if (!best || distance < best.distance) best = { name, distance }
    }
  }
  return best && best.distance <= Math.max(1, Math.ceil(needle.length / 3))
    ? best.name
    : null
}

function levenshtein(a: string, b: string): number {
  const rows = a.length + 1
  const cols = b.length + 1
  let previous = Array.from({ length: cols }, (_, j) => j)
  for (let i = 1; i < rows; i++) {
    const current = [i]
    for (let j = 1; j < cols; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1
      current[j] = Math.min(
        (previous[j] ?? 0) + 1,
        (current[j - 1] ?? 0) + 1,
        (previous[j - 1] ?? 0) + cost,
      )
    }
    previous = current
  }
  return previous[cols - 1] ?? 0
}

/** `text` split into plain and highlighted pieces along `ranges`. */
export function splitRanges(
  text: string,
  ranges: readonly Range[],
): { text: string; hit: boolean }[] {
  if (ranges.length === 0) return [{ text, hit: false }]
  const out: { text: string; hit: boolean }[] = []
  let cursor = 0
  for (const [start, end] of ranges) {
    if (start > cursor)
      out.push({ text: text.slice(cursor, start), hit: false })
    out.push({ text: text.slice(start, end), hit: true })
    cursor = end
  }
  if (cursor < text.length) out.push({ text: text.slice(cursor), hit: false })
  return out
}
