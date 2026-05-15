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

/* Monochrome blueprint theme — every color references a design token so the
   palette inverts automatically with `[data-theme="dark"]`. Keys get the
   strongest ink, structural punctuation fades to `ink-ghost`, and the single
   tonal hit is the hot orange `accent` reserved for literals (numbers,
   booleans, null) so the eye lands on the values that actually matter. */
const jsonTheme: PrismTheme = {
  plain: { color: 'var(--color-ink)' },
  styles: [
    { types: ['property'], style: { color: 'var(--color-ink)' } },
    { types: ['string'], style: { color: 'var(--color-ink-faint)' } },
    {
      types: ['number', 'boolean', 'null', 'keyword'],
      style: { color: 'var(--color-accent)', fontStyle: 'italic' },
    },
    {
      types: ['punctuation', 'operator'],
      style: { color: 'var(--color-ink-ghost)' },
    },
    {
      types: ['comment'],
      style: { color: 'var(--color-ink-ghost)', fontStyle: 'italic' },
    },
  ],
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
 * coloring. The chrome (`bg-bg`, `font-mono`, `text-[12.5px]/[1.55]`) mirrors
 * the existing fenced code block in the assistant Markdown so a highlighted
 * pane sits side-by-side with a plain `<pre>` without a visual seam.
 */
export function JsonHighlight({ code, className, wrap }: JsonHighlightProps) {
  return (
    <Highlight prism={Prism} theme={jsonTheme} language="json" code={code}>
      {({ tokens, getLineProps, getTokenProps, className: hlClass, style }) => (
        <pre
          className={cn(
            'bg-bg overflow-x-auto px-3 py-2 font-mono text-[12.5px] leading-[1.55]',
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
