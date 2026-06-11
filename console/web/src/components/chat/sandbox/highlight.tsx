/**
 * Best-effort match highlighting for grep-style views — shared by coder
 * SearchView and sandbox FsGrepView (grep patterns are always regexes,
 * so FsGrepView passes `isRegex: true`). The daemons use the Rust
 * `regex` crate; JS regex covers the simple patterns agents use and we
 * fall back to substring matching when the pattern doesn't compile — or
 * when `isRegex` is false, since the query is then a literal and regex
 * metacharacters must not fire.
 */
import type { ReactNode } from 'react'

export interface HighlightOptions {
  /** Treat `query` as a regex; false = literal substring match. */
  isRegex: boolean
  ignoreCase: boolean
}

export function renderWithHighlight(
  line: string,
  query: string,
  { isRegex, ignoreCase }: HighlightOptions,
): ReactNode {
  if (!query) return line
  if (isRegex) {
    let re: RegExp | null = null
    try {
      re = new RegExp(query, ignoreCase ? 'gi' : 'g')
    } catch {
      re = null
    }
    if (re) return highlightRegex(line, re)
  }
  return highlightSubstring(line, query, ignoreCase)
}

function highlightRegex(line: string, re: RegExp): ReactNode {
  const parts: ReactNode[] = []
  let last = 0
  let n = 0
  for (const hit of line.matchAll(re)) {
    const start = hit.index ?? 0
    const text = hit[0]
    // Skip zero-width matches that would otherwise loop forever.
    if (text.length === 0) continue
    if (start > last) parts.push(line.slice(last, start))
    parts.push(
      <span key={`m:${n}`} className="bg-accent/15 text-accent">
        {text}
      </span>,
    )
    last = start + text.length
    n++
    if (n > 200) break
  }
  if (last < line.length) parts.push(line.slice(last))
  return parts
}

// Substring path: literal queries and patterns the JS regex engine rejects.
function highlightSubstring(
  line: string,
  query: string,
  ignoreCase: boolean,
): ReactNode {
  const needle = ignoreCase ? query.toLowerCase() : query
  const hay = ignoreCase ? line.toLowerCase() : line
  const parts: ReactNode[] = []
  let i = 0
  let n = 0
  while (i < line.length) {
    const j = hay.indexOf(needle, i)
    if (j === -1) {
      parts.push(line.slice(i))
      break
    }
    if (j > i) parts.push(line.slice(i, j))
    parts.push(
      <span key={`s:${n++}`} className="bg-accent/15 text-accent">
        {line.slice(j, j + query.length)}
      </span>,
    )
    i = j + query.length
  }
  return parts
}
