import { Highlight, Prism, type PrismTheme } from 'prism-react-renderer'
import { cn } from '@/lib/utils'

const monoTheme: PrismTheme = {
  plain: { color: 'var(--color-ink)' },
  styles: [
    {
      types: ['comment', 'prolog', 'doctype', 'cdata'],
      style: { color: 'var(--color-ink-ghost)', fontStyle: 'italic' },
    },
    {
      types: ['string', 'attr-value', 'regex'],
      style: { color: 'var(--color-ink-faint)' },
    },
    {
      types: ['number', 'boolean', 'keyword', 'null'],
      style: { color: 'var(--color-accent)', fontStyle: 'italic' },
    },
    {
      types: ['function', 'class-name', 'builtin'],
      style: { color: 'var(--color-ink)' },
    },
    {
      types: ['punctuation', 'operator'],
      style: { color: 'var(--color-ink-ghost)' },
    },
  ],
}

interface CodeHighlightProps {
  code: string
  language?: string
  className?: string
}

export function CodeHighlight({
  code,
  language = 'text',
  className,
}: CodeHighlightProps) {
  const lang = Prism.languages[language] ? language : 'text'
  return (
    <Highlight prism={Prism} theme={monoTheme} language={lang} code={code}>
      {({ tokens, getLineProps, getTokenProps, className: hlClass, style }) => (
        <pre
          className={cn(
            'bg-bg overflow-x-auto px-3 py-2 font-mono text-[12.5px] leading-[1.55] whitespace-pre-wrap break-words',
            hlClass,
            className,
          )}
          style={style}
        >
          <code>
            {tokens.map((line, lineIdx) => {
              const lineProps = getLineProps({ line })
              /* Prism tokenization is deterministic for a given `code`;
                 positional+content keys stay stable across renders. */
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
