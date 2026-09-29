/**
 * The chrome every sandbox::* family card shares: the labelled chip, the
 * sandbox-id copy/jump chip, the terminal frame (command line + streams +
 * footer) and the stdout/stderr pair. Streams render through the console's
 * `TerminalStream` (ANSI on, clamped) — the family carries no parser.
 */

import { Badge, Chip as UiChip, TerminalCommandLine, TerminalStream } from '@iii-dev/console-ui'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { uiClasses } from '@iii-dev/console-ui/ui-classes'
import type { ReactNode } from 'react'
import { jumpToSandbox } from '../lib/selection'
import type { ExitReason } from './format'
import { streamTruncatedAtCap, truncateMiddle } from './format'

export function cx(...parts: (string | false | null | undefined)[]): string {
  return parts.filter(Boolean).join(' ')
}

export function Chip({
  label,
  children,
  tone,
}: {
  label?: string
  children: ReactNode
  tone?: 'neutral' | 'warning'
}) {
  return (
    <UiChip tone={tone}>
      {label ? <span className={uiClasses.eyebrow}>{label}</span> : null}
      <span className="num">{children}</span>
    </UiChip>
  )
}

export function ExitReasonPill({ reason }: { reason: ExitReason }) {
  return <Badge variant={reason.tone}>{reason.label}</Badge>
}

export function SandboxIdChip({
  sandboxId,
  label = 'sandbox',
  jump = true,
}: {
  sandboxId: string
  label?: string
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
        <span className={uiClasses.eyebrow}>{label}</span>
        <span className="num">{truncateMiddle(sandboxId, 14)}</span>
        {state === 'idle' ? null : (
          <span className={uiClasses.eyebrow}>{state === 'copied' ? 'copied' : 'copy failed'}</span>
        )}
      </button>
      {jump ? (
        <button
          type="button"
          className={cx('cr-fam-sbx-open', uiClasses.eyebrow)}
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
  command?: string
  running?: boolean
  chips?: ReactNode
  children?: ReactNode
  footer?: ReactNode
}

export function Terminal({ command, running, chips, children, footer }: TerminalProps) {
  return (
    <div className="cr-fam-card">
      {command !== undefined ? (
        <TerminalCommandLine command={command} chips={chips} className="cr-fam-term-head" />
      ) : chips ? (
        <div className="cr-fam-chips-row">{chips}</div>
      ) : null}

      {running ? <div className={cx('cr-fam-running', uiClasses.pulse)}>triggering…</div> : children}

      {footer ? <div className="cr-fam-foot">{footer}</div> : null}
    </div>
  )
}

/* --- stdout / stderr ------------------------------------------------------ */

function Stream({ text, tone }: { text: string; tone: 'out' | 'err' }) {
  return (
    <div className="cr-fam-stream">
      {streamTruncatedAtCap(text) ? <Chip tone="warning">output truncated at 1 MiB</Chip> : null}
      <TerminalStream
        label={tone === 'err' ? 'stderr' : 'stdout'}
        text={text}
        tone={tone}
        ansi
        className="cr-fam-stream-pane"
      />
    </div>
  )
}

export function Streams({ stdout, stderr }: { stdout?: string; stderr?: string }) {
  if (!stdout && !stderr) {
    return <div className="cr-fam-note-ghost">· no output</div>
  }
  return (
    <div className="cr-fam-streams">
      {stdout ? <Stream text={stdout} tone="out" /> : null}
      {stderr ? <Stream text={stderr} tone="err" /> : null}
    </div>
  )
}
