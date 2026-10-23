/**
 * Ticket keys inside Markdown become links to that ticket (design §6.2): the
 * console UI's ui/src/links.ts, unchanged but for the href, which the app's
 * `onLink` reads back (`#kanban-ticket/KAN-12`). Its patterns are fixed, never
 * built from what a person typed beyond the escaped ticket keys.
 */

export const TICKET_HREF = '#kanban-ticket/'

/** A ticket the open one is linked to; `status` is the column label. */
export type LinkedTicket = { key: string; title: string; status: string }

// What makes a neighbour part of a word, as the Rust scanner (`cited_keys`) sees it:
// `char::is_alphanumeric` (= \p{Alphabetic} + \p{N}) and the five combining-mark blocks it lists.
// `/` and `-` are added where the rule is applied.
const WORD = String.raw`\p{Alphabetic}\p{N}\u0300-\u036F\u1AB0-\u1AFF\u1DC0-\u1DFF\u20D0-\u20FF\uFE20-\uFE2F`
// The key is no mention when the character before it is part of a word, makes an image (`!`) or escapes the bracket (`\`).
// The `-` goes last, unescaped: QuickJS refuses `\-` in a class under the `u` flag (a SyntaxError at load).
const BEFORE = new RegExp(`[${WORD}/!\\\\-]$`, 'u')
// Emphasis markers. A run of one that touches a key must stand against a space or the edge of the line: the link
// puts `[` and `)` there, and a run beside punctuation opens and closes differently from one beside a letter
// (`日**KAN-1**を` is emphasis, `日**[KAN-1](…)**を` is not).
const isMark = (code: number) => code === 42 || code === 95 || code === 126 // `*` `_` `~`
const isEdge = (code: number) => Number.isNaN(code) || code === 32 || code === 9 || code === 13 // or past either end
function crowded(line: string, at: number, end: number): boolean {
  const before = line.charCodeAt(at - 1)
  if (isMark(before)) {
    let start = at - 1
    while (line.charCodeAt(start - 1) === before) start--
    if (!isEdge(line.charCodeAt(start - 1))) return true
  }
  const after = line.charCodeAt(end)
  if (isMark(after)) {
    let stop = end
    while (line.charCodeAt(stop + 1) === after) stop++
    if (!isEdge(line.charCodeAt(stop + 1))) return true
  }
  return false
}

// What can stand in front of a line's own text: blockquote markers, list markers, indentation.
const LEAD = String.raw`[ \t>*+\-\d.)]*`
// A text with a link definition (`[kan-1]: url`) is left alone whole: a key anywhere can be its label, a reference
// to it, or its title, and the parser decides which.
const DEFINITION = new RegExp(String.raw`^${LEAD}\[[^\]\n]+\]:`, 'm')
// A bare CR ends a line for the parser, not for `split('\n')`.
const BARE_CR = /\r(?!\n)/
const BLANK = /^[ \t]*\r?$/
// A fence opens at the start of a line (group 1 empty) or after a prefix (a list item, a quote); a backtick
// fence has no backtick in its info string, and the run is maximal so the lookahead scans the line once.
const FENCE = new RegExp(String.raw`^(${LEAD})(\x60{3,}(?!\x60)(?![^\n]*\x60)|~{3,})`)
const CLOSER = /^ {0,3}(`{3,}|~{3,})[ \t]*\r?$/
// HTML that ends at a marker, not at a blank line: comments, scripts, declarations.
const RAW_HTML = new RegExp(`^${LEAD}<(?:[!?]|script|pre|style|textarea)`, 'i')
// Characters that can open something spanning lines: a code span, a link or its label, a tag.
const HOT = /[`<[\]]/
const HTML_BLOCK = /^ {0,3}</
// Lines that begin with anything else cannot be blank, a fence or HTML: the common case skips those tests.
const SPECIAL = /^(?:[ \t>*+\-\d.)`~<\r]|$)/
// Indented code (also right after a list marker), a quote, a backslash escape, or an address (`www.`, a bare
// URL, an email, `@name`: GFM autolinks and mentions).
const NOT_PROSE = new RegExp(String.raw`^${LEAD}(?:>| {4}|\t)|\\|@|https?:\/\/|www\.`, 'i')

/** Only ASCII: the `iu` flags also fold U+212A to `k` and U+017F to `s`, which the Rust scanner does not. */
const isAscii = (text: string) => {
  for (let i = 0; i < text.length; i++) if (text.charCodeAt(i) > 127) return false
  return true
}

const escapeRe = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
// A title must not end its quotes or the link: whitespace (a blank line ends the paragraph) collapses
// to one space and ASCII punctuation is backslash-escaped (`"`, `\`, `&` entities, `|` in GFM tables,
// parentheses; `;` stays, it only ends an entity and `&` is escaped). A backtick becomes an entity: a bare
// one would pair with the code spans around it.
const inert = (text: string) =>
  text.replace(/\s+/g, ' ').replace(/[!-/:<-@[-`{-~]/g, (char) => (char === '`' ? '&#96;' : `\\${char}`))

/**
 * Replace every case-insensitive key of `tickets` with
 * `[KEY](#kanban-ticket/KEY "Title · Status")`, in plain prose. A key is a
 * mention unless a word character (see `WORD`), `/` or `-` touches it (same
 * rule as the Rust scanner, so `_KAN-1_` is one). Rewriting its own output
 * changes nothing. Linear in the text: lines are tested a few times each and
 * no pattern rescans what an unterminated bracket or fence leaves behind.
 * ponytail: conservative on purpose, a wrong link corrupts text and a missing
 * one costs nothing (the "Linked tickets" list has every key). Not linked:
 * - a paragraph (the lines up to a blank one) with a backtick, `<`, `[` or `]`;
 * - a line with a backslash, `@`, `www.` or a URL, with a `>` before its text,
 *   or indented by four spaces or a tab;
 * - fenced code, and everything after a fence in a list or quote, a fence
 *   never closed, or a comment/script block: the parser knows where those end;
 * - the whole text when it has a link definition.
 * Parse the Markdown instead if text with code, links or lists must link too.
 */
export function linkTicketKeys(markdown: string, tickets: readonly LinkedTicket[]): string {
  // Only keys that are inert inside `[…](…)` (word characters, `.`, `+`, `-`); a prefix with others is not linked.
  const byKey = new Map(
    tickets.filter(({ key }) => /^[\w.+-]+$/.test(key)).map((ticket) => [ticket.key.toUpperCase(), ticket] as const),
  )
  if (byKey.size === 0 || BARE_CR.test(markdown) || DEFINITION.test(markdown)) return markdown
  const tails = new Map(
    [...byKey].map(([key, ticket]) => [
      key,
      `](${TICKET_HREF}${ticket.key} "${inert(`${ticket.title} · ${ticket.status}`)}")`,
    ]),
  )
  const keys = [...byKey.keys()].map(escapeRe).join('|')
  // No lookbehind (older Safari): the characters around a key are tested by offset. The `iu` flags also
  // fold U+212A to `k` and U+017F to `s`; the scanner is ASCII-only, so those matches are dropped.
  const re = new RegExp(`(?:${keys})(?![${WORD}/-])`, 'giu')
  if (markdown.search(re) < 0) return markdown // most texts name no ticket

  const rewrite = (line: string): string =>
    NOT_PROSE.test(line)
      ? line
      : line.replace(re, (key, at: number) =>
          !isAscii(key) || BEFORE.test(line.slice(Math.max(0, at - 8), at)) || crowded(line, at, at + key.length)
            ? key
            : `[${key}${tails.get(key.toUpperCase())}`,
        )

  const out: string[] = []
  let chunk: string[] = [] // the lines since the last blank one: what a code span, link or tag can span
  let hot = false // the chunk has a character that can open one
  let html = false // the chunk starts an HTML block, which swallows a fence line
  let fence = '' // the open fence, when it started at the beginning of a line
  let dead = false // from here on the parser, not this function, knows where code is
  const flush = () => {
    for (const line of chunk) out.push(hot ? line : rewrite(line))
    chunk = []
    hot = html = false
  }
  for (const line of markdown.split('\n')) {
    if (dead) {
      out.push(line)
    } else if (fence) {
      const close = CLOSER.exec(line)
      if (close && close[1][0] === fence[0] && close[1].length >= fence.length) fence = ''
      out.push(line)
    } else if (!SPECIAL.test(line)) {
      chunk.push(line)
      hot ||= HOT.test(line)
    } else if (BLANK.test(line)) {
      flush()
      out.push(line)
    } else {
      const open = FENCE.exec(line)
      if (open && !open[1] && !html) {
        flush()
        fence = open[2]
        out.push(line)
      } else if (open || RAW_HTML.test(line)) {
        flush()
        dead = true
        out.push(line)
      } else {
        chunk.push(line)
        hot ||= HOT.test(line)
        html ||= HTML_BLOCK.test(line)
      }
    }
  }
  flush()
  return out.join('\n')
}
