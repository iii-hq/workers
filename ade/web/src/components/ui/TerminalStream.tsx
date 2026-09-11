import { useState } from 'react'
import { AnsiText } from '@/components/ui/AnsiText'
import { cn } from '@/lib/utils'

/** Lines of a stream shown before it collapses behind the toggle. */
const DEFAULT_CLAMP_LINES = 12
/** …and a character ceiling, for the one 400 KB line a minifier emits. */
const DEFAULT_CLAMP_CHARS = 2000

interface ClampedStream {
  /** The full body, one trailing newline trimmed. */
  full: string
  /** What a collapsed pane shows. */
  shown: string
  clamped: boolean
  totalLines: number
}

/** Pure clamp behind `TerminalStream`; exported for its unit tests. */
export function clampStream(
  text: string,
  maxLines: number,
  maxChars: number,
): ClampedStream {
  const full = text.endsWith('\n') ? text.slice(0, -1) : text
  const lines = full.split('\n')
  const clamped = lines.length > maxLines || full.length > maxChars
  const shown = clamped
    ? lines.slice(0, maxLines).join('\n').slice(0, maxChars)
    : full
  return { full, shown, clamped, totalLines: lines.length }
}

interface TerminalStreamProps {
  /** Pane label (`stdout`, `stderr`, `build`), rendered uppercase. */
  label: string
  /** The stream body; renders nothing when empty — the caller decides
      what "no output" should say, if anything. */
  text: string
  /** `err` tints the body warn, and nothing more: stderr is the user's
      program failing, not the console failing. */
  tone?: 'out' | 'err'
  /** Parse ANSI SGR colors in the body (`AnsiText`); plain otherwise. */
  ansi?: boolean
  clampLines?: number
  clampChars?: number
  className?: string
}

/**
 * Labeled monospace stream pane — the shared rendering for stdout /
 * stderr / log bodies in terminal-shaped cards. Whitespace is preserved,
 * long output collapses behind an `expand · N lines` toggle (the clamp
 * caps what is in the DOM at all), and the pane scrolls within itself —
 * never page-wide.
 */
export function TerminalStream({
  label,
  text,
  tone = 'out',
  ansi,
  clampLines = DEFAULT_CLAMP_LINES,
  clampChars = DEFAULT_CLAMP_CHARS,
  className,
}: TerminalStreamProps) {
  const [expanded, setExpanded] = useState(false)
  if (text.length === 0) return null

  const { full, shown, clamped, totalLines } = clampStream(
    text,
    clampLines,
    clampChars,
  )
  const body = expanded ? full : shown

  return (
    <div className={cn('min-w-0', className)}>
      <div className="font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
        {label}
      </div>
      <pre
        className={cn(
          'm-0 mt-1 max-h-[480px] overflow-y-auto whitespace-pre-wrap break-words font-mono text-[12.5px] leading-[1.55]',
          tone === 'err' ? 'text-warn' : 'text-ink',
        )}
      >
        <code>{ansi ? <AnsiText text={body} /> : body}</code>
      </pre>
      {clamped ? (
        <button
          type="button"
          onClick={() => setExpanded((v) => !v)}
          className="mt-1 cursor-pointer font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint transition-colors hover:text-ink"
        >
          {expanded ? 'collapse' : `expand · ${totalLines} lines`}
        </button>
      ) : null}
    </div>
  )
}
