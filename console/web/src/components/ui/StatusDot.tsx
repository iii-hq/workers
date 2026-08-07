import type * as React from 'react'
import { cn } from '@/lib/utils'

type DotTone = 'accent' | 'alert' | 'warn' | 'ink' | 'ok'

const dotTone: Record<DotTone, string> = {
  accent: 'bg-accent',
  alert: 'bg-alert',
  warn: 'bg-warn',
  ink: 'bg-ink',
  ok: 'bg-ok',
}

interface StatusDotProps extends React.HTMLAttributes<HTMLSpanElement> {
  tone?: DotTone
  pulse?: boolean
}

export function StatusDot({
  tone = 'accent',
  pulse,
  className,
  ...props
}: StatusDotProps) {
  return (
    <span
      aria-hidden
      className={cn(
        'inline-block size-1.5 rounded-full shrink-0',
        dotTone[tone],
        pulse && 'pulse-dot',
        className,
      )}
      {...props}
    />
  )
}
