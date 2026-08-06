/* Terminal chrome + small chips/pills shared by the shell renderers —
   ported from the console's sandbox terminal components when the shell
   function-trigger family moved into this worker's injected UI. Styling
   lives in ../../styles.css under the `shui-` classes (scoped to
   [data-iii-ui="shell"]); tone pills reuse the shared `Badge`. */

import { Badge } from '@iii-dev/console-ui'
import type * as React from 'react'

function cx(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(' ')
}

export function Prompt({
  symbol = '$',
  pulse,
}: {
  symbol?: string
  pulse?: boolean
}) {
  return <span className={cx('shui-prompt', pulse && 'pulse')}>{symbol}</span>
}

/** A small mono pill used for header chips (path / mode / cwd). Distinct
    from `Badge` (text-only) — Chip carries a key and value pair. */
interface ChipProps {
  label?: string
  children: React.ReactNode
  className?: string
}

export function Chip({ label, children, className }: ChipProps) {
  return (
    <span className={cx('shui-chip', className)}>
      {label ? <span className="k">{label}</span> : null}
      <span className="v">{children}</span>
    </span>
  )
}

/** Standardised footer pill — a Badge with pill-flat numerals. */
interface FooterPillProps {
  tone?: 'accent' | 'warn' | 'alert' | 'default'
  children: React.ReactNode
}

export function FooterPill({ tone = 'default', children }: FooterPillProps) {
  return (
    <Badge variant={tone} className="shui-pill-flat">
      {children}
    </Badge>
  )
}

export function StatusPill({
  label,
  variant = 'default',
}: {
  label: string
  variant?: 'default' | 'warn' | 'alert' | 'accent'
}) {
  return (
    <Badge variant={variant} className="shui-pill-flat">
      {label}
    </Badge>
  )
}

export function ActionLine({
  symbol,
  children,
  tone = 'accent',
}: {
  symbol: string
  children: React.ReactNode
  tone?: 'accent' | 'warn' | 'ink'
}) {
  return (
    <div className="shui-action-line">
      <span className={cx('sym', `tone-${tone}`)}>{symbol}</span>
      <div className="body">{children}</div>
    </div>
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
}

/**
 * Shared terminal chrome. The exec card sets `command`, `chips`,
 * `<AnsiOutput>`, and `footer`; status reuses the chrome for the full
 * job record.
 */
export function Terminal({
  command,
  running,
  chips,
  children,
  footer,
}: TerminalProps) {
  return (
    <div className="shui-card">
      {command !== undefined ? (
        <div className="shui-head">
          <span className="shui-cmd">
            <Prompt symbol="$" pulse={running} />
            <span className="cmd-text">{command}</span>
          </span>
          {chips ? <span className="shui-chips end">{chips}</span> : null}
        </div>
      ) : chips ? (
        <div className="shui-head">
          <span className="shui-chips">{chips}</span>
        </div>
      ) : null}

      {running ? <div className="shui-running">triggering…</div> : children}

      {footer ? <div className="shui-foot">{footer}</div> : null}
    </div>
  )
}

interface AnsiOutputProps {
  stdout?: string
  stderr?: string
}

/**
 * Stream-coloured stdout/stderr pane. Exec output is buffered, so stdout
 * stacks above stderr with distinct tones rather than interleaving by
 * timestamp (which the wire shape doesn't carry). ANSI escape parsing is
 * out of scope; the escape introducers are stripped so raw `\x1b[` bytes
 * never leak into the rendered text.
 */
export function AnsiOutput({ stdout, stderr }: AnsiOutputProps) {
  const hasStdout = !!stdout && stdout.length > 0
  const hasStderr = !!stderr && stderr.length > 0

  if (!hasStdout && !hasStderr) {
    return <div className="shui-empty">· no output</div>
  }

  return (
    <div className="shui-body">
      {hasStdout ? (
        <pre className="shui-pre out">
          <code>{stripAnsi(stdout ?? '')}</code>
        </pre>
      ) : null}
      {hasStdout && hasStderr ? <div className="shui-stream-gap" /> : null}
      {hasStderr ? (
        <pre className="shui-pre err">
          <code>{stripAnsi(stderr ?? '')}</code>
        </pre>
      ) : null}
    </div>
  )
}

/* Single-pass strip of CSI/OSC escape sequences. Keeps the visible text
   intact (newlines, tabs) so monospace alignment in tools like `ls -l`
   survives. 0x1B is the ANSI ESC introducer (CSI/OSC); 0x07 is the BEL
   terminator for OSC sequences. */
// biome-ignore lint/suspicious/noControlCharactersInRegex: stripping ANSI escape sequences.
const CSI_REGEX = /\x1b\[[0-9;?]*[ -/]*[@-~]/g
// biome-ignore lint/suspicious/noControlCharactersInRegex: stripping ANSI OSC (ESC ] … BEL) sequences.
const OSC_REGEX = /\x1b\][^\x07]*\x07/g

function stripAnsi(s: string): string {
  return s.replace(CSI_REGEX, '').replace(OSC_REGEX, '')
}
