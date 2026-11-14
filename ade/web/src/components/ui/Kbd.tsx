import type { HTMLAttributes } from 'react'
import { Kbd as BaseKbd } from '@/components/shadcn/kbd'
import { cn } from '@/lib/utils'

/**
 * A key cap: the shadcn `Kbd` on the console's keycap elevation
 * (`--shadow-keycap`: the lit edge along the bottom, the drops cast upward),
 * so a cap reads as a key set into the surface rather than a card floating
 * on it. One cap names a key in prose; `KeyCombo` composes a chord out of
 * them.
 */
export function Kbd({ className, ...props }: HTMLAttributes<HTMLElement>) {
  return (
    <BaseKbd
      className={cn(
        'pointer-events-auto h-auto rounded-xs py-px font-mono text-meta font-normal text-ink-ghost shadow-keycap',
        className,
      )}
      {...props}
    />
  )
}
