import type { HTMLAttributes } from 'react'
import { cn } from '@/lib/utils'

/**
 * A key cap. The lift elevation turned upside down (`--shadow-keycap`: the
 * lit edge along the bottom, the drops cast upward), so a cap reads as a key
 * set into the surface rather than a card floating on it. One cap names a
 * key in prose; `KeyCombo` composes a chord out of them.
 */
export function Kbd({ className, ...props }: HTMLAttributes<HTMLElement>) {
  return (
    <kbd
      className={cn(
        // `rounded` is the bare 4px step: at the system's 6px a cap this
        // small turns into a pill.
        'inline-flex min-w-5 items-center justify-center rounded bg-surface px-1 py-px font-mono text-[0.65rem] text-ink-ghost shadow-keycap',
        className,
      )}
      {...props}
    />
  )
}
