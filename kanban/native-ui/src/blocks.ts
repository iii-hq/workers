/**
 * Long Markdown in pieces the host can draw one at a time (design §6.2):
 * the app's Markdown costs the host 150–550 ms of main thread for 20 KB the
 * first time it draws it, so a long description becomes several List items
 * (the LazyColumn composes the visible ones) and a long comment shows its
 * start until "Show more". The cuts are at blank lines between top-level
 * blocks: never inside a fence, a list (with its loose items and indented
 * continuations) or indented code; a table holds no blank line, so it stays
 * whole too. A block too big for one piece (a long checklist, a wall of text
 * with no blank line) is cut at a line: before a top-level item of a list,
 * anywhere in a paragraph or a quote; never in a fence, a table or HTML. The
 * pieces are slices of the text: joined, they are the text. Pure: no `iii`.
 */

/** A description longer than this is drawn in pieces. */
export const SPLIT_AT = 2048
/** The size a piece grows to by merging neighbouring blocks (a block bigger than it stays whole). */
export const PIECE = 1536
/** A comment longer than this is folded… */
export const FOLD_AT = 3072
/** …to its first blocks, about this long. */
export const FOLD_TO = 1536

const FENCE = /^\s*(`{3,}|~{3,})/
const CLOSER = /^\s*(`{3,}|~{3,})\s*$/
const LIST_ITEM = /^ {0,3}(?:[-*+]|\d{1,9}[.)])(?:[ \t]|$)/
/** A list item's continuation after a blank line. */
const INDENTED = /^(?: {2,}|\t)/
const CODE = /^(?: {4,}|\t)/
const HEADING = /^ {0,3}#{1,6}(?:[ \t]|$)/
/** A link reference definition: a reference in one piece would lose its definition in another. */
const DEFINITION = /^ {0,3}\[[^\]\n]+\]:/m
const BLANK = /^\s*$/
/** The delimiter row under a table's header. */
const TABLE_RULE = /^\s*\|?\s*:?-+:?\s*(?:\|\s*:?-+:?\s*)*\|?\s*$/
const HTML = /^ {0,3}</
const TABLE_ROW = /^\s*\|/

/**
 * The text's top-level blocks, as contiguous slices: each starts at a line
 * that follows a blank one, outside a fence, and is not the continuation of
 * a list or of indented code.
 */
export function chunks(text: string): string[] {
  const starts = [0]
  let fence: { char: string; length: number } | null = null
  let blankBefore = false
  let kind: 'list' | 'code' | 'other' = 'other'
  let at = 0
  for (const line of text.split('\n')) {
    const offset = at
    at += line.length + 1
    if (fence) {
      const close = CLOSER.exec(line)
      if (close && close[1][0] === fence.char && close[1].length >= fence.length) fence = null
      blankBefore = false
      continue
    }
    if (BLANK.test(line)) {
      blankBefore = true
      continue
    }
    const continues =
      (kind === 'list' && (LIST_ITEM.test(line) || INDENTED.test(line))) || (kind === 'code' && CODE.test(line))
    if (blankBefore && offset > 0 && !continues) {
      starts.push(offset)
      kind = LIST_ITEM.test(line) ? 'list' : CODE.test(line) ? 'code' : 'other'
    } else if (offset === 0) {
      kind = LIST_ITEM.test(line) ? 'list' : CODE.test(line) ? 'code' : 'other'
    }
    blankBefore = false
    const open = FENCE.exec(line)
    if (open) fence = { char: open[1][0], length: open[1].length }
  }
  return starts.map((start, i) => text.slice(start, starts[i + 1] ?? text.length))
}

/**
 * A block cut at line starts into parts of about `size` (design above):
 * before a top-level item in a list, at any line in a paragraph or a quote,
 * never inside a fence; a table, HTML or a fenced block stays whole.
 */
export function lines(block: string, size: number): string[] {
  const first = block.split('\n', 3)
  if (
    block.length <= size ||
    HTML.test(first[0] ?? '') ||
    FENCE.test(first[0] ?? '') ||
    TABLE_RULE.test(first[1] ?? '')
  )
    return [block]
  const list = LIST_ITEM.test(first[0] ?? '')
  const starts = [0]
  let fence: { char: string; length: number } | null = null
  let at = 0
  for (const line of block.split('\n')) {
    const offset = at
    at += line.length + 1
    if (fence) {
      const close = CLOSER.exec(line)
      if (close && close[1][0] === fence.char && close[1].length >= fence.length) fence = null
      continue
    }
    const open = FENCE.exec(line)
    // Never before a table row: a table cut in two loses its header in the second part.
    const cut =
      offset - starts[starts.length - 1] >= size &&
      !BLANK.test(line) &&
      !TABLE_ROW.test(line) &&
      (!list || LIST_ITEM.test(line))
    if (cut) starts.push(offset)
    if (open) fence = { char: open[1][0], length: open[1].length }
  }
  return starts.map((start, i) => block.slice(start, starts[i + 1] ?? block.length))
}

/**
 * A description as the List's pieces: the whole text when it is short (or
 * has link definitions), else its blocks merged up to about `PIECE`, a
 * heading never left at the end of a piece away from what it heads.
 */
export function pieces(text: string, splitAt = SPLIT_AT, size = PIECE): string[] {
  if (text.length <= splitAt || DEFINITION.test(text)) return [text]
  const groups: string[][] = []
  let length = 0
  for (const chunk of chunks(text).flatMap((block) => lines(block, size))) {
    const group = groups[groups.length - 1]
    if (!group || length + chunk.length > size) {
      const carried = group && group.length > 1 && HEADING.test(group[group.length - 1]) ? [group.pop() as string] : []
      groups.push([...carried, chunk])
      length = carried.reduce((n, c) => n + c.length, chunk.length)
    } else {
      group.push(chunk)
      length += chunk.length
    }
  }
  return groups.map((group) => group.join(''))
}

/** The start of a long comment, cut after a block, about `FOLD_TO` long; null when it is short enough to show whole. */
export function folded(text: string): string | null {
  if (text.length <= FOLD_AT || DEFINITION.test(text)) return null
  let cut = ''
  for (const chunk of chunks(text).flatMap((block) => lines(block, FOLD_TO))) {
    if (cut && cut.length + chunk.length > FOLD_TO) break
    cut += chunk
  }
  return cut.length < text.length ? cut.replace(/\s+$/, '') : null
}
