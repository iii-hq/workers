/* Go to file (Ctrl+P): the pure half. Which key opens it on each platform,
   how a typed query is prepared for the worker, and which characters of a
   path the worker's fuzzy scorer matched — so a row can show WHY it is
   there. React-free so the tests and the editor pane import it without
   the dialog. */

import type { PageShortcut } from '@iii-dev/console-ui'
import { relativeTo } from './coder'
import { basename, dirname } from './paths'

export type QuickOpenPlatform = 'mac' | 'other'

/** `Ctrl+P` on a Mac, where ⌘P is the browser's Print and Ctrl is the
    free tier the console's own keys sit on. On Windows and Linux Ctrl IS
    the browser's modifier — `Ctrl+P` prints there and the console refuses
    it — so the chord moves to Alt, the free tier on those platforms. */
export const QUICK_OPEN_SHORTCUT: PageShortcut = { mac: ['Ctrl+P'], other: ['Alt+P'] }

export function quickOpenPlatform(): QuickOpenPlatform {
  if (typeof navigator === 'undefined') return 'other'
  return /Mac|iPhone|iPad|iPod/.test(navigator.userAgent) ? 'mac' : 'other'
}

/** How the key prints in a hint, for the reader's platform. */
export function quickOpenKeyLabel(platform: QuickOpenPlatform): string {
  return platform === 'mac' ? 'Ctrl+P' : 'Alt+P'
}

export interface KeyLike {
  key: string
  ctrlKey: boolean
  metaKey: boolean
  altKey: boolean
  shiftKey: boolean
}

/** Whether a keystroke is the quick-open chord. The editor body asks this
    in the capture phase: Monaco binds Ctrl+P to cursor-up on a Mac and
    would otherwise cancel the event before the console's dispatcher sees
    it. */
export function isQuickOpenKey(event: KeyLike, platform: QuickOpenPlatform): boolean {
  if (event.key.toLowerCase() !== 'p' || event.shiftKey || event.metaKey) return false
  return platform === 'mac' ? event.ctrlKey && !event.altKey : event.altKey && !event.ctrlKey
}

/** The query the worker scores: whitespace dropped (a space separates
    words for a person, but the scorer would look for a literal space in
    the path) and lowercased the way the scorer lowercases the path. */
export function normalizeQuickOpenQuery(query: string): string {
  return query.replace(/\s+/g, '').toLowerCase()
}

/** Lowercase per code point so indices stay aligned with the original
    string; a character whose lowercase form is longer keeps itself. */
function foldChars(text: string): string[] {
  return Array.from(text).map((char) => {
    const lower = char.toLowerCase()
    return Array.from(lower).length === 1 ? lower : char
  })
}

function indexOfSequence(hay: readonly string[], needle: readonly string[], from: number): number {
  if (needle.length === 0) return -1
  for (let start = from; start + needle.length <= hay.length; start += 1) {
    let matched = true
    for (let offset = 0; offset < needle.length; offset += 1) {
      if (hay[start + offset] !== needle[offset]) {
        matched = false
        break
      }
    }
    if (matched) return start
  }
  return -1
}

function range(start: number, length: number): number[] {
  return Array.from({ length }, (_, index) => start + index)
}

/** Which code points of `rel` the worker's scorer matched for `query`,
    following its tiers: the whole query inside the file name, then
    anywhere in the path, then the first subsequence in order. `null` when
    the query is not a subsequence at all. */
export function fuzzyMatchIndices(query: string, rel: string): number[] | null {
  const needle = Array.from(normalizeQuickOpenQuery(query))
  if (needle.length === 0) return []
  const hay = foldChars(rel)
  const slash = rel.lastIndexOf('/')
  const basenameStart = slash === -1 ? 0 : Array.from(rel.slice(0, slash + 1)).length

  const inName = indexOfSequence(hay, needle, basenameStart)
  if (inName !== -1) return range(inName, needle.length)
  const inPath = indexOfSequence(hay, needle, 0)
  if (inPath !== -1) return range(inPath, needle.length)

  const indices: number[] = []
  let qi = 0
  for (let i = 0; i < hay.length && qi < needle.length; i += 1) {
    if (hay[i] !== needle[qi]) continue
    indices.push(i)
    qi += 1
  }
  return qi === needle.length ? indices : null
}

export interface HighlightSegment {
  text: string
  hit: boolean
}

/** `text` cut into runs, `hit` where its code points (numbered from
    `offset` in the matched string) are in `indices`. Adjacent hits merge
    into one run. */
export function highlightSegments(
  text: string,
  indices: readonly number[] | null,
  offset = 0,
): HighlightSegment[] {
  const chars = Array.from(text)
  if (chars.length === 0) return []
  const hits = new Set(indices ?? [])
  const segments: HighlightSegment[] = []
  for (let i = 0; i < chars.length; i += 1) {
    const hit = hits.has(offset + i)
    const last = segments[segments.length - 1]
    if (last && last.hit === hit) last.text += chars[i]
    else segments.push({ text: chars[i], hit })
  }
  return segments
}

export interface QuickOpenRow {
  /** Root-relative path, the key and what opens. */
  rel: string
  name: string
  dir: string
  /** Matched code points of `rel`; `[]` for a listing, `null` when unknown. */
  indices: number[] | null
  /** Code-point offset of `name` within `rel`. */
  nameOffset: number
}

export function quickOpenRow(rel: string, query: string): QuickOpenRow {
  const name = basename(rel)
  const dir = dirname(rel)
  return {
    rel,
    name,
    dir,
    indices: fuzzyMatchIndices(query, rel),
    nameOffset: dir === '' ? 0 : Array.from(dir).length + 1,
  }
}

/** The worker's ranked path matches as rows: files only, root-relative,
    at most `limit`. Order is the worker's (best first). */
export function toQuickOpenRows(
  root: string,
  matches: readonly { path: string; kind?: 'file' | 'dir' }[],
  query: string,
  limit: number,
): QuickOpenRow[] {
  const rows: QuickOpenRow[] = []
  const seen = new Set<string>()
  for (const match of matches) {
    if (match.kind === 'dir') continue
    const rel = relativeTo(root, match.path)
    if (rel === '' || seen.has(rel)) continue
    seen.add(rel)
    rows.push(quickOpenRow(rel, query))
    if (rows.length >= limit) break
  }
  return rows
}

/** Step the active row with wrap-around; `-1` (nothing active) steps onto
    the first or the last row. */
export function stepQuickOpenIndex(current: number, delta: 1 | -1, count: number): number {
  if (count === 0) return -1
  if (current < 0) return delta === 1 ? 0 : count - 1
  return (current + delta + count) % count
}
