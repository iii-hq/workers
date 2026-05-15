import type { Element, Root, RootContent, Text } from 'hast'
import ReactMarkdown, { type Components } from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { FunctionMentionPill } from '@/components/chat/lexical/FunctionMentionNode'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'

interface MarkdownProps {
  children: string
  className?: string
}

/* Matches `@fn(<id>)` where `<id>` excludes whitespace and `)`. Tolerates
   trailing punctuation outside the parens. The pattern is intentionally
   defensive: we never accept arbitrary content inside the parens, so the
   pill can't be smuggled into otherwise-safe markdown. */
const FN_MENTION_RE = /@fn\(([^)\s]+)\)/g

/* Inline rehype plugin: walks the hast tree and splits any `text` node that
   contains `@fn(<id>)` into a mix of leftover text + a marker `span` element
   carrying the function id. The actual pill rendering happens in the `span`
   component override below. Code / pre subtrees are skipped so literal
   mentions inside fenced blocks stay verbatim. */
function rehypeFnMention() {
  return (tree: Root) => walk(tree)
}

function walk(node: Root | Element): void {
  if (
    node.type === 'element' &&
    (node.tagName === 'code' || node.tagName === 'pre')
  ) {
    return
  }
  const children = node.children as RootContent[]
  for (let i = 0; i < children.length; i++) {
    const child = children[i]
    if (child.type === 'text') {
      const parts = splitMention(child.value)
      if (parts.length > 0) {
        children.splice(i, 1, ...parts)
        i += parts.length - 1
      }
    } else if (child.type === 'element') {
      walk(child)
    }
  }
}

/* hast's `className` lands as either `string[]` or a space-joined `string`,
   depending on how it was parsed. Normalize both shapes to a single check. */
function hasLanguageJson(node: Element): boolean {
  const cls = node.properties?.className
  if (Array.isArray(cls)) return cls.includes('language-json')
  if (typeof cls === 'string') return cls.split(/\s+/).includes('language-json')
  return false
}

function splitMention(value: string): Array<Text | Element> {
  if (!value.includes('@fn(')) return []
  const out: Array<Text | Element> = []
  let last = 0
  /* matchAll iterates with stateless semantics on a /g regex, so we don't
     have to babysit FN_MENTION_RE.lastIndex between calls. */
  for (const m of value.matchAll(FN_MENTION_RE)) {
    const index = m.index ?? 0
    if (index > last) {
      out.push({ type: 'text', value: value.slice(last, index) })
    }
    out.push({
      type: 'element',
      tagName: 'span',
      properties: { className: ['fn-mention'] },
      children: [{ type: 'text', value: m[1] }],
    })
    last = index + m[0].length
  }
  if (out.length === 0) return []
  if (last < value.length) {
    out.push({ type: 'text', value: value.slice(last) })
  }
  return out
}

const components: Components = {
  h1: ({ className, ...rest }) => (
    <h1
      className={cn(
        'font-mono text-[20px] font-medium tracking-[-0.02em] text-ink mt-6 mb-3 first:mt-0',
        className,
      )}
      {...rest}
    />
  ),
  h2: ({ className, ...rest }) => (
    <h2
      className={cn(
        'font-mono text-[16px] font-semibold tracking-[-0.01em] text-ink mt-6 mb-2 first:mt-0',
        className,
      )}
      {...rest}
    />
  ),
  h3: ({ className, ...rest }) => (
    <h3
      className={cn(
        'font-mono text-[14px] font-semibold text-ink mt-5 mb-2 first:mt-0',
        className,
      )}
      {...rest}
    />
  ),
  h4: ({ className, ...rest }) => (
    <h4
      className={cn(
        'font-mono text-[13px] font-semibold uppercase tracking-[0.06em] text-ink-faint mt-4 mb-2 first:mt-0',
        className,
      )}
      {...rest}
    />
  ),
  p: ({ className, ...rest }) => (
    <p
      className={cn(
        'font-mono text-[14px] leading-[1.7] text-ink my-3 first:mt-0 last:mb-0',
        className,
      )}
      {...rest}
    />
  ),
  ul: ({ className, ...rest }) => (
    <ul
      className={cn(
        'font-mono text-[14px] leading-[1.7] text-ink my-3 pl-5 list-disc marker:text-ink-faint',
        className,
      )}
      {...rest}
    />
  ),
  ol: ({ className, ...rest }) => (
    <ol
      className={cn(
        'font-mono text-[14px] leading-[1.7] text-ink my-3 pl-5 list-decimal marker:text-ink-faint tabular-nums',
        className,
      )}
      {...rest}
    />
  ),
  li: ({ className, ...rest }) => (
    <li className={cn('my-1', className)} {...rest} />
  ),
  blockquote: ({ className, ...rest }) => (
    <blockquote
      className={cn(
        'border-l-2 border-rule pl-4 my-3 text-ink-faint italic',
        className,
      )}
      {...rest}
    />
  ),
  hr: ({ className, ...rest }) => (
    <hr
      className={cn('my-6 border-0 border-t border-rule', className)}
      {...rest}
    />
  ),
  a: ({ className, ...rest }) => (
    <a
      className={cn(
        'text-ink underline decoration-rule decoration-1 underline-offset-2 hover:text-accent hover:decoration-accent transition-colors',
        className,
      )}
      target="_blank"
      rel="noopener noreferrer"
      {...rest}
    />
  ),
  strong: ({ className, ...rest }) => (
    <strong className={cn('font-semibold text-ink', className)} {...rest} />
  ),
  em: ({ className, ...rest }) => (
    <em className={cn('italic text-ink', className)} {...rest} />
  ),
  code: ({ className, children, ...rest }) => {
    /* react-markdown v9+: inline vs block is distinguished by parent (pre) -
       inline if no surrounding pre is present. We rely on the `pre` renderer
       to drop the wrapper styling on block code, and style inline here. */
    const isBlock = typeof className === 'string' && /language-/.test(className)
    if (isBlock) {
      return (
        <code className={cn('font-mono', className)} {...rest}>
          {children}
        </code>
      )
    }
    return (
      <code
        className={cn(
          'font-mono text-[12.5px] border border-rule-2 bg-paper-2 text-ink px-1 py-0.5',
          className,
        )}
        {...rest}
      >
        {children}
      </code>
    )
  },
  pre: ({ node, className, children, ...rest }) => {
    /* Detect a JSON fenced block (`<pre><code class="language-json">`) and
       route it to the Prism-driven highlighter. We read the raw text out of
       the hast tree directly instead of leaning on the rendered `code`
       output so the highlighter gets the unmodified source. */
    const codeChild = node?.children?.[0]
    if (
      codeChild &&
      codeChild.type === 'element' &&
      codeChild.tagName === 'code' &&
      hasLanguageJson(codeChild)
    ) {
      const text = codeChild.children?.[0]
      const source = text && text.type === 'text' ? text.value : ''
      return (
        <JsonHighlight
          code={source.replace(/\n$/, '')}
          className="border border-rule my-4"
        />
      )
    }
    return (
      <pre
        className={cn(
          'border border-rule bg-bg overflow-x-auto px-5 py-4 my-4 font-mono text-[12.5px] leading-[1.55] text-ink',
          className,
        )}
        {...rest}
      >
        {children}
      </pre>
    )
  },
  span: ({ className, children, ...rest }) => {
    const cls = typeof className === 'string' ? className : ''
    if (cls.split(/\s+/).includes('fn-mention')) {
      const id =
        typeof children === 'string'
          ? children
          : Array.isArray(children) && typeof children[0] === 'string'
            ? children[0]
            : ''
      return <FunctionMentionPill functionId={id} />
    }
    return (
      <span className={className} {...rest}>
        {children}
      </span>
    )
  },
  table: ({ className, ...rest }) => (
    <div className="my-4 overflow-x-auto border border-rule">
      <table
        className={cn(
          'w-full border-collapse font-mono text-[13px] text-ink',
          className,
        )}
        {...rest}
      />
    </div>
  ),
  thead: ({ className, ...rest }) => (
    <thead className={cn('bg-panel', className)} {...rest} />
  ),
  th: ({ className, ...rest }) => (
    <th
      className={cn(
        'text-left px-3 py-2 border-b border-rule font-medium text-[11px] uppercase tracking-[0.06em] text-ink-faint',
        className,
      )}
      {...rest}
    />
  ),
  td: ({ className, ...rest }) => (
    <td
      className={cn(
        'px-3 py-2 border-b border-rule-2 last:border-b-0',
        className,
      )}
      {...rest}
    />
  ),
}

export function Markdown({ children, className }: MarkdownProps) {
  return (
    <div className={cn('text-ink', className)}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeFnMention]}
        components={components}
      >
        {children}
      </ReactMarkdown>
    </div>
  )
}
