/**
 * The `#file(<inner>)` token the composer pill, the transcript renderer and
 * the send path all share. `<inner>` is a path (working-directory-relative,
 * or absolute when the file lives outside the chat's folder) optionally
 * followed by a line window:
 *
 *   #file(src/a.ts)          the whole file
 *   #file(src/a.ts:12)       one line
 *   #file(src/a.ts:12-40)    an inclusive range
 *   #file(src/)              a folder (never takes a window)
 *
 * The window is what "Reference in chat" from a code selection produces; at
 * send time it turns into a windowed `coder::read-file`, so the model sees
 * only the lines the person pointed at.
 */

export interface LineRange {
  /** 1-based, inclusive. */
  from: number
  /** 1-based, inclusive; never below `from`. */
  to: number
}

export interface FileMentionRef {
  path: string
  range?: LineRange
}

/** Path may contain spaces but not `)` — see file-search (paren paths dropped). */
export const FILE_MENTION_RE = /#file\(([^)]+)\)/g

const WINDOW_SUFFIX_RE = /:(\d+)(?:-(\d+))?$/

/** Order the ends so `to` is never below `from`. */
export function normalizeLineRange(a: number, b: number): LineRange {
  return a <= b ? { from: a, to: b } : { from: b, to: a }
}

/** `12` for a single line, `12-40` for a range. */
export function formatLineRange(range: LineRange): string {
  return range.from === range.to ? `${range.from}` : `${range.from}-${range.to}`
}

/**
 * Split `path[:from[-to]]`. A folder (trailing `/`) or a malformed window
 * comes back as a plain path, the suffix left in place — a file that really
 * is named `notes:3` still mentions.
 */
export function parseFileMentionInner(inner: string): FileMentionRef {
  const trimmed = inner.trim()
  if (trimmed.endsWith('/')) return { path: trimmed }
  const match = trimmed.match(WINDOW_SUFFIX_RE)
  if (!match || match.index === undefined || match.index === 0) {
    return { path: trimmed }
  }
  const from = Number.parseInt(match[1], 10)
  const to = match[2] !== undefined ? Number.parseInt(match[2], 10) : from
  if (!(from >= 1) || !(to >= 1)) return { path: trimmed }
  return {
    path: trimmed.slice(0, match.index),
    range: normalizeLineRange(from, to),
  }
}

/** The text inside the parens: `src/a.ts:12-40`. */
export function formatFileMentionInner(ref: FileMentionRef): string {
  return ref.range ? `${ref.path}:${formatLineRange(ref.range)}` : ref.path
}

/** The whole token: `#file(src/a.ts:12-40)`. */
export function formatFileMention(ref: FileMentionRef): string {
  return `#file(${formatFileMentionInner(ref)})`
}

/** Two references to the same lines of the same file. */
export function sameFileMention(a: FileMentionRef, b: FileMentionRef): boolean {
  return formatFileMentionInner(a) === formatFileMentionInner(b)
}
