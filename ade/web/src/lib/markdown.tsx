import { PreviewCard } from '@base-ui/react/preview-card'
import type { Element, Root, RootContent, Text } from 'hast'
import { ArrowUpRight, ImageIcon, ImageOff } from 'lucide-react'
import {
  type ComponentPropsWithoutRef,
  type ReactNode,
  useId,
  useState,
} from 'react'
import ReactMarkdown, {
  type Components,
  defaultUrlTransform,
} from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { FileMentionPill } from '@/components/chat/lexical/FileMentionNode'
import { FunctionMentionPill } from '@/components/chat/lexical/FunctionMentionNode'
import { SlashCommandPill } from '@/components/chat/lexical/SlashCommandNode'
import { WorkerMentionPill } from '@/components/chat/mentions/WorkerMentionPill'
import { CodeBlock } from '@/components/ui/CodeBlock'
import {
  MermaidDiagram,
  MermaidStreamingContext,
} from '@/components/ui/MermaidDiagram'
import {
  Table,
  TableBody,
  TableCaption,
  TableCell,
  TableFooter,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from '@/components/ui/Table'
import {
  type FileMentionRef,
  parseFileMentionInner,
} from '@/lib/file-mention-token'
import { useOpenMessageFile } from '@/lib/file-navigation'
import { inlineCodeKind, inlineFileIconSvg } from '@/lib/inline-code-kind'
import {
  markdownFileLinkError,
  parseMarkdownFileLink,
} from '@/lib/markdown-file-link'
import { findMentions } from '@/lib/mentions/token'
import { NEUTRAL_SURFACE } from '@/lib/neutral-surface'
import { SKILL_PREFIX, SKILL_TOKEN_SOURCE } from '@/lib/slash-commands'
import { cn } from '@/lib/utils'

interface MarkdownProps {
  children: string
  className?: string
  streaming?: boolean
}

/* Matches `@fn(<id>)` (id excludes whitespace and `)`), `#file(<path>)`
   (path excludes `)` — file names may carry spaces) or a `/skill:<id>`
   invocation (the composer's own token shape, see `slash-commands`).
   Tolerates trailing punctuation outside the parens / after the id. The
   pattern is intentionally defensive: we never accept arbitrary content
   inside the parens, so the pill can't be smuggled into otherwise-safe
   markdown. */
const MENTION_RE = new RegExp(
  String.raw`@fn\(([^)\s]+)\)|#file\(([^)]+)\)|${SKILL_TOKEN_SOURCE}`,
  'g',
)

/* Inline rehype plugin: walks the hast tree and splits any `text` node that
   contains `@fn(<id>)`, `#file(<path>)` or `/skill:<id>` into a mix of
   leftover text + a marker `span` element carrying the mention payload. The
   actual pill rendering happens in the `span` component override below.
   Code / pre subtrees stay verbatim; anchors are skipped to avoid nesting
   interactive mentions inside links. */
/** Transform prose mentions into pills while leaving literal code and link labels intact. */
function rehypeFnMention() {
  return (tree: Root) => walk(tree)
}

/** Rewrite mention text without creating nested interactive elements or modifying code blocks. */
function walk(node: Root | Element): void {
  if (
    node.type === 'element' &&
    (node.tagName === 'code' || node.tagName === 'pre' || node.tagName === 'a')
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
   depending on how it was parsed. Extract the fence language from either shape.
   (Typed `unknown` on purpose: current @types/hast declares only the array
   shape, but the string shape still occurs at runtime.) */
function codeLanguage(node: Element): string | undefined {
  const cls: unknown = node.properties?.className
  const classes = Array.isArray(cls)
    ? cls
    : typeof cls === 'string'
      ? cls.split(/\s+/)
      : []
  return classes
    .find(
      (name): name is string =>
        typeof name === 'string' && name.startsWith('language-'),
    )
    ?.slice('language-'.length)
    .toLowerCase()
}

interface MentionSpan {
  index: number
  length: number
  element: Element
}

/* Built-in mentions: `@fn(<id>)`, `#file(<path>)`, `/skill:<id>`. */
function builtinMentionSpans(value: string): MentionSpan[] {
  if (
    !value.includes('@fn(') &&
    !value.includes('#file(') &&
    !value.includes(SKILL_PREFIX)
  ) {
    return []
  }
  const spans: MentionSpan[] = []
  /* matchAll iterates with stateless semantics on a /g regex, so we don't
     have to babysit MENTION_RE.lastIndex between calls. */
  for (const m of value.matchAll(MENTION_RE)) {
    /* Group 1 = `@fn` id, group 2 = `#file` path, group 3 = skill id (the
       alternation makes exactly one of them defined per match). */
    const className =
      m[1] !== undefined
        ? 'fn-mention'
        : m[2] !== undefined
          ? 'file-mention'
          : 'skill-mention'
    spans.push({
      index: m.index ?? 0,
      length: m[0].length,
      element: {
        type: 'element',
        tagName: 'span',
        properties: { className: [className] },
        children: [{ type: 'text', value: m[1] ?? m[2] ?? m[3] }],
      },
    })
  }
  return spans
}

/* Worker-defined mentions: `@<provider>(id="<id>")`. Markdown has already
   applied its backslash escapes by now, so an id holding a quote or a
   backslash (never the case for the uuids and keys providers use) does not
   render as a pill here; the composer and the agent read the raw text. */
function workerMentionSpans(value: string): MentionSpan[] {
  return findMentions(value).map((m) => ({
    index: m.index,
    length: m.token.length,
    element: {
      type: 'element',
      tagName: 'span',
      properties: {
        className: ['worker-mention'],
        dataMentionName: m.name,
        dataMentionId: m.id,
      },
      children: [{ type: 'text', value: m.token }],
    },
  }))
}

function splitMention(value: string): Array<Text | Element> {
  const spans = [...builtinMentionSpans(value), ...workerMentionSpans(value)]
  if (spans.length === 0) return []
  spans.sort((a, b) => a.index - b.index)
  const out: Array<Text | Element> = []
  let last = 0
  for (const span of spans) {
    // Overlapping matches cannot both be pills; the earlier one wins.
    if (span.index < last) continue
    if (span.index > last) {
      out.push({ type: 'text', value: value.slice(last, span.index) })
    }
    out.push(span.element)
    last = span.index + span.length
  }
  if (last < value.length) {
    out.push({ type: 'text', value: value.slice(last) })
  }
  return out
}

const linkClassName =
  'chat-prose-link text-ink underline decoration-1 underline-offset-[3px] transition-colors'

/** Open a validated local reference and announce recoverable failures beside its label. */
function FileReferenceButton({
  reference,
  problem,
  className,
  children,
  title,
}: {
  reference?: FileMentionRef | null
  problem?: string | null
  className?: string
  children: ReactNode
  title: string
}) {
  const openFile = useOpenMessageFile()
  const [error, setError] = useState<string | null>(null)
  const id = useId()
  return (
    <>
      <button
        type="button"
        className={className}
        title={title}
        aria-describedby={error ? id : undefined}
        onClick={async () => {
          setError(null)
          try {
            if (problem) throw new Error(problem)
            if (reference) await openFile?.(reference)
          } catch (cause) {
            setError(
              cause instanceof Error
                ? cause.message
                : 'Could not open this file. Try again.',
            )
          }
        }}
      >
        {children}
      </button>
      {error ? (
        <span
          id={id}
          role="alert"
          className="ml-2 break-words text-[12px] text-alert"
        >
          {error}
        </span>
      ) : null}
    </>
  )
}

/** Route local file destinations to the originating chat's IDE; preserve ordinary web links. */
function MarkdownLink({
  href,
  className,
  children,
  title,
}: ComponentPropsWithoutRef<'a'>) {
  const openFile = useOpenMessageFile()
  const ref = href ? parseMarkdownFileLink(href) : null
  const problem = href ? markdownFileLinkError(href) : null
  if (openFile && (ref || problem)) {
    return (
      <FileReferenceButton
        reference={ref}
        problem={problem}
        className={cn(
          linkClassName,
          'cursor-pointer text-left break-words focus-visible:outline-2 focus-visible:outline-rule-focus',
          className,
        )}
        title={
          ref
            ? `open ${ref.path}${ref.range ? `:${ref.range.from}-${ref.range.to}` : ''} in the IDE (current file on disk)`
            : (problem ?? '')
        }
      >
        {children}
      </FileReferenceButton>
    )
  }
  return (
    <a
      href={href}
      className={cn(linkClassName, className)}
      title={title}
      target="_blank"
      rel="noopener noreferrer"
    >
      {children}
      <ArrowUpRight aria-hidden className="chat-link-arrow size-4" />
    </a>
  )
}

const INLINE_CODE_CLASS =
  'font-code text-[12.5px] border border-rule-2 bg-paper-2 text-ink px-1 py-0.5'

/** A file's language mark (Material, as the IDE tree draws it), 16px. */
function InlineFileMark({ icon }: { icon: string }) {
  const svg = inlineFileIconSvg(icon)
  if (!svg) return null
  return (
    <svg
      aria-hidden="true"
      viewBox={svg.viewBox}
      className="chat-inline-file-mark"
      // Generated from the bundled Material set, never from message content.
      // biome-ignore lint/security/noDangerouslySetInnerHtml: static, generated icon markup
      dangerouslySetInnerHTML={{ __html: svg.body }}
    />
  )
}

/**
 * Inline code that is more than code: a bare web address becomes a link
 * that opens in a new tab; a file name gets its language mark, and opens in
 * the IDE when the chat has one (like a markdown file link).
 */
function InlineCode({
  text,
  className,
  children,
  rest,
}: {
  text: string | null
  className?: string
  children: ReactNode
  rest: ComponentPropsWithoutRef<'code'>
}) {
  const openFile = useOpenMessageFile()
  const kind = text === null ? { kind: 'code' as const } : inlineCodeKind(text)
  if (kind.kind === 'url') {
    return (
      <a
        href={kind.href}
        target="_blank"
        rel="noopener noreferrer"
        title={`open ${kind.href}`}
        className="chat-inline-code-link"
      >
        <code className={cn(INLINE_CODE_CLASS, className)} {...rest}>
          {children}
        </code>
        <ArrowUpRight aria-hidden className="chat-link-arrow size-4" />
      </a>
    )
  }
  if (kind.kind === 'file') {
    const code = (
      <code
        className={cn(INLINE_CODE_CLASS, 'chat-inline-file', className)}
        {...rest}
      >
        <InlineFileMark icon={kind.icon} />
        {children}
      </code>
    )
    const ref = text ? parseMarkdownFileLink(text) : null
    if (openFile && ref) {
      return (
        <FileReferenceButton
          reference={ref}
          className="chat-inline-file-button"
          title={`open ${ref.path} in the IDE (current file on disk)`}
        >
          {code}
        </FileReferenceButton>
      )
    }
    return code
  }
  return (
    <code className={cn(INLINE_CODE_CLASS, className)} {...rest}>
      {children}
    </code>
  )
}

/** Make transcript file mentions actionable without turning folder pills into file opens. */
function MarkdownFileMention({ path, range }: FileMentionRef) {
  const openFile = useOpenMessageFile()
  const pill = <FileMentionPill path={path} range={range} />
  if (!openFile || path.endsWith('/')) return pill
  return (
    <FileReferenceButton
      reference={{ path, range }}
      className="inline-flex align-middle cursor-pointer rounded-xs hover:bg-surface-hover focus-visible:outline-2 focus-visible:outline-rule-focus"
      title={`open ${path} in the IDE (current file on disk)`}
    >
      {pill}
    </FileReferenceButton>
  )
}

/** `/a/b/shot.png` → `shot.png`; a URL keeps its last path segment. */
export function imageFileName(src: string | undefined, alt: string): string {
  if (!src) return alt || 'image'
  const withoutQuery = src.split(/[?#]/)[0]
  const last = withoutQuery.split('/').filter(Boolean).pop()
  try {
    return last ? decodeURIComponent(last) : alt || 'image'
  } catch {
    return last ?? alt ?? 'image'
  }
}

/** A source the browser can fetch on its own; a bare local path cannot be. */
function isFetchableImage(src: string | undefined): boolean {
  return (
    !!src &&
    /^(https?:|data:|blob:|\/[^/]|\.\.?\/|[^/:]+\/)/.test(src) &&
    !/^\/Users\/|^\/home\/|^[A-Za-z]:\\/.test(src)
  )
}

/**
 * An image in a reply. A fetchable source renders inline; anything the
 * browser cannot load (a path on the machine, a missing file) becomes a
 * file chip naming the image, with the picture itself in a hover card
 * when it turns out to load after all. No broken-image glyphs either way.
 */
function MarkdownImage({
  src,
  alt,
  title,
}: {
  src?: string
  alt: string
  title?: string
}) {
  const [failed, setFailed] = useState(false)
  const [previewFailed, setPreviewFailed] = useState(false)
  const name = imageFileName(src, alt)
  if (src && isFetchableImage(src) && !failed) {
    return (
      <img
        className="chat-prose-image"
        src={src}
        alt={alt}
        title={title}
        loading="lazy"
        onError={() => setFailed(true)}
      />
    )
  }
  const chip = (
    // An image that is not drawn inline is still an image to a screen
    // reader: named by its alt text (else its file name).
    <span
      className="chat-prose-image-chip"
      data-image-chip=""
      role="img"
      aria-label={alt || name}
      title={title ?? src ?? name}
    >
      <ImageIcon aria-hidden className="size-4 shrink-0 text-ink-faint" />
      <span className="chat-prose-image-chip__name">{name}</span>
      {alt && alt !== name ? (
        <span className="chat-prose-image-chip__alt">{alt}</span>
      ) : null}
    </span>
  )
  if (!src) return chip
  return (
    <PreviewCard.Root>
      <PreviewCard.Trigger
        render={<span />}
        className="chat-prose-image-chip-trigger"
      >
        {chip}
      </PreviewCard.Trigger>
      <PreviewCard.Portal>
        <PreviewCard.Positioner side="top" sideOffset={8} className="z-50">
          <PreviewCard.Popup
            className={cn(
              NEUTRAL_SURFACE,
              'chat-prose-image-preview rounded-xl bg-popover p-1.5 font-inter text-popover-foreground shadow-md ring-1 ring-foreground/10',
            )}
          >
            {previewFailed ? (
              <span className="chat-prose-image-preview__missing">
                <ImageOff aria-hidden className="size-4 shrink-0" />
                No preview for {name}
              </span>
            ) : (
              <img
                className="chat-prose-image-preview__img"
                src={src}
                alt={alt}
                onError={() => setPreviewFailed(true)}
              />
            )}
          </PreviewCard.Popup>
        </PreviewCard.Positioner>
      </PreviewCard.Portal>
    </PreviewCard.Root>
  )
}

const components: Components = {
  h1: ({ className, ...rest }) => (
    <h1
      className={cn(
        'chat-prose-h1 font-mono text-[20px] font-medium tracking-[-0.02em] text-ink mt-6 mb-3 first:mt-0',
        className,
      )}
      {...rest}
    />
  ),
  h2: ({ className, ...rest }) => (
    <h2
      className={cn(
        'chat-prose-h2 font-mono text-[16px] font-semibold tracking-[-0.01em] text-ink mt-6 mb-2 first:mt-0',
        className,
      )}
      {...rest}
    />
  ),
  h3: ({ className, ...rest }) => (
    <h3
      className={cn(
        'chat-prose-h3 font-mono text-[14px] font-semibold text-ink mt-5 mb-2 first:mt-0',
        className,
      )}
      {...rest}
    />
  ),
  h4: ({ className, ...rest }) => (
    <h4
      className={cn(
        'chat-prose-h4 font-mono text-[13px] font-semibold text-ink-faint mt-4 mb-2 first:mt-0',
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
        'chat-prose-quote border-l-2 border-edge pl-4 my-3 text-ink-faint',
        className,
      )}
      {...rest}
    />
  ),
  hr: ({ className, ...rest }) => (
    <hr
      className={cn('my-6 border-0 border-t border-edge', className)}
      {...rest}
    />
  ),
  a: MarkdownLink,
  img: ({ src, alt, title }) => (
    <MarkdownImage
      src={typeof src === 'string' ? src : undefined}
      alt={typeof alt === 'string' ? alt : ''}
      title={typeof title === 'string' ? title : undefined}
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
        <code className={cn('font-code', className)} {...rest}>
          {children}
        </code>
      )
    }
    const text =
      typeof children === 'string'
        ? children
        : Array.isArray(children) &&
            children.every((c) => typeof c === 'string')
          ? children.join('')
          : null
    return (
      <InlineCode text={text} className={className} rest={rest}>
        {children}
      </InlineCode>
    )
  },
  pre: ({ node, className, children, ...rest }) => {
    // Read the unmodified source from hast, before code/mention rendering.
    const codeChild = node?.children?.[0]
    if (
      codeChild &&
      codeChild.type === 'element' &&
      codeChild.tagName === 'code'
    ) {
      const source = codeChild.children
        .filter((child) => child.type === 'text')
        .map((child) => child.value)
        .join('')
        .replace(/\n$/, '')
      const language = codeLanguage(codeChild)
      if (language === 'mermaid') {
        return <MermaidDiagram source={source} />
      }
      return <CodeBlock code={source} language={language || undefined} />
    }
    return (
      <pre
        className={cn(
          'rounded-xl border border-rule-2 bg-bg overflow-x-auto px-5 py-4 my-4 font-code text-[12.5px] leading-[1.55] text-ink',
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
    const classes = cls.split(/\s+/)
    if (classes.includes('worker-mention')) {
      const data = rest as Record<string, unknown>
      const name = data['data-mention-name']
      const id = data['data-mention-id']
      if (typeof name === 'string' && typeof id === 'string') {
        return <WorkerMentionPill name={name} id={id} openOnClick />
      }
    }
    if (
      classes.includes('fn-mention') ||
      classes.includes('file-mention') ||
      classes.includes('skill-mention')
    ) {
      const payload =
        typeof children === 'string'
          ? children
          : Array.isArray(children) && typeof children[0] === 'string'
            ? children[0]
            : ''
      if (classes.includes('fn-mention')) {
        return <FunctionMentionPill functionId={payload} />
      }
      if (classes.includes('skill-mention')) {
        return <SlashCommandPill command={`${SKILL_PREFIX}${payload}`} />
      }
      const ref = parseFileMentionInner(payload)
      return <MarkdownFileMention path={ref.path} range={ref.range} />
    }
    return (
      <span className={className} {...rest}>
        {children}
      </span>
    )
  },
  table: ({ className, ...rest }) => (
    <TableViewport className="my-4">
      <TableFrame>
        <Table density="compact" inset className={className} {...rest} />
      </TableFrame>
    </TableViewport>
  ),
  thead: ({ className, ...rest }) => (
    <TableHeader className={className} {...rest} />
  ),
  tbody: ({ className, ...rest }) => (
    <TableBody className={className} {...rest} />
  ),
  tfoot: ({ className, ...rest }) => (
    <TableFooter className={className} {...rest} />
  ),
  tr: ({ className, ...rest }) => <TableRow className={className} {...rest} />,
  th: ({ className, ...rest }) => <TableHead className={className} {...rest} />,
  td: ({ className, ...rest }) => <TableCell className={className} {...rest} />,
  caption: ({ className, ref: _ref, ...rest }) => (
    <TableCaption className={className} {...rest} />
  ),
}

/** Render sanitized Markdown, routing local references through the enclosing chat when available. */
export function Markdown({
  children,
  className,
  streaming = false,
}: MarkdownProps) {
  return (
    <MermaidStreamingContext.Provider value={streaming}>
      <div className={cn('text-ink', className)}>
        <ReactMarkdown
          urlTransform={(url, key) =>
            key === 'href' &&
            (parseMarkdownFileLink(url) || markdownFileLinkError(url))
              ? url
              : defaultUrlTransform(url)
          }
          remarkPlugins={[remarkGfm]}
          rehypePlugins={[rehypeFnMention]}
          components={components}
        >
          {children}
        </ReactMarkdown>
      </div>
    </MermaidStreamingContext.Provider>
  )
}
