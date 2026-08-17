import { useMemo } from 'react'
import { type AnsiTone, parseAnsi } from '@/lib/ansi'
import { cn } from '@/lib/utils'

const toneClass: Record<AnsiTone, string> = {
  alert: 'text-alert',
  ok: 'text-ok',
  warn: 'text-warn',
  accent: 'text-accent',
}

interface AnsiTextProps {
  /** Raw terminal output, ANSI escapes included. */
  text: string
  className?: string
}

/**
 * Terminal text with its ANSI SGR colors mapped onto the design tokens:
 * red→alert, green→ok, yellow→warn, blue/cyan/magenta→accent,
 * bold→semibold. Extended-color introducers (38/48/58) are consumed with
 * their params, every other CSI/OSC sequence is stripped, and pathological
 * input falls back to stripped plain text (`src/lib/ansi.ts`). Inherits
 * the parent's font — put it inside a whitespace-preserving mono pane
 * (`TerminalStream` does).
 */
export function AnsiText({ text, className }: AnsiTextProps) {
  const children = useMemo(() => {
    // Keys are segment start offsets: content-derived and stable for a
    // given `text`, unlike array indices under the merge in parseAnsi.
    let offset = 0
    return parseAnsi(text).map((segment) => {
      const key = offset
      offset += segment.text.length
      if (!segment.tone && !segment.bold) return segment.text
      return (
        <span
          key={key}
          className={cn(
            segment.tone && toneClass[segment.tone],
            segment.bold && 'font-semibold',
          )}
        >
          {segment.text}
        </span>
      )
    })
  }, [text])

  return <span className={className}>{children}</span>
}
