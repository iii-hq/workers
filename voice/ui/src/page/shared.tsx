/**
 * Building blocks every voice section composes, settings-page style: a
 * titled group (heading and actions above, never inside, the panel), the
 * panel itself (one shadcn card, rows split by hairlines), the setting row
 * (label and hint on the left, the control on the right), a compact empty
 * row, loading skeleton rows and byte formatting.
 */

import { ShadcnCard, Skeleton } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { type ReactNode, useState } from 'react'

export function SectionCard({
  title,
  description,
  actions,
  children,
  className,
}: {
  title: ReactNode
  description?: ReactNode
  actions?: ReactNode
  children: ReactNode
  className?: string
}) {
  return (
    <section className={className ? `voice-group ${className}` : 'voice-group'}>
      <header className="voice-group-head">
        <div className="voice-group-heading">
          <h2 className="voice-group-title">{title}</h2>
          {description ? <p className="voice-group-description">{description}</p> : null}
        </div>
        {actions ? <div className="voice-group-actions">{actions}</div> : null}
      </header>
      <ShadcnCard className="voice-panel">{children}</ShadcnCard>
    </section>
  )
}

/** Setting rows inside a panel. */
export function Facts({ children }: { children: ReactNode }) {
  return <dl className="voice-rows">{children}</dl>
}

/** One setting: the name and what it does on the left, the control on the right. */
export function Fact({
  label,
  hint,
  children,
  stack,
}: {
  label: ReactNode
  hint?: ReactNode
  children?: ReactNode
  /** The control takes the full width under the label (a picker, a long value). */
  stack?: boolean
}) {
  return (
    <div className={stack ? 'voice-row voice-row-stack' : 'voice-row'}>
      <dt className="voice-row-text">
        <span className="voice-row-label">{label}</span>
        {hint ? <span className="voice-row-hint">{hint}</span> : null}
      </dt>
      {children ? <dd className="voice-row-control">{children}</dd> : null}
    </div>
  )
}

/** A panel's one-line empty state: what is missing, and the way to fill it. */
export function PanelEmpty({
  title,
  description,
  action,
}: {
  title: ReactNode
  description?: ReactNode
  action?: ReactNode
}) {
  return (
    <div className="voice-empty">
      <div className="voice-row-text">
        <span className="voice-row-label">{title}</span>
        {description ? <span className="voice-row-hint">{description}</span> : null}
      </div>
      {action ? <div className="voice-row-control">{action}</div> : null}
    </div>
  )
}

/** Inner padding for free content (a transcript, a textarea) inside a panel. */
export function PanelBody({ children }: { children: ReactNode }) {
  return <div className="voice-panel-body">{children}</div>
}

const SKELETON_KEYS = ['one', 'two', 'three', 'four', 'five', 'six'] as const

export function LoadingRows({ rows = 3 }: { rows?: number }) {
  return (
    <output className="voice-skeletons" aria-busy="true" aria-label="Loading">
      {SKELETON_KEYS.slice(0, Math.min(rows, SKELETON_KEYS.length)).map((key) => (
        <Skeleton key={key} className="voice-skeleton" />
      ))}
    </output>
  )
}

export { formatBytes } from '@iii-dev/console-ui/format'
export { formatDuration } from '../lib/format'

export function useBusyAction(
  onNotice: (notice: { kind: 'error' | 'success'; text: string } | null) => void,
  onChanged: () => void,
): readonly [string | null, (id: string, work: Promise<unknown>, done: string) => void] {
  const [busy, setBusy] = useState<string | null>(null)
  const run = (id: string, work: Promise<unknown>, done: string) => {
    setBusy(id)
    onNotice(null)
    work
      .then(() => {
        onNotice({ kind: 'success', text: done })
        onChanged()
      })
      .catch((err: unknown) => onNotice({ kind: 'error', text: errorMessage(err) }))
      .finally(() => setBusy(null))
  }
  return [busy, run] as const
}
