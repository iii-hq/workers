import type * as React from 'react'
import { Badge } from '@/components/ui/Badge'
import { Prompt } from '@/components/ui/Prompt'
import { cn } from '@/lib/utils'

/** A small mono pill used for header chips (image / workdir / lang).
    Distinct from `Badge` (which is text-only) — Chip carries a key
    and value pair separated by a hair line. */
interface ChipProps {
  label?: string
  children: React.ReactNode
  className?: string
}

export function Chip({ label, children, className }: ChipProps) {
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1.5 border border-rule-2 bg-paper-2 px-1.5 py-0.5',
        'font-mono text-[11px] text-ink',
        className,
      )}
    >
      {label ? (
        <span className="text-ink-faint uppercase tracking-[0.06em]">
          {label}
        </span>
      ) : null}
      <span className="tabular-nums">{children}</span>
    </span>
  )
}

interface TerminalProps {
  /** Single-line command shown to the right of the `$` prompt. */
  command?: React.ReactNode
  /** Pulsing prompt + `triggering…` shimmer in place of stdout. */
  running?: boolean
  /** Optional chip row above the body. */
  chips?: React.ReactNode
  /** Body region — usually `<AnsiOutput>` or a code block. */
  children?: React.ReactNode
  /** Footer pill row (exit code, duration, timed-out). */
  footer?: React.ReactNode
  className?: string
}

/**
 * Shared terminal chrome. The exec card sets `command`, `chips`,
 * `<AnsiOutput>`, and `footer`. `sandbox::run` reuses the chrome
 * with an extra code-preview pane in the body slot. `fs::read`
 * borrows just the header + body styling (no command line).
 */
export function Terminal({
  command,
  running,
  chips,
  children,
  footer,
  className,
}: TerminalProps) {
  return (
    <div className={cn('border-t border-rule-2', className)} data-keep-mono>
      {command !== undefined ? (
        <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-x-3 gap-y-1.5">
          <span className="font-mono text-[12.5px] text-ink whitespace-pre-wrap break-all">
            <Prompt symbol="$" className={running ? 'animate-pulse' : ''} />
            <span className="ml-2">{command}</span>
          </span>
          {chips ? (
            <span className="flex flex-wrap items-center gap-1.5 ml-auto">
              {chips}
            </span>
          ) : null}
        </div>
      ) : chips ? (
        <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
          {chips}
        </div>
      ) : null}

      {running ? (
        <div className="bg-bg px-3 py-3 font-mono text-[12.5px] leading-[1.55] text-ink-faint italic">
          triggering…
        </div>
      ) : (
        children
      )}

      {footer ? (
        <div className="bg-paper-2 border-t border-rule-2 px-3 py-1.5 flex flex-wrap items-center gap-2">
          {footer}
        </div>
      ) : null}
    </div>
  )
}

/** Standardised footer pill — like a Badge but always tabular-nums. */
interface FooterPillProps {
  tone?: 'accent' | 'warn' | 'alert' | 'default'
  children: React.ReactNode
}

export function FooterPill({ tone = 'default', children }: FooterPillProps) {
  return (
    <Badge variant={tone} className="tabular-nums normal-case tracking-normal">
      {children}
    </Badge>
  )
}
