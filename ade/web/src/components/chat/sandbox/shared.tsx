import type { ReactNode } from 'react'
import { Badge } from '@/components/ui/Badge'
import { cn } from '@/lib/utils'

export function Chip({
  children,
  className,
}: {
  children: ReactNode
  className?: string
}) {
  return (
    <span
      className={cn(
        'inline-flex items-center rounded-sm border border-rule-2 bg-paper-2 px-1.5 py-0.5 font-mono text-[10px] text-ink-faint',
        className,
      )}
    >
      {children}
    </span>
  )
}

export function MetaRow({ children }: { children: ReactNode }) {
  return (
    <div className="flex flex-wrap items-center gap-1.5 px-3 py-1.5 border-b border-rule-2 bg-paper-2">
      {children}
    </div>
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
    <Badge variant={variant} className="normal-case tracking-normal">
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
  const toneClass =
    tone === 'warn' ? 'text-warn' : tone === 'ink' ? 'text-ink' : 'text-accent'
  return (
    <div className="flex items-start gap-2 px-3 py-2 border-b border-rule-2 bg-bg">
      <span className={cn('font-mono font-semibold shrink-0', toneClass)}>
        {symbol}
      </span>
      <div className="min-w-0 text-ink break-all">{children}</div>
    </div>
  )
}
