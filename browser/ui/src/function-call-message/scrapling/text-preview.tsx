import type { ReactNode } from 'react'
import { cn } from '../../lib/cn'

/**
 * A fetched page's text, still a `<pre>` (no markdown render), with just
 * enough colour to scan: links and bare URLs in the link ink, heading
 * lines in weight with their `#` marks receded, setext/rule lines receded.
 * Plain text gets the URL pass only.
 */
export function TextPreview({
  text,
  format,
  className,
}: {
  text: string
  format?: string
  className?: string
}) {
  const markdown = isMarkdown(format)
  const lines = text.split('\n')
  return (
    <pre className={cn('br-ui-text', className)}>
      <code>
        {lines.map((line, i) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: a static text snapshot; lines never reorder
          <span key={i}>
            {markdown ? markdownLine(line, i) : inline(line, `${i}`, false)}
            {i < lines.length - 1 ? '\n' : null}
          </span>
        ))}
      </code>
    </pre>
  )
}

function isMarkdown(format?: string): boolean {
  const f = (format ?? '').toLowerCase()
  return f === 'markdown' || f === 'md'
}

const HEADING = /^(\s{0,3})(#{1,6})(\s+)(.*)$/
const RULE = /^\s{0,3}(?:={3,}|-{3,}|\*{3,}|_{3,})\s*$/

function markdownLine(line: string, n: number): ReactNode {
  const heading = HEADING.exec(line)
  if (heading) {
    const [, indent, marks, gap, body] = heading
    return (
      <>
        {indent}
        <span className="br-ui-md-mark">{marks}</span>
        {gap}
        <span className="br-ui-md-heading">{inline(body ?? '', `${n}`, true)}</span>
      </>
    )
  }
  if (RULE.test(line)) return <span className="br-ui-md-mark">{line}</span>
  return inline(line, `${n}`, true)
}

// `[text](url)` / `![alt](url)` (optional "title"), or a bare http(s) URL.
const MD_INLINE =
  /(!?)\[([^\]\n]*)\]\(([^)\s]*)((?:\s+"[^"\n]*")?)\)|https?:\/\/[^\s<>()[\]"'`]+/g
const BARE_URL = /https?:\/\/[^\s<>()[\]"'`]+/g
const TRAILING_PUNCT = /[.,;:!?]+$/

function inline(line: string, key: string, markdown: boolean): ReactNode {
  const re = markdown ? MD_INLINE : BARE_URL
  re.lastIndex = 0
  const parts: ReactNode[] = []
  let last = 0
  let n = 0
  for (const hit of line.matchAll(re)) {
    const start = hit.index ?? 0
    let text = hit[0]
    if (start > last) parts.push(line.slice(last, start))
    const k = `${key}:${n++}`
    if (markdown && hit[3] !== undefined) {
      const [, bang, label, url, title] = hit
      parts.push(
        <span key={k}>
          <span className="br-ui-md-mark">{`${bang}[`}</span>
          <span className="br-ui-md-link">{label}</span>
          <span className="br-ui-md-mark">](</span>
          <span className="br-ui-md-url">{url}</span>
          {title ? <span className="br-ui-md-mark">{title}</span> : null}
          <span className="br-ui-md-mark">)</span>
        </span>,
      )
    } else {
      // a sentence-ending period is not part of the URL
      const tail = TRAILING_PUNCT.exec(text)?.[0] ?? ''
      if (tail) text = text.slice(0, -tail.length)
      parts.push(
        <span key={k} className="br-ui-md-link">
          {text}
        </span>,
      )
      if (tail) parts.push(tail)
    }
    last = start + hit[0].length
    if (n > 400) break
  }
  if (parts.length === 0) return line
  if (last < line.length) parts.push(line.slice(last))
  return parts
}
