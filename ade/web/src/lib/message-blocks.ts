/**
 * Cutting worker-written blocks out of a user message: a `<tag attr="…">`
 * line, its body and a `</tag>` line, standing as their own paragraph (a
 * workflow node's `<workflow_input>`, `build_opening` in
 * workflow/src/functions/tick.rs). The chat renders each block through the
 * renderer its worker registered (`host.chat.registerMessageBlockRenderer`).
 */

/** A tag a worker may register. */
export const MESSAGE_BLOCK_TAG = /^[a-z][a-z0-9_-]*$/

export interface MessageBlock {
  tag: string
  /** The opening tag's `name="value"` pairs, values as written. */
  attributes: Readonly<Record<string, string>>
  /** The lines between the tags. */
  body: string
}

export interface SplitMessage {
  /** The text without its blocks (and the blank line after each), trimmed;
   *  the text as given when it has no block. */
  prose: string
  blocks: MessageBlock[]
}

// The whole opening line: the tag, then only `name="value"` attributes.
const OPEN = /^<([a-z][a-z0-9_-]*)((?:\s+[\w:.-]+="[^"]*")*)\s*>$/
const ATTRIBUTE = /([\w:.-]+)="([^"]*)"/g
// CommonMark: a backtick fence's info string has no backtick, so ```js``` is inline code.
const FENCE = /^ {0,3}(`{3,}(?=[^`]*$)|~{3,})/

const blank = (line: string | undefined) =>
  line === undefined || line.trim() === ''

/** The line closing the block opened at `from`, or -1. */
function closingLine(lines: string[], from: number, tag: string): number {
  for (let j = from + 1; j < lines.length; j++) {
    if (lines[j] === `</${tag}>` && blank(lines[j + 1])) return j
  }
  return -1
}

/**
 * Every block of a tag in `tags`, in order, and the prose around them. A
 * block opens at the text's start or after a blank line and closes before
 * one or at the end; inside fenced code (or not on lines of its own, as in
 * inline code) a tag stays text.
 */
export function splitMessageBlocks(
  text: string,
  tags: { has(tag: string): boolean },
): SplitMessage {
  if (!text.includes('</')) return { prose: text, blocks: [] }
  const lines = text.split(/\r?\n/)
  const kept: string[] = []
  const blocks: MessageBlock[] = []
  let fence = ''
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    const marker = FENCE.exec(line)?.[1]
    if (fence) {
      // A fence closes on a bare run of its own character, at least as long.
      if (
        marker?.[0] === fence[0] &&
        marker.length >= fence.length &&
        line.trim() === marker
      )
        fence = ''
      kept.push(line)
      continue
    }
    if (marker) {
      fence = marker
      kept.push(line)
      continue
    }
    const open = OPEN.exec(line)
    const end =
      open && tags.has(open[1]) && blank(lines[i - 1])
        ? closingLine(lines, i, open[1])
        : -1
    if (!open || end < 0) {
      kept.push(line)
      continue
    }
    blocks.push({
      tag: open[1],
      attributes: Object.fromEntries(
        [...open[2].matchAll(ATTRIBUTE)].map(([, name, value]) => [
          name,
          value,
        ]),
      ),
      body: lines.slice(i + 1, end).join('\n'),
    })
    // The blank line after the block goes with it (closingLine guarantees one or the end).
    i = end + 1
  }
  if (blocks.length === 0) return { prose: text, blocks }
  return {
    // No blank-line collapse: it would rewrite fenced code in the prose.
    prose: kept.join('\n').trim(),
    blocks,
  }
}
