/* Pure shaping of `coder::search` results into the view the search tab
   renders: matches grouped by file, each line trimmed to a window around
   the hit, plus the flat row list a virtualized list walks. */

import type { ContentMatch, SearchResponse } from './coder'
import { basename, dirname } from './paths'

export interface SearchMatchRow {
  line: number
  column: number
  /** The whole (clipped) line the worker returned. */
  text: string
  /** Text before the match, already trimmed for display. */
  lead: string
  /** The matched text itself when it could be located, else ''. */
  hit: string
  /** Text after the match. */
  trail: string
  /** True when `lead` was cut on the left. */
  leadCut: boolean
}

export interface SearchFileGroup {
  /** Canonical absolute path from the worker. */
  path: string
  /** Root-relative path. */
  rel: string
  name: string
  dir: string
  matches: SearchMatchRow[]
}

export interface SearchMatchOptions {
  query: string
  regex: boolean
  ignoreCase: boolean
  wholeWord: boolean
}

/** Characters kept before the match so the eye lands on the hit. */
export const PREVIEW_LEAD_CHARS = 28
/** Characters kept after the match. */
export const PREVIEW_TRAIL_CHARS = 120

export function escapeRegex(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/** The pattern the worker should run for a query typed with the search
    toggles: literal queries are escaped, whole-word wraps in `\b`. The
    worker gets `regex: true` whenever the pattern was transformed. */
export function effectivePattern(options: SearchMatchOptions): { pattern: string; regex: boolean } {
  const base = options.regex ? options.query : escapeRegex(options.query)
  if (options.wholeWord) return { pattern: `\\b(?:${base})\\b`, regex: true }
  return { pattern: base, regex: options.regex }
}

function compile(options: SearchMatchOptions): RegExp | null {
  const { pattern } = effectivePattern(options)
  try {
    return new RegExp(pattern, options.ignoreCase ? 'i' : '')
  } catch {
    return null
  }
}

/** Locate the hit inside `text` from the worker's 1-based byte column,
    falling back to a JS regex when the column does not land on it (the
    column counts bytes, the line may hold multi-byte characters). */
export function locateHit(
  text: string,
  column: number,
  matcher: RegExp | null,
): { start: number; end: number } | null {
  const at = Math.max(0, column - 1)
  if (matcher) {
    if (at <= text.length) {
      matcher.lastIndex = 0
      const m = matcher.exec(text.slice(at))
      if (m && m.index === 0 && m[0].length > 0) return { start: at, end: at + m[0].length }
    }
    const m = matcher.exec(text)
    if (m && m[0].length > 0) return { start: m.index, end: m.index + m[0].length }
  }
  return null
}

export function previewRow(match: ContentMatch, matcher: RegExp | null): SearchMatchRow {
  const text = match.text
  const hit = locateHit(text, match.column, matcher)
  if (hit === null) {
    const trimmed = text.replace(/^\s+/, '')
    return {
      line: match.line,
      column: match.column,
      text,
      lead: trimmed.slice(0, PREVIEW_TRAIL_CHARS),
      hit: '',
      trail: '',
      leadCut: false,
    }
  }
  let leadStart = 0
  let leadCut = false
  const before = text.slice(0, hit.start)
  const beforeTrimmed = before.replace(/^\s+/, '')
  let lead = beforeTrimmed
  if (lead.length > PREVIEW_LEAD_CHARS) {
    leadStart = lead.length - PREVIEW_LEAD_CHARS
    lead = lead.slice(leadStart)
    leadCut = true
  }
  const trail = text.slice(hit.end, hit.end + PREVIEW_TRAIL_CHARS)
  return {
    line: match.line,
    column: match.column,
    text,
    lead,
    hit: text.slice(hit.start, hit.end),
    trail,
    leadCut,
  }
}

/** Absolute → root-relative for the worker's canonical paths. */
function relativeTo(root: string, abs: string): string {
  const prefix = root.endsWith('/') ? root : `${root}/`
  if (abs === root) return ''
  return abs.startsWith(prefix) ? abs.slice(prefix.length) : abs
}

export function groupContentMatches(
  matches: readonly ContentMatch[],
  root: string,
  options: SearchMatchOptions,
): SearchFileGroup[] {
  const matcher = compile(options)
  const groups = new Map<string, SearchFileGroup>()
  for (const match of matches) {
    let group = groups.get(match.path)
    if (group === undefined) {
      const rel = relativeTo(root, match.path)
      group = {
        path: match.path,
        rel,
        name: basename(rel),
        dir: dirname(rel),
        matches: [],
      }
      groups.set(match.path, group)
    }
    group.matches.push(previewRow(match, matcher))
  }
  return [...groups.values()]
}

export interface SearchPathRow {
  path: string
  rel: string
  name: string
  dir: string
  kind: 'file' | 'dir'
}

export function pathRows(response: SearchResponse, root: string): SearchPathRow[] {
  return response.path_matches.map((m) => {
    const rel = relativeTo(root, m.path)
    return {
      path: m.path,
      rel,
      name: basename(rel),
      dir: dirname(rel),
      kind: m.kind === 'dir' ? 'dir' : 'file',
    }
  })
}

export type SearchRow =
  | { type: 'file'; key: string; group: SearchFileGroup; collapsed: boolean }
  | { type: 'match'; key: string; group: SearchFileGroup; match: SearchMatchRow }
  | { type: 'path'; key: string; entry: SearchPathRow }
  | { type: 'section'; key: string; label: string; count: number }

/** The rows a virtual list renders: a section per kind, a header per
    file with its matches indented beneath unless collapsed. */
export function flattenSearchRows(
  groups: readonly SearchFileGroup[],
  paths: readonly SearchPathRow[],
  collapsed: ReadonlySet<string>,
): SearchRow[] {
  const rows: SearchRow[] = []
  if (paths.length > 0) {
    rows.push({ type: 'section', key: 'section:paths', label: 'Files and folders', count: paths.length })
    for (const entry of paths) rows.push({ type: 'path', key: `path:${entry.path}`, entry })
  }
  if (groups.length > 0 && paths.length > 0) {
    const count = groups.reduce((total, group) => total + group.matches.length, 0)
    rows.push({ type: 'section', key: 'section:content', label: 'Text matches', count })
  }
  for (const group of groups) {
    const isCollapsed = collapsed.has(group.path)
    rows.push({ type: 'file', key: `file:${group.path}`, group, collapsed: isCollapsed })
    if (isCollapsed) continue
    for (const match of group.matches) {
      rows.push({
        type: 'match',
        key: `match:${group.path}:${match.line}:${match.column}`,
        group,
        match,
      })
    }
  }
  return rows
}

export function searchSummary(
  groups: readonly SearchFileGroup[],
  paths: readonly SearchPathRow[],
  truncated: boolean,
): string {
  const matches = groups.reduce((total, group) => total + group.matches.length, 0)
  const parts: string[] = []
  if (matches > 0) {
    parts.push(
      `${matches} ${matches === 1 ? 'result' : 'results'} in ${groups.length} ${groups.length === 1 ? 'file' : 'files'}`,
    )
  }
  if (paths.length > 0) parts.push(`${paths.length} matching ${paths.length === 1 ? 'name' : 'names'}`)
  if (parts.length === 0) return 'No results'
  const text = parts.join(' · ')
  return truncated ? `${text} (more available, refine the query)` : text
}

/** Move a roving focus through rows, skipping section labels and wrapping
    at neither end. */
export function stepSearchRow(rows: readonly SearchRow[], index: number, delta: 1 | -1): number {
  let next = index
  for (;;) {
    next += delta
    if (next < 0 || next >= rows.length) return index
    if (rows[next].type !== 'section') return next
  }
}
