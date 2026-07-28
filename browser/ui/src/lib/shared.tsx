/**
 * Terminal-card primitives shared by the `browser::*` function-trigger views.
 * Ported from the console's sandbox `shared.tsx`; Tailwind utility classes are
 * replaced with scoped `br-ui-*` classes (see styles.css) and `Badge` comes
 * from the console's shared component library.
 */

import { Badge } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { cn } from './cn'

export function Chip({
  children,
  className,
}: {
  children: ReactNode
  className?: string
}) {
  return <span className={cn('br-ui-chip', className)}>{children}</span>
}

export function MetaRow({ children }: { children: ReactNode }) {
  return <div className="br-ui-meta-row">{children}</div>
}

export function StatusPill({
  label,
  variant = 'default',
}: {
  label: string
  variant?: 'default' | 'warn' | 'alert' | 'accent'
}) {
  return (
    <Badge variant={variant} className="br-ui-pill">
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
  children: ReactNode
  tone?: 'accent' | 'warn' | 'ink'
}) {
  return (
    <div className="br-ui-action-line">
      <span className={cn('br-ui-action-sym', `tone-${tone}`)}>{symbol}</span>
      <div className="br-ui-action-body">{children}</div>
    </div>
  )
}
