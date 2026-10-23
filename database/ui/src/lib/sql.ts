/**
 * A small SQL lexer: enough to colour one line of SQL and to split an editor
 * buffer into statements. It is not a parser — it only has to agree with the
 * database on where strings, quoted identifiers and comments begin and end,
 * because those are the places a `;` stops being a separator.
 *
 * Dialect differences that change token boundaries are honoured: `#` starts a
 * comment and `"…"` is a string on MySQL; `$tag$ … $tag$` dollar quoting and
 * `E'…'` backslash escapes are Postgres. Everything else is shared.
 */

export type SqlDialect = 'postgres' | 'mysql' | 'sqlite' | 'unknown'

export type SqlTokenKind =
  | 'keyword'
  | 'ident'
  | 'quoted'
  | 'string'
  | 'number'
  | 'comment'
  | 'operator'
  | 'punct'
  | 'param'
  | 'space'
  | 'text'

export interface SqlToken {
  kind: SqlTokenKind
  text: string
  start: number
  end: number
}

/** Highlighted as keywords. Functions (`count`, `now`) stay identifiers. */
const KEYWORDS = new Set(
  `add all alter analyze and any as asc begin between by case cast check collate column commit constraint
  create cross current_date current_time current_timestamp default delete desc describe distinct drop else
  end escape except exists explain false fetch filter first following for foreign from full grant group
  having if ilike in index inner insert intersect interval into is join key last lateral left like limit
  natural next not null nulls offset on only or order outer over partition preceding primary range
  recursive references replace returning revoke right rollback row rows select set show similar some table
  then to transaction true truncate unbounded union unique update using values view when where window
  with`
    .split(/\s+/)
    .filter(Boolean),
)

const OPERATORS = ['->>', '<=>', '->', '::', '<>', '<=', '>=', '!=', '||', '&&', '<<', '>>']
const SINGLE_OPERATORS = new Set(['=', '<', '>', '+', '-', '*', '/', '%', '~', '!', '|', '&', '^'])
const PUNCT = new Set(['(', ')', ',', ';', '.', '[', ']', '{', '}'])

const isIdentStart = (c: string) => /[A-Za-z_\u0080-￿]/.test(c)
const isIdentPart = (c: string) => /[A-Za-z0-9_$\u0080-￿]/.test(c)
const isDigit = (c: string) => c >= '0' && c <= '9'

/** End index (exclusive) of a quoted run starting at `i` (the opening quote). */
function quotedEnd(text: string, i: number, quote: string, backslash: boolean): number {
  let j = i + 1
  while (j < text.length) {
    const c = text[j]
    if (backslash && c === '\\') {
      j += 2
      continue
    }
    if (c === quote) {
      // A doubled quote is an escaped quote, not the end.
      if (text[j + 1] === quote) {
        j += 2
        continue
      }
      return j + 1
    }
    j++
  }
  return text.length
}

export function tokenizeSql(text: string, dialect: SqlDialect = 'unknown'): SqlToken[] {
  const tokens: SqlToken[] = []
  const push = (kind: SqlTokenKind, start: number, end: number) => {
    tokens.push({ kind, text: text.slice(start, end), start, end })
  }
  const mysql = dialect === 'mysql'
  let i = 0
  while (i < text.length) {
    const c = text[i]
    const next = text[i + 1] ?? ''

    if (/\s/.test(c)) {
      let j = i + 1
      while (j < text.length && /\s/.test(text[j])) j++
      push('space', i, j)
      i = j
      continue
    }

    if ((c === '-' && next === '-') || (c === '#' && mysql)) {
      let j = i
      while (j < text.length && text[j] !== '\n') j++
      push('comment', i, j)
      i = j
      continue
    }

    if (c === '/' && next === '*') {
      const close = text.indexOf('*/', i + 2)
      const j = close === -1 ? text.length : close + 2
      push('comment', i, j)
      i = j
      continue
    }

    // Prefixed strings: E'…' (backslash escapes), N'…', X'…', B'…'.
    if (/[eEnNxXbB]/.test(c) && next === "'") {
      const backslash = c === 'e' || c === 'E' || mysql
      const j = quotedEnd(text, i + 1, "'", backslash)
      push('string', i, j)
      i = j
      continue
    }

    if (c === "'") {
      const j = quotedEnd(text, i, "'", mysql)
      push('string', i, j)
      i = j
      continue
    }

    if (c === '"') {
      const j = quotedEnd(text, i, '"', mysql)
      push(mysql ? 'string' : 'quoted', i, j)
      i = j
      continue
    }

    if (c === '`') {
      const j = quotedEnd(text, i, '`', false)
      push('quoted', i, j)
      i = j
      continue
    }

    if (c === '$') {
      // `$1` is a positional parameter; `$tag$ … $tag$` a dollar-quoted string.
      if (isDigit(next)) {
        let j = i + 1
        while (j < text.length && isDigit(text[j])) j++
        push('param', i, j)
        i = j
        continue
      }
      const tag = /^\$([A-Za-z_][A-Za-z0-9_]*)?\$/.exec(text.slice(i))
      if (tag && !mysql) {
        const close = text.indexOf(tag[0], i + tag[0].length)
        const j = close === -1 ? text.length : close + tag[0].length
        push('string', i, j)
        i = j
        continue
      }
    }

    if (isDigit(c) || (c === '.' && isDigit(next))) {
      const m = /^(?:0[xX][0-9A-Fa-f]+|(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?)/.exec(text.slice(i))
      const j = i + (m ? m[0].length : 1)
      push('number', i, j)
      i = j
      continue
    }

    if (isIdentStart(c)) {
      let j = i + 1
      while (j < text.length && isIdentPart(text[j])) j++
      push(KEYWORDS.has(text.slice(i, j).toLowerCase()) ? 'keyword' : 'ident', i, j)
      i = j
      continue
    }

    if (c === '?' || ((c === ':' || c === '@') && isIdentStart(next))) {
      let j = i + 1
      while (j < text.length && isIdentPart(text[j])) j++
      push('param', i, j)
      i = j
      continue
    }

    const op = OPERATORS.find((o) => text.startsWith(o, i))
    if (op) {
      push('operator', i, i + op.length)
      i += op.length
      continue
    }
    if (SINGLE_OPERATORS.has(c)) {
      push('operator', i, i + 1)
      i++
      continue
    }
    if (PUNCT.has(c)) {
      push('punct', i, i + 1)
      i++
      continue
    }
    push('text', i, i + 1)
    i++
  }
  return tokens
}

export interface SqlStatement {
  /** The statement without its terminating `;`, trimmed. */
  text: string
  /** Offsets of `text` in the buffer. */
  start: number
  end: number
}

/**
 * Statements in a buffer, split on top-level `;`. A `;` inside a string,
 * a quoted identifier, a comment or a dollar-quoted body does not split.
 * Statements that hold only comments and whitespace are dropped — sending
 * one would be a round trip to learn that it is empty.
 */
export function splitStatements(text: string, dialect: SqlDialect = 'unknown'): SqlStatement[] {
  const out: SqlStatement[] = []
  let from = 0
  let meaningful = false
  const flush = (to: number) => {
    if (meaningful) {
      const raw = text.slice(from, to)
      const lead = raw.length - raw.trimStart().length
      const body = raw.trim()
      out.push({ text: body, start: from + lead, end: from + lead + body.length })
    }
    meaningful = false
  }
  for (const token of tokenizeSql(text, dialect)) {
    if (token.kind === 'punct' && token.text === ';') {
      flush(token.start)
      from = token.end
      continue
    }
    if (token.kind !== 'space' && token.kind !== 'comment') meaningful = true
  }
  flush(text.length)
  return out
}
