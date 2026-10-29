import { Highlight, Prism, type PrismTheme } from 'prism-react-renderer'
import { cn } from '@/lib/utils'

/* prism-react-renderer bundles only a handful of languages. JSON isn't among
   them, so we register it directly on the shared Prism instance. The grammar
   below is the canonical one from prismjs/components/prism-json.js — small,
   well-tested, and inlined here to avoid pulling in `prismjs` as a peer dep.
   Guarded so HMR reloads don't double-register. */
type PrismLanguages = { languages: Record<string, unknown> }
const prism = Prism as unknown as PrismLanguages
if (!prism.languages.json) {
  prism.languages.json = {
    property: {
      pattern: /(^|[^\\])"(?:\\.|[^\\"\r\n])*"(?=\s*:)/,
      lookbehind: true,
      greedy: true,
    },
    string: {
      pattern: /(^|[^\\])"(?:\\.|[^\\"\r\n])*"(?!\s*:)/,
      lookbehind: true,
      greedy: true,
    },
    comment: {
      pattern: /\/\/.*|\/\*[\s\S]*?(?:\*\/|$)/,
      greedy: true,
    },
    number: /-?\b\d+(?:\.\d+)?(?:e[+-]?\d+)?\b/i,
    punctuation: /[{}[\],]/,
    operator: /:/,
    boolean: /\b(?:true|false)\b/,
    null: {
      pattern: /\bnull\b/,
      alias: 'keyword',
    },
  }
}

/* Token palette for read-only code. Every colour is a design token, so the
   palette flips with `[data-theme="dark"]`: the `syntax-*` tones (GitHub
   "prettylights", AA against `bg`) carry the token kinds, comments and
   punctuation recede to `ink-faint`, and diff lines reuse the `ok` /
   `alert-strong` status tones. prism-react-renderer merges a nested token's
   styles parent -> child -> alias with the last one winning, so `property`
   is deliberately unstyled: JSON keys keep the plain ink and aliased kinds
   such as Rust's `macro` (alias `property`) or TOML's `key` keep their own
   colour. */
export const syntaxTheme: PrismTheme = {
  plain: { color: 'var(--color-ink)' },
  styles: [
    {
      types: ['comment', 'prolog', 'doctype', 'cdata', 'shebang'],
      style: { color: 'var(--color-ink-faint)', fontStyle: 'italic' },
    },
    {
      types: ['punctuation', 'operator'],
      style: { color: 'var(--color-ink-faint)' },
    },
    { types: ['keyword'], style: { color: 'var(--color-syntax-keyword)' } },
    {
      types: ['function', 'function-definition', 'method', 'macro', 'coord'],
      style: { color: 'var(--color-syntax-function)' },
    },
    {
      types: ['number', 'boolean', 'null', 'constant', 'date'],
      style: { color: 'var(--color-syntax-constant)' },
    },
    {
      types: ['string', 'char', 'regex', 'attr-value', 'url'],
      style: { color: 'var(--color-syntax-string)' },
    },
    {
      types: [
        'class-name',
        'builtin',
        'type-definition',
        'tag',
        'selector',
        'table',
      ],
      style: { color: 'var(--color-syntax-type)' },
    },
    {
      types: [
        'attribute',
        'attr-name',
        'decorator',
        'annotation',
        'lifetime-annotation',
        'key',
      ],
      style: { color: 'var(--color-syntax-attribute)' },
    },
    { types: ['inserted'], style: { color: 'var(--color-ok)' } },
    { types: ['deleted'], style: { color: 'var(--color-alert-strong)' } },
  ],
}

/* Extra grammars beyond the Prism core bundled by `prism-react-renderer`.
   Registered alongside JSON above so `CodeHighlight` can highlight
   `sandbox::run` / `fs::read` payloads without pulling in `prismjs`.
   Grammars taken verbatim from prismjs/components, trimmed to the
   tokens our `syntaxTheme` actually styles (the unstyled tokens render
   as plain `var(--color-ink)`). */
if (!prism.languages.python) {
  prism.languages.python = {
    comment: { pattern: /(^|[^\\])#.*/, lookbehind: true, greedy: true },
    'string-interpolation': {
      pattern:
        /(?:f|fr|rf)(?:("""|''')[\s\S]*?\1|("|')(?:\\.|(?!\2)[^\\\r\n])*\2)/i,
      greedy: true,
      alias: 'string',
    },
    'triple-quoted-string': {
      pattern: /(?:[rub]|br|rb)?("""|''')[\s\S]*?\1/i,
      greedy: true,
      alias: 'string',
    },
    string: {
      pattern: /(?:[rub]|br|rb)?("|')(?:\\.|(?!\1)[^\\\r\n])*\1/i,
      greedy: true,
    },
    function: {
      pattern: /((?:^|\s)def[ \t]+)[a-zA-Z_]\w*(?=\s*\()/g,
      lookbehind: true,
    },
    'class-name': { pattern: /(\bclass\s+)\w+/i, lookbehind: true },
    decorator: {
      pattern: /(^[\t ]*)@\w+(?:\.\w+)*/m,
      lookbehind: true,
      alias: ['annotation', 'punctuation'],
    },
    keyword:
      /\b(?:and|as|assert|async|await|break|class|continue|def|del|elif|else|except|exec|finally|for|from|global|if|import|in|is|lambda|nonlocal|not|or|pass|print|raise|return|try|while|with|yield)\b/,
    builtin:
      /\b(?:False|None|True|__import__|abs|all|any|apply|ascii|basestring|bin|bool|buffer|bytearray|bytes|callable|chr|classmethod|cmp|coerce|compile|complex|delattr|dict|dir|divmod|enumerate|eval|execfile|file|filter|float|format|frozenset|getattr|globals|hasattr|hash|help|hex|id|input|int|intern|isinstance|issubclass|iter|len|list|locals|long|map|max|memoryview|min|next|object|oct|open|ord|pow|property|range|raw_input|reduce|reload|repr|reversed|round|set|setattr|slice|sorted|staticmethod|str|sum|super|tuple|type|unichr|unicode|vars|xrange|zip)\b/,
    boolean: /\b(?:True|False|None)\b/,
    number:
      /(?:\b(?=\d)|\B(?=\.))(?:0[bo])?(?:(?:\d|0x[\da-f])[\da-f]*(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?j?\b/i,
    operator: /[-+%=]=?|!=|\*\*?=?|\/\/?=?|<[<=>]?|>[=>]?|[&|^~]/,
    punctuation: /[{}[\];(),.:]/,
  }
}

if (!prism.languages.bash) {
  /* Minimal bash grammar — enough for the shapes that ride through
     `sandbox::run` (shebangs, comments, common builtins) without
     mirroring the full prism-bash grammar. */
  prism.languages.bash = {
    shebang: { pattern: /^#!\s*\/.*/, alias: 'important' },
    comment: { pattern: /(^|[^"{\\$])#.*/, lookbehind: true },
    string: [{ pattern: /("|')(?:\\[\s\S]|(?!\1)[^\\])*\1/, greedy: true }],
    number: /(?:\b\d+(?:\.\d+)?|\B\.\d+)(?:e[+-]?\d+)?/i,
    keyword:
      /\b(?:if|then|else|elif|fi|for|while|in|until|do|done|case|esac|function|select|return|break|continue|exit|export|local|readonly|set|shift|trap|umask|wait)\b/,
    builtin:
      /\b(?:cd|echo|eval|exec|exit|export|pwd|printf|read|test|true|false|sleep|kill|sudo|cat|ls|mkdir|rm|mv|cp|grep|sed|awk|find|chmod|chown|tar|gzip|gunzip|curl|wget|git|node|python|python3|npm|pnpm|yarn|pip|pip3)\b/,
    operator: /<<<?|>>?|[=!]==?|[<>]=?|&&?|\|\|?|[*?+~^]/,
    punctuation: /[{}[\];(),.]/,
  }
  prism.languages.sh = prism.languages.bash
  prism.languages.shell = prism.languages.bash
}

if (!prism.languages.toml) {
  /* prismjs/components/prism-toml.js, verbatim. */
  const key = /(?:[\w-]+|'[^'\n\r]*'|"(?:\\.|[^\\"\r\n])*")/.source
  const withKey = (pattern: string) => pattern.replace(/__/g, () => key)
  prism.languages.toml = {
    comment: { pattern: /#.*/, greedy: true },
    table: {
      pattern: new RegExp(
        withKey(/(^[\t ]*\[\s*(?:\[\s*)?)__(?:\s*\.\s*__)*(?=\s*\])/.source),
        'm',
      ),
      lookbehind: true,
      greedy: true,
      alias: 'class-name',
    },
    key: {
      pattern: new RegExp(
        withKey(/(^[\t ]*|[{,]\s*)__(?:\s*\.\s*__)*(?=\s*=)/.source),
        'm',
      ),
      lookbehind: true,
      greedy: true,
      alias: 'property',
    },
    string: {
      pattern:
        /"""(?:\\[\s\S]|[^\\])*?"""|'''[\s\S]*?'''|'[^'\n\r]*'|"(?:\\.|[^\\"\r\n])*"/,
      greedy: true,
    },
    date: [
      {
        pattern:
          /\b\d{4}-\d{2}-\d{2}(?:[T\s]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?)?\b/i,
        alias: 'number',
      },
      { pattern: /\b\d{2}:\d{2}:\d{2}(?:\.\d+)?\b/, alias: 'number' },
    ],
    number:
      /(?:\b0(?:x[\da-zA-Z]+(?:_[\da-zA-Z]+)*|o[0-7]+(?:_[0-7]+)*|b[10]+(?:_[10]+)*))\b|[-+]?\b\d+(?:_\d+)*(?:\.\d+(?:_\d+)*)?(?:[eE][+-]?\d+(?:_\d+)*)?\b|[-+]?\b(?:inf|nan)\b/,
    boolean: /\b(?:false|true)\b/,
    punctuation: /[.,=[\]{}]/,
  }
}

if (!prism.languages.diff) {
  /* Unified and normal diffs: file and hunk headers as `coord`, then whole
     removed / added lines. Trimmed from prismjs/components/prism-diff.js. */
  prism.languages.diff = {
    coord: [
      /^(?:diff --git|index) .*$/m,
      /^(?:\*{3}|-{3}|\+{3}).*$/m,
      /^@@.*$/m,
      /^\d.*$/m,
    ],
    deleted: /^[-<].*$/m,
    inserted: /^[+>].*$/m,
  }
}

/* Fence names people and models write that Prism does not register under
   that id. Each points at a grammar registered by now, so a ```rs or
   ```golang block highlights instead of falling back to plain text. */
const LANGUAGE_ALIASES: Record<string, string> = {
  rs: 'rust',
  golang: 'go',
  zsh: 'bash',
  patch: 'diff',
  jsonc: 'json',
  json5: 'json',
  mjs: 'javascript',
  cjs: 'javascript',
  mts: 'typescript',
  cts: 'typescript',
}
for (const [alias, target] of Object.entries(LANGUAGE_ALIASES)) {
  if (!prism.languages[alias] && prism.languages[target]) {
    prism.languages[alias] = prism.languages[target]
  }
}

interface JsonHighlightProps {
  code: string
  className?: string
  /** When set, the wrapper uses `whitespace-pre-wrap` so long lines wrap
      naturally; default keeps strict pre so JSON indentation is preserved. */
  wrap?: boolean
}

/**
 * Renders a JSON payload as a `<pre><code>` block with Prism-driven token
 * coloring. The chrome (`bg-bg`, `font-code`, `text-[12.5px]/[1.55]`) mirrors
 * the existing fenced code block in the assistant Markdown so a highlighted
 * pane sits side-by-side with a plain `<pre>` without a visual seam.
 */
interface CodeHighlightProps {
  code: string
  /** Prism language id (`'javascript'`, `'python'`, `'bash'`, …). When
      the grammar isn't registered Prism falls back to plain text — the
      same chrome still applies, just without coloring. */
  language: string
  className?: string
  wrap?: boolean
}

/**
 * Generic Prism-driven highlight sibling to `JsonHighlight`. Same
 * `bg-bg / font-code / text-[12.5px]` chrome, same `syntaxTheme` token
 * palette (so multi-language blocks visually compose with JSON blocks
 * in the same surface). Unknown languages render as plain text without
 * crashing; the language id passes straight to Prism.
 */
export function CodeHighlight({
  code,
  language,
  className,
  wrap,
}: CodeHighlightProps) {
  return (
    <Highlight
      prism={Prism}
      theme={syntaxTheme}
      language={language}
      code={code}
    >
      {({ tokens, getLineProps, getTokenProps, className: hlClass, style }) => (
        <pre
          className={cn(
            'bg-bg overflow-x-auto px-3 py-2 font-code text-[12.5px] leading-[1.55]',
            wrap ? 'whitespace-pre-wrap break-words' : 'whitespace-pre',
            hlClass,
            className,
          )}
          style={style}
        >
          <code>
            {tokens.map((line, lineIdx) => {
              const lineProps = getLineProps({ line })
              const lineKey = `${lineIdx}:${line.length}`
              return (
                <span key={lineKey} {...lineProps}>
                  {line.map((token, tokenIdx) => {
                    const tokenProps = getTokenProps({ token })
                    const tokenKey = `${tokenIdx}:${token.types.join('.')}:${token.content}`
                    return <span key={tokenKey} {...tokenProps} />
                  })}
                  {lineIdx < tokens.length - 1 ? '\n' : ''}
                </span>
              )
            })}
          </code>
        </pre>
      )}
    </Highlight>
  )
}

export function JsonHighlight({ code, className, wrap }: JsonHighlightProps) {
  return (
    <Highlight prism={Prism} theme={syntaxTheme} language="json" code={code}>
      {({ tokens, getLineProps, getTokenProps, className: hlClass, style }) => (
        <pre
          className={cn(
            'bg-bg overflow-x-auto px-3 py-2 font-code text-[12.5px] leading-[1.55]',
            wrap ? 'whitespace-pre-wrap break-words' : 'whitespace-pre',
            hlClass,
            className,
          )}
          style={style}
        >
          <code>
            {tokens.map((line, lineIdx) => {
              const lineProps = getLineProps({ line })
              /* Prism's tokenization is deterministic for a given `code`: line
                 and token order maps 1:1 to source position, so positional
                 keys are stable between renders of the same input. */
              const lineKey = `${lineIdx}:${line.length}`
              return (
                <span key={lineKey} {...lineProps}>
                  {line.map((token, tokenIdx) => {
                    const tokenProps = getTokenProps({ token })
                    const tokenKey = `${tokenIdx}:${token.types.join('.')}:${token.content}`
                    return <span key={tokenKey} {...tokenProps} />
                  })}
                  {lineIdx < tokens.length - 1 ? '\n' : ''}
                </span>
              )
            })}
          </code>
        </pre>
      )}
    </Highlight>
  )
}
