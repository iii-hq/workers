/**
 * The mention token: `@<name>(id="<id>")` — the TypeScript twin of
 * `crates/mention-contract/src/token.rs`. Both run the shared fixtures in
 * `crates/mention-contract/fixtures/tokens.json`.
 *
 * - `<name>`: lowercase ASCII letters, digits and `-`, starting with a
 *   letter, at most 40 characters, not `fn` / `file` / `skill`.
 * - `<id>`: a JSON string literal decoding to 1..512 characters.
 * - The `@` must not follow a letter, digit or `_`.
 * - Tolerated on read, never written: a missing closing quote,
 *   `@<name>(id="<id>)`, when the id holds no quote, backslash or `)` —
 *   models drop that quote often enough to matter.
 */

import type { MentionRef } from './types'

export const RESERVED_MENTION_NAMES: readonly string[] = ['fn', 'file', 'skill']
export const MAX_MENTION_NAME_LENGTH = 40
export const MAX_MENTION_ID_LENGTH = 512

const NAME_RE = /^[a-z][a-z0-9-]{0,39}$/

/* The candidate shape: group 2 is a JSON string body closed by `")`, group
   3 the tolerated unclosed id. `findMentions` then checks the name and
   decodes the id, so a reserved name or an over-long id is rejected the
   same way the Rust parser rejects it. */
const TOKEN_SOURCE = String.raw`(?<![\p{L}\p{N}_])@([a-z][a-z0-9-]*)\(id="(?:((?:[^"\\\u0000-\u001f]|\\["\\/bfnrt]|\\u[0-9a-fA-F]{4})*)"|([^"\\)\u0000-\u001f]+))\)`

/** Whether `name` can be a token name. */
export function isValidMentionName(name: string): boolean {
  return NAME_RE.test(name) && !RESERVED_MENTION_NAMES.includes(name)
}

/** Writes the token for one item. */
export function formatMention(name: string, id: string): string {
  return `@${name}(id=${JSON.stringify(id)})`
}

/** A mention found in text, with where it sits. */
export interface MentionMatch extends MentionRef {
  /** UTF-16 offset of the `@`. */
  index: number
}

function decodeId(body: string): string | null {
  try {
    const id: unknown = JSON.parse(`"${body}"`)
    if (typeof id !== 'string') return null
    const length = Array.from(id).length
    return length > 0 && length <= MAX_MENTION_ID_LENGTH ? id : null
  } catch {
    return null
  }
}

/** Every mention in `text`, in order, with offsets. */
export function findMentions(text: string): MentionMatch[] {
  if (!text.includes('(id="')) return []
  const out: MentionMatch[] = []
  for (const m of text.matchAll(new RegExp(TOKEN_SOURCE, 'gu'))) {
    const name = m[1]
    if (!isValidMentionName(name)) continue
    // The unclosed form holds no quote or backslash: its body is the id.
    const id = decodeId(m[2] ?? m[3])
    if (id === null) continue
    out.push({ name, id, token: m[0], index: m.index ?? 0 })
  }
  return out
}

/** Every mention in `text`, in order. */
export function parseMentions(text: string): MentionRef[] {
  return findMentions(text).map(({ name, id, token }) => ({ name, id, token }))
}

/**
 * Like `parseMentions`, but skips markdown code: anything between a run of
 * backticks and the next run of the same length (inline code and fenced
 * blocks alike). An unmatched run is text.
 */
export function parseProseMentions(text: string): MentionRef[] {
  const out: MentionRef[] = []
  for (const segment of proseSegments(text)) out.push(...parseMentions(segment))
  return out
}

function proseSegments(text: string): string[] {
  const segments: string[] = []
  let proseStart = 0
  let i = 0
  while (i < text.length) {
    if (text[i] !== '`') {
      i++
      continue
    }
    const runStart = i
    while (i < text.length && text[i] === '`') i++
    const closeEnd = findBacktickRun(text, i, i - runStart)
    if (closeEnd !== null) {
      segments.push(text.slice(proseStart, runStart))
      proseStart = closeEnd
      i = closeEnd
    }
  }
  segments.push(text.slice(proseStart))
  return segments
}

function findBacktickRun(
  text: string,
  from: number,
  length: number,
): number | null {
  let i = from
  while (i < text.length) {
    if (text[i] !== '`') {
      i++
      continue
    }
    const start = i
    while (i < text.length && text[i] === '`') i++
    if (i - start === length) return i
  }
  return null
}

/** `name` + `id`: the identity two tokens share however their ids were escaped. */
export function mentionKey(name: string, id: string): string {
  return `${name}\u0000${id}`
}

/** Mentions in order of first appearance, each item once. */
export function uniqueMentions(refs: readonly MentionRef[]): MentionRef[] {
  const seen = new Set<string>()
  const out: MentionRef[] = []
  for (const ref of refs) {
    const key = mentionKey(ref.name, ref.id)
    if (seen.has(key)) continue
    seen.add(key)
    out.push(ref)
  }
  return out
}
