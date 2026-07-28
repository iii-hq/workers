/**
 * Best-effort match highlighting for the snapshot a11y tree ([ref=eN]
 * handles). Ported from the console's sandbox `highlight.tsx`; the match
 * span carries the scoped `br-ui-hl` class instead of Tailwind utilities.
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
    if (text.length === 0) continue
    if (start > last) parts.push(line.slice(last, start))
    parts.push(
      <span key={`m:${n}`} className="br-ui-hl">
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
      <span key={`s:${n++}`} className="br-ui-hl">
        {line.slice(j, j + query.length)}
      </span>,
    )
    i = j + query.length
  }
  return parts
}
