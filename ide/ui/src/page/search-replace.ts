/* Replace for the Search view. The pure half shapes the text (the same
   pattern the search ran, so a replace never touches what the results did
   not show); the effectful half reads each file whole, rewrites it and
   writes it back against the revision it read, so an edit that landed in
   between is a conflict, not a silent overwrite. */

import type { Host } from '@iii-dev/console-ui'
import { coderReadFile, coderWriteFile } from './coder'
import { effectivePattern, type SearchMatchOptions } from './search-model'

export interface ReplaceOptions extends SearchMatchOptions {
  /** What each match becomes. In regex mode `$1`/`$<name>` refer to groups. */
  replacement: string
  /** Keep the match's casing: `foo`→`bar`, `Foo`→`Bar`, `FOO`→`BAR`. */
  preserveCase: boolean
}

/** The search pattern as a global, line-anchored JS regex; null when the
    query does not compile (an unfinished regex). */
export function replaceMatcher(options: SearchMatchOptions): RegExp | null {
  if (options.query === '') return null
  const { pattern } = effectivePattern(options)
  try {
    return new RegExp(pattern, `gm${options.ignoreCase ? 'i' : ''}`)
  } catch {
    return null
  }
}

/** `replacement` cased like `matched`: all caps, all lower, or capitalized. */
export function applyCase(matched: string, replacement: string): string {
  const letters = matched.replace(/[^\p{L}]/gu, '')
  if (letters === '') return replacement
  if (letters === letters.toUpperCase() && letters !== letters.toLowerCase()) return replacement.toUpperCase()
  if (letters === letters.toLowerCase()) return replacement.toLowerCase()
  const first = letters[0]
  if (first === first.toUpperCase()) return replacement.charAt(0).toUpperCase() + replacement.slice(1)
  return replacement
}

/** `$&`, `$1`…`$99`, `$<name>` and `$$` in `template`, from one match. */
export function expandTemplate(template: string, match: readonly (string | undefined)[], groups?: Record<string, string | undefined>): string {
  return template.replace(/\$(\$|&|<([^>]*)>|(\d{1,2}))/g, (token, kind: string, name?: string, digits?: string) => {
    if (kind === '$') return '$'
    if (kind === '&') return match[0] ?? ''
    if (name !== undefined) return groups && name in groups ? (groups[name] ?? '') : token
    const index = Number(digits)
    if (index > 0 && index < match.length) return match[index] ?? ''
    // `$12` with one group means `$1` then a literal `2`.
    const first = Number(digits?.[0])
    if (digits && digits.length === 2 && first > 0 && first < match.length) return (match[first] ?? '') + digits[1]
    return token
  })
}

/** What one match turns into: group references expand in regex mode only;
    a literal replacement is taken as typed (a `$` stays a `$`). */
export function replacementFor(
  match: readonly (string | undefined)[],
  options: ReplaceOptions,
  groups?: Record<string, string | undefined>,
): string {
  const matched = match[0] ?? ''
  const out = options.regex ? expandTemplate(options.replacement, match, groups) : options.replacement
  return options.preserveCase ? applyCase(matched, out) : out
}

/** The preview of one hit in the results: the hit alone stands in for its
    line, which is close enough for group references to read right. */
export function previewReplacement(hit: string, options: ReplaceOptions): string {
  const matcher = replaceMatcher(options)
  if (matcher) {
    matcher.lastIndex = 0
    const m = matcher.exec(hit)
    if (m && m.index === 0) return replacementFor(m, options, m.groups)
  }
  return replacementFor([hit], options)
}

/** Every match in `content` replaced. Returns the new text and the count. */
export function replaceAllInText(content: string, options: ReplaceOptions): { content: string; count: number } {
  const matcher = replaceMatcher(options)
  if (!matcher) return { content, count: 0 }
  let count = 0
  let out = ''
  let last = 0
  for (let m = matcher.exec(content); m; m = matcher.exec(content)) {
    if (m[0] === '') {
      matcher.lastIndex += 1
      continue
    }
    count += 1
    out += content.slice(last, m.index) + replacementFor(m, options, m.groups)
    last = m.index + m[0].length
  }
  return { content: out + content.slice(last), count }
}

/** Bytes of `text` in UTF-8: the worker's match columns count bytes. */
function utf8Length(text: string): number {
  return new TextEncoder().encode(text).length
}

/** The one match on 1-based `line` the worker reported at byte `column`
    replaced (the first on the line when the column does not land on one). */
export function replaceOneInText(
  content: string,
  line: number,
  column: number,
  options: ReplaceOptions,
): { content: string; count: number } {
  const matcher = replaceMatcher(options)
  if (!matcher || line < 1) return { content, count: 0 }
  // Walk to the line without losing the file's own line endings.
  let start = 0
  for (let index = 1; index < line; index += 1) {
    const newline = content.indexOf('\n', start)
    if (newline === -1) return { content, count: 0 }
    start = newline + 1
  }
  const newline = content.indexOf('\n', start)
  const end = newline === -1 ? content.length : newline
  const text = content.slice(start, end)
  let chosen: RegExpExecArray | null = null
  matcher.lastIndex = 0
  for (let m = matcher.exec(text); m; m = matcher.exec(text)) {
    if (m[0] === '') {
      matcher.lastIndex += 1
      continue
    }
    chosen ??= m
    if (utf8Length(text.slice(0, m.index)) + 1 === column) {
      chosen = m
      break
    }
  }
  if (!chosen) return { content, count: 0 }
  const at = start + chosen.index
  const replaced = replacementFor(chosen, options, chosen.groups)
  return { content: content.slice(0, at) + replaced + content.slice(at + chosen[0].length), count: 1 }
}

/** The worker's whole-file read budget for a replace: large enough for any
    source file, and the worker clamps it to its own ceiling anyway. */
const REPLACE_READ_BYTES = 8 * 1024 * 1024

export type FileReplaceOutcome =
  | { path: string; status: 'replaced'; count: number }
  | { path: string; status: 'unchanged' }
  | { path: string; status: 'skipped'; reason: string }

/** Read `absPath` whole, rewrite it with `edit`, and write it back against
    the revision read. A partial read or a binary file is left alone. */
export async function replaceInFile(
  host: Host,
  absPath: string,
  edit: (content: string) => { content: string; count: number },
): Promise<FileReplaceOutcome> {
  const read = await coderReadFile(host, absPath, { maxOutputBytes: REPLACE_READ_BYTES })
  if (read.is_utf8 === false) return { path: absPath, status: 'skipped', reason: 'not a text file' }
  if (read.more_lines === true || typeof read.content !== 'string') {
    return { path: absPath, status: 'skipped', reason: 'too large to rewrite' }
  }
  const { content, count } = edit(read.content)
  if (count === 0 || content === read.content) return { path: absPath, status: 'unchanged' }
  const written = await coderWriteFile(host, absPath, content, read.mode, read.revision)
  if (!written.success) {
    return { path: absPath, status: 'skipped', reason: written.error?.message ?? 'the write failed' }
  }
  return { path: absPath, status: 'replaced', count }
}

/** One line for the outcome of a replace across files. */
export function replaceSummary(outcomes: readonly FileReplaceOutcome[]): string {
  let count = 0
  let files = 0
  const skipped: string[] = []
  for (const outcome of outcomes) {
    if (outcome.status === 'replaced') {
      count += outcome.count
      files += 1
    } else if (outcome.status === 'skipped') skipped.push(outcome.reason)
  }
  const head =
    count === 0
      ? 'Nothing was replaced'
      : `Replaced ${count} ${count === 1 ? 'occurrence' : 'occurrences'} in ${files} ${files === 1 ? 'file' : 'files'}`
  return skipped.length === 0 ? head : `${head} · ${skipped.length} skipped (${skipped[0]})`
}
