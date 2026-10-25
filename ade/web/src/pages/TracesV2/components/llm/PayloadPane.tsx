/**
 * One captured payload (a request, a response, a tool's arguments) as a
 * highlighted code pane: a header naming the pane and its language, the
 * engine's truncation notice when the capture was cut, a copy button and a
 * wrap toggle, and a body that clamps tall payloads behind "Show all". The
 * body is the console's Prism highlighter (`lib/syntax`) wearing the
 * trace panes' Vesper code theme (`.code-theme-editor` in index.css):
 * Geist Mono at 13px, loose leading, wide tracking.
 */

import { Check, Copy, WrapText } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Button } from '@/components/shadcn/button'
import { copyTextToClipboard } from '@/lib/clipboard'
import { CodeHighlight, JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'

/** Rendered lines above which the body clamps behind a "Show all" footer. */
const CLAMP_LINES = 28
const CLAMP_MAX_H = 'max-h-[26rem]'

export interface PayloadPaneProps {
  label: string
  /** The value to show: an object renders as pretty JSON, a string as-is. */
  value: unknown
  /** Prism language for a string value; objects are always JSON. */
  language?: string
  /** A short note beside the label: `json · 1,204 lines`, `truncated` … */
  hint?: string
  /** Bytes the engine dropped from this capture, when it was cut. */
  truncatedBytes?: number | null
  /** Collapsed by default (long system prompts, tool schemas). */
  defaultCollapsed?: boolean
  className?: string
}

function countLines(s: string): number {
  let n = 1
  for (let i = 0; i < s.length; i++) if (s[i] === '\n') n++
  return n
}

export function stringifyPayload(value: unknown): string {
  if (typeof value === 'string') return value
  try {
    return JSON.stringify(value, null, 2) ?? String(value)
  } catch {
    return String(value)
  }
}

export function PayloadPane({
  label,
  value,
  language,
  hint,
  truncatedBytes,
  defaultCollapsed = false,
  className,
}: PayloadPaneProps) {
  const text = useMemo(() => stringifyPayload(value), [value])
  const json = typeof value !== 'string'
  const lineCount = useMemo(() => countLines(text), [text])
  const clampable = lineCount > CLAMP_LINES
  const [expanded, setExpanded] = useState(!defaultCollapsed && !clampable)
  const [collapsed, setCollapsed] = useState(defaultCollapsed)
  const [wrap, setWrap] = useState(true)
  const [copied, setCopied] = useState(false)

  const copy = () => {
    void copyTextToClipboard(text).then((ok) => {
      if (!ok) return
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1200)
    })
  }

  const lang = json ? 'json' : (language ?? 'text')

  return (
    <div
      className={cn(
        'code-theme-editor min-w-0 overflow-hidden rounded-lg border border-border bg-background',
        className,
      )}
      data-payload-pane={label.toLowerCase().replaceAll(' ', '-')}
    >
      <div className="flex h-8 items-center gap-2 border-b border-border bg-muted/40 pr-1 pl-3">
        <button
          type="button"
          onClick={() => setCollapsed((v) => !v)}
          aria-expanded={!collapsed}
          className="flex min-w-0 flex-1 items-center gap-2 text-left text-xs font-medium text-foreground"
        >
          <span className="truncate">{label}</span>
          <span className="shrink-0 rounded-md bg-muted px-1.5 py-px font-mono text-[11px] text-muted-foreground">
            {lang}
          </span>
          {hint ? (
            <span className="truncate text-[11px] font-normal text-muted-foreground">
              {hint}
            </span>
          ) : null}
          {truncatedBytes != null && truncatedBytes > 0 ? (
            <span
              className="shrink-0 rounded-md bg-warn-muted px-1.5 py-px text-[11px] font-medium text-warn-strong"
              title={`the engine caps a captured payload at 64 KB; ${truncatedBytes.toLocaleString('en-US')} bytes were not recorded`}
            >
              truncated · {Math.round(truncatedBytes / 1024)} KB dropped
            </span>
          ) : null}
        </button>
        <span className="shrink-0 font-mono text-[11px] text-muted-foreground tabular-nums">
          {lineCount.toLocaleString('en-US')}{' '}
          {lineCount === 1 ? 'line' : 'lines'}
        </span>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-pressed={wrap}
          aria-label={wrap ? 'disable line wrapping' : 'wrap long lines'}
          title={wrap ? 'Scroll long lines' : 'Wrap long lines'}
          onClick={() => setWrap((v) => !v)}
          className={cn(wrap && 'bg-muted text-foreground')}
        >
          <WrapText />
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label={copied ? 'copied' : `copy ${label}`}
          title={copied ? 'Copied' : 'Copy'}
          onClick={copy}
        >
          {copied ? <Check className="text-ok" /> : <Copy />}
        </Button>
      </div>
      {collapsed ? null : (
        <>
          <div
            className={cn(
              'min-w-0',
              clampable && !expanded && `${CLAMP_MAX_H} overflow-hidden`,
            )}
          >
            {json ? (
              <JsonHighlight
                code={text}
                wrap={wrap}
                className="px-3.5 py-2.5"
              />
            ) : (
              <CodeHighlight
                code={text}
                language={lang}
                wrap={wrap}
                className="px-3.5 py-2.5"
              />
            )}
          </div>
          {clampable ? (
            <button
              type="button"
              onClick={() => setExpanded((v) => !v)}
              aria-expanded={expanded}
              className="w-full border-t border-border bg-muted/40 px-3 py-1 text-center text-[11px] font-medium text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
            >
              {expanded
                ? 'Collapse'
                : `Show all · ${lineCount.toLocaleString('en-US')} lines`}
            </button>
          ) : null}
        </>
      )}
    </div>
  )
}
