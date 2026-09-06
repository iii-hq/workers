/**
 * Building blocks every voice section composes: the card with a titled
 * header row, the label/value facts grid, loading skeleton rows, and byte
 * formatting. All of it sits on the shared console components.
 */

import { Card, CardBody, CardHeader, Skeleton } from '@iii-dev/console-ui'
import { type ReactNode, useState } from 'react'
import { errorMessage } from '../lib/format'

export function SectionCard({
  title,
  actions,
  children,
  className,
}: {
  title: ReactNode
  actions?: ReactNode
  children: ReactNode
  className?: string
}) {
  return (
    <Card className={className ? `voice-card ${className}` : 'voice-card'}>
      <CardHeader className="voice-card-header">
        <span className="voice-card-title">{title}</span>
        {actions ? <span className="voice-card-actions">{actions}</span> : null}
      </CardHeader>
      <CardBody className="voice-card-body">{children}</CardBody>
    </Card>
  )
}

export function Facts({ children, wide }: { children: ReactNode; wide?: boolean }) {
  return <dl className={wide ? 'voice-facts voice-facts-wide' : 'voice-facts'}>{children}</dl>
}

export function Fact({ label, children }: { label: ReactNode; children: ReactNode }) {
  return (
    <div className="voice-fact">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  )
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

export { formatBytes, formatDuration } from '../lib/format'

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
