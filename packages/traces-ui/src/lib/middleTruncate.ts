/**
 * Middle truncation, the way macOS Finder shortens a long file name: keep
 * the start and the end, drop the middle — `iii-directory::search_functions`
 * in a tight bar reads `iii-dir…search_functions`, so both the namespace
 * and the specific function stay recognisable.
 *
 * CSS has no middle ellipsis, so a label renders as two pieces
 * (`MiddleTruncate`): a HEAD that shrinks and ellipsizes at its end, and a
 * TAIL that keeps its width. This module picks where to cut. When the label
 * fits, the pieces sit flush and read as the whole name.
 */

/** Names this short just ellipsize at the end — there is no middle to drop. */
const MIN_SPLIT_LENGTH = 12
/** The tail never holds more than this many characters. */
const MAX_TAIL_CHARS = 18
/** A tail shorter than this is too little to recognise a name by. */
const MIN_TAIL_CHARS = 3
/** Without a separator to cut at, the tail keeps this share of the name. */
const FALLBACK_TAIL_SHARE = 0.4

/** Cut points in priority order: a namespace segment beats a path segment,
 *  which beats a dotted segment, which beats a word. */
const SEPARATORS = ['::', '/', '.', ' '] as const

export interface MiddleTruncateParts {
  head: string
  tail: string
}

/**
 * Split `text` into the head that may ellipsize and the tail that stays.
 * The tail is the last `::` (else `/`, `.`, space) segment when that
 * segment is a recognisable length, otherwise the last ~40% of the name
 * (capped). `head + tail === text` always holds.
 */
export function splitForMiddleTruncation(text: string): MiddleTruncateParts {
  if (text.length < MIN_SPLIT_LENGTH) return { head: text, tail: '' }

  for (const separator of SEPARATORS) {
    const at = text.lastIndexOf(separator)
    if (at <= 0) continue
    const cut = at + separator.length
    const tailLength = text.length - cut
    if (tailLength >= MIN_TAIL_CHARS && tailLength <= MAX_TAIL_CHARS) {
      return { head: text.slice(0, cut), tail: text.slice(cut) }
    }
  }

  const tailLength = Math.min(
    MAX_TAIL_CHARS,
    Math.max(MIN_TAIL_CHARS, Math.round(text.length * FALLBACK_TAIL_SHARE)),
  )
  const cut = text.length - tailLength
  return { head: text.slice(0, cut), tail: text.slice(cut) }
}
