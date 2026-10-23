// Small shared pieces of the analysis detail: the status dot, the pill, the
// section head, inline code in model-written text, and the disclosure row.
import { Badge, CollapsibleCard, CollapsibleCardContent, CollapsibleCardTrigger, StatusDot } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { Fragment, useState } from 'react'
import type { Tone } from '../../../model'

export type DotTone = 'accent' | 'ok' | 'warn' | 'alert' | 'ink' | 'ghost'

/** The 6 px dot. `ghost` is the quiet "not validated / not reached" mark. */
export function Dot({ tone, pulse }: { tone: DotTone; pulse?: boolean }) {
  if (tone === 'ghost') return <span className="eval-ui-ad-dot" aria-hidden="true" />
  return <StatusDot tone={tone} pulse={pulse} />
}

const BADGE: Record<Tone, 'default' | 'ok' | 'warn' | 'alert' | 'accent'> = {
  ok: 'ok',
  accent: 'accent',
  warn: 'warn',
  alert: 'alert',
  neutral: 'default',
}

const TONE_DOT: Record<Tone, DotTone> = {
  ok: 'ok',
  accent: 'accent',
  warn: 'warn',
  alert: 'alert',
  neutral: 'ghost',
}

/** A status pill: dot + words, one word or a short phrase. */
export function Pill({
  tone = 'neutral',
  strong,
  pulse,
  children,
}: {
  tone?: Tone
  /** The neutral emphasis: surface-selected fill, ink text (a verdict, not a state). */
  strong?: boolean
  pulse?: boolean
  children: ReactNode
}) {
  return (
    <Badge variant={BADGE[tone]} className="eval-ui-ad-pill" data-strong={strong || undefined}>
      <Dot tone={strong ? 'ink' : TONE_DOT[tone]} pulse={pulse} />
      {children}
    </Badge>
  )
}

/** `Suggestions   1 of 3 max` — the heading row of a section. */
export function SectionHead({ id, title, meta, note }: { id: string; title: string; meta?: string; note?: string }) {
  return (
    <div className="eval-ui-ad-head">
      <h2 id={id} className="eval-ui-ad-h2">
        {title}
      </h2>
      {meta ? <span className="eval-ui-ad-mono-quiet">{meta}</span> : null}
      {note ? <span className="eval-ui-ad-head-note">{note}</span> : null}
    </div>
  )
}

/** Model-written text: `backticked` spans read as code, everything else is plain. */
export function Inline({ text }: { text: string }) {
  const parts = text.split(/(`[^`\n]+`)/)
  return (
    <>
      {parts.map((part, index) =>
        part.length > 2 && part.startsWith('`') && part.endsWith('`') ? (
          <code key={index} className="eval-ui-ad-code">
            {part.slice(1, -1)}
          </code>
        ) : (
          <Fragment key={index}>{part}</Fragment>
        ),
      )}
    </>
  )
}

/** A disclosure row over the shared collapsible card, closed unless `defaultOpen`. */
export function Disclosure({
  summary,
  children,
  className,
  defaultOpen = false,
}: {
  summary: (open: boolean) => ReactNode
  children: ReactNode
  className?: string
  defaultOpen?: boolean
}) {
  const [open, setOpen] = useState(defaultOpen)
  return (
    <CollapsibleCard open={open} onOpenChange={setOpen} className={`eval-ui-ad-disclosure ${className ?? ''}`.trim()}>
      <CollapsibleCardTrigger className="eval-ui-ad-disclosure-trigger">{summary(open)}</CollapsibleCardTrigger>
      <CollapsibleCardContent>{children}</CollapsibleCardContent>
    </CollapsibleCard>
  )
}
