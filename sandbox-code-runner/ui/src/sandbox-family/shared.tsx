/**
 * Shared chrome for the sandbox family cards: the terminal frame, the
 * key/value chips, the verdict pills, and the sandbox-id chip.
 *
 * Ported from the console's sandbox family (terminal/Terminal.tsx +
 * shared.tsx), converted to the worker-UI standard: components come
 * from `@iii-dev/console-ui` or live here, styling is `cr-fam-*`
 * classes in styles/family.css (design tokens only, no Tailwind).
 *
 * Daemon sandbox ids are NOT redacted in this family — deliberate
 * design decision: `sandbox_id` is the operator-visible address of a
 * fleet-page row, not a capability like the worker's own `runtime_id`.
 * The upgrade over the console version is that every sandbox-id chip
 * is now interactive: the id text copies on click, and the `open`
 * affordance jumps to the fleet page with that sandbox selected.
 */

import type { ReactNode } from 'react'
import { useCopyFlash } from '../lib/clipboard'
import { jumpToSandbox } from '../lib/selection'
import type { ExitReason } from './format'
import { truncateMiddle } from './format'

export function cx(...parts: (string | false | null | undefined)[]): string {
  return parts.filter(Boolean).join(' ')
}

/** A small mono chip carrying an optional key and a value. */
export function Chip({ label, children, className }: { label?: string; children: ReactNode; className?: string }) {
  return (
    <span className={cx('cr-fam-chip', className)}>
      {label ? <span className="k">{label}</span> : null}
      <span className="v">{children}</span>
    </span>
  )
}

/** Standardised footer pill — tabular-nums, toned. */
export function FooterPill({
  tone = 'default',
  children,
}: {
  tone?: 'ok' | 'accent' | 'warn' | 'alert' | 'default'
  children: ReactNode
}) {
  return <span className={cx('cr-fam-pill', tone)}>{children}</span>
}

/** The single per-card verdict (upgrade §4.1.1). */
export function ExitReasonPill({ reason }: { reason: ExitReason }) {
  return <FooterPill tone={reason.tone}>{reason.label}</FooterPill>
}

/**
 * The sandbox-id chip: two keyboard-accessible buttons in one chip.
 * The id text copies the full id (the chip shows a truncated form);
 * `open` navigates to the fleet page with this sandbox selected.
 */
export function SandboxIdChip({
  sandboxId,
  label = 'sandbox',
  jump = true,
}: {
  sandboxId: string
  label?: string
  /** `false` for cards where jumping makes no sense (a stopped sandbox). */
  jump?: boolean
}) {
  const { state, copy } = useCopyFlash(sandboxId)

  return (
    <span className="cr-fam-sbx">
      <button
        type="button"
        className={cx('cr-fam-sbx-copy', state !== 'idle' && state)}
        onClick={copy}
        title={state === 'copied' ? `copied ${sandboxId}` : `copy ${sandboxId}`}
        aria-label={`copy sandbox id ${sandboxId}`}
      >
        <span className="k">{label}</span>
        <span className="v">{truncateMiddle(sandboxId, 14)}</span>
        {state === 'idle' ? null : (
          <span className="cr-fam-sbx-flash">{state === 'copied' ? 'copied' : 'copy failed'}</span>
        )}
      </button>
      {jump ? (
        <button
          type="button"
          className="cr-fam-sbx-open"
          onClick={() => jumpToSandbox(sandboxId)}
          title="open in the sandbox fleet page"
          aria-label={`open sandbox ${sandboxId} in the fleet page`}
        >
          open
        </button>
      ) : null}
    </span>
  )
}

/* --- terminal chrome ----------------------------------------------------- */

interface TerminalProps {
  /** Single-line command shown to the right of the `$` prompt. */
  command?: ReactNode
  /** Pulsing prompt + `triggering…` shimmer in place of stdout. */
  running?: boolean
  /** Optional chip row (right-aligned next to the command). */
  chips?: ReactNode
  /** Body region — usually `<AnsiOutput>` or a code block. */
  children?: ReactNode
  /** Footer pill row (exit verdict, duration). */
  footer?: ReactNode
}

/**
 * Shared terminal chrome. The exec card sets `command`, `chips`,
 * `<AnsiOutput>`, and `footer`. `sandbox::run` reuses the chrome with
 * an extra code-preview pane in the body slot. `fs::read` borrows just
 * the header + body styling (no command line).
 */
export function Terminal({ command, running, chips, children, footer }: TerminalProps) {
  return (
    <div className="cr-fam-card">
      {command !== undefined ? (
        <div className="cr-fam-term-head">
          <span className="cr-fam-cmd">
            <span className={cx('cr-fam-prompt', running && 'cr-fam-pulse')}>$</span> {command}
          </span>
          {chips ? <span className="cr-fam-chips-end">{chips}</span> : null}
        </div>
      ) : chips ? (
        <div className="cr-fam-chips-row">{chips}</div>
      ) : null}

      {running ? <div className="cr-fam-running">triggering…</div> : children}

      {footer ? <div className="cr-fam-foot">{footer}</div> : null}
    </div>
  )
}
