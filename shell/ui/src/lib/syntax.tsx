/**
 * Line-at-a-time syntax coloring for the diff pane — a lightweight
 * tokenizer, not a grammar: strings, comments, numbers, and per-family
 * keywords, everything else plain. Multiline constructs (block comments,
 * template strings) color best-effort per line; fidelity belongs to the
 * Monaco buffer one click away.
 */
import type { ReactNode } from 'react'

export type TokenType = 'str' | 'com' | 'num' | 'kw' | 'plain'

export interface Token {
  t: TokenType
  s: string
}

interface LangDef {
  /** Line-comment starters, longest first. */
  comments: string[]
  keywords: Set<string>
}

const C_KEYWORDS = [
  'as', 'async', 'await', 'break', 'case', 'catch', 'class', 'const', 'continue', 'default',
  'do', 'else', 'enum', 'export', 'extends', 'false', 'finally', 'for', 'from', 'function',
  'if', 'impl', 'import', 'in', 'interface', 'let', 'loop', 'match', 'mod', 'mut', 'new',
  'null', 'of', 'pub', 'return', 'self', 'static', 'struct', 'super', 'switch', 'this',
  'throw', 'trait', 'true', 'try', 'type', 'typeof', 'undefined', 'use', 'var', 'void',
  'while', 'yield',
]

const PY_KEYWORDS = [
  'and', 'as', 'assert', 'async', 'await', 'break', 'class', 'continue', 'def', 'del',
  'elif', 'else', 'except', 'False', 'finally', 'for', 'from', 'global', 'if', 'import',
  'in', 'is', 'lambda', 'None', 'not', 'or', 'pass', 'raise', 'return', 'True', 'try',
  'while', 'with', 'yield',
]

const SH_KEYWORDS = [
  'case', 'do', 'done', 'elif', 'else', 'esac', 'export', 'fi', 'for', 'function', 'if',
  'in', 'local', 'return', 'then', 'while',
]

const LANGS: Record<string, LangDef> = {
  c: { comments: ['//'], keywords: new Set(C_KEYWORDS) },
  python: { comments: ['#'], keywords: new Set(PY_KEYWORDS) },
  shell: { comments: ['#'], keywords: new Set(SH_KEYWORDS) },
  yaml: { comments: ['#'], keywords: new Set(['true', 'false', 'null']) },
  css: { comments: [], keywords: new Set() },
  html: { comments: [], keywords: new Set() },
  plain: { comments: [], keywords: new Set() },
}

const EXT_LANG: Record<string, string> = {
  js: 'c', jsx: 'c', ts: 'c', tsx: 'c', mjs: 'c', cjs: 'c', rs: 'c', go: 'c', java: 'c',
  c: 'c', h: 'c', cc: 'c', cpp: 'c', hpp: 'c', cs: 'c', swift: 'c', kt: 'c', json: 'c',
  py: 'python',
  sh: 'shell', bash: 'shell', zsh: 'shell',
  yml: 'yaml', yaml: 'yaml', toml: 'yaml',
  css: 'css', scss: 'css', less: 'css',
  html: 'html', htm: 'html', xml: 'html', svg: 'html', vue: 'html', md: 'plain',
}

export function langFromPath(path: string): string {
  const dot = path.lastIndexOf('.')
  if (dot === -1) return 'plain'
  return EXT_LANG[path.slice(dot + 1).toLowerCase()] ?? 'plain'
}

const WORD = /[A-Za-z_$][A-Za-z0-9_$]*|\d[\w.]*/y

export function tokenizeLine(line: string, lang: string): Token[] {
  const def = LANGS[lang] ?? LANGS.plain
  const tokens: Token[] = []
  let plain = ''
  const flush = () => {
    if (plain !== '') {
      tokens.push({ t: 'plain', s: plain })
      plain = ''
    }
  }

  let i = 0
  scan: while (i < line.length) {
    const ch = line[i]

    for (const c of def.comments) {
      if (line.startsWith(c, i)) {
        flush()
        tokens.push({ t: 'com', s: line.slice(i) })
        break scan
      }
    }
    if (ch === '/' && line.startsWith('/*', i)) {
      const end = line.indexOf('*/', i + 2)
      flush()
      const stop = end === -1 ? line.length : end + 2
      tokens.push({ t: 'com', s: line.slice(i, stop) })
      i = stop
      continue
    }

    if (ch === '"' || ch === "'" || ch === '`') {
      let j = i + 1
      while (j < line.length && line[j] !== ch) {
        if (line[j] === '\\') j++
        j++
      }
      const stop = Math.min(j + 1, line.length)
      flush()
      tokens.push({ t: 'str', s: line.slice(i, stop) })
      i = stop
      continue
    }

    WORD.lastIndex = i
    const m = WORD.exec(line)
    if (m !== null && m.index === i) {
      const word = m[0]
      const t: TokenType = /^\d/.test(word) ? 'num' : def.keywords.has(word) ? 'kw' : 'plain'
      if (t === 'plain') {
        plain += word
      } else {
        flush()
        tokens.push({ t, s: word })
      }
      i += word.length
      continue
    }

    plain += ch
    i++
  }
  flush()
  return tokens
}

function renderTokens(tokens: Token[], keyBase: string): ReactNode[] {
  return tokens.map((tok, i) =>
    tok.t === 'plain' ? (
      tok.s
    ) : (
      <span key={`${keyBase}:${String(i)}`} className={`shui-tok-${tok.t}`}>
        {tok.s}
      </span>
    ),
  )
}

/** Syntax-colored line, optionally with the changed char span `[from,
    to)` wrapped in the intraline emphasis background. Each segment
    tokenizes independently — best-effort at the boundary. */
export function renderCodeLine(
  line: string,
  lang: string,
  hl?: [number, number],
): ReactNode {
  if (hl === undefined) return renderTokens(tokenizeLine(line, lang), 'w')
  const [from, to] = hl
  return (
    <>
      {renderTokens(tokenizeLine(line.slice(0, from), lang), 'a')}
      <span className="shui-dl-emph">{renderTokens(tokenizeLine(line.slice(from, to), lang), 'b')}</span>
      {renderTokens(tokenizeLine(line.slice(to), lang), 'c')}
    </>
  )
}
