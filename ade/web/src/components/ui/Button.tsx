import { Slot } from '@radix-ui/react-slot'
import * as React from 'react'
import { buttonVariants as shadcnButtonVariants } from '@/components/shadcn/button'
import { cn } from '@/lib/utils'

/*
 * The legacy Radix-Slot button, kept for its public props (`asChild`,
 * `variant`, `size`) because console-ui hands it to the worker UIs. It owns
 * no look of its own: every variant paints with the shadcn `buttonVariants`,
 * so a console-ui `Button` and a shadcn `Button` are the same button.
 */

type LegacyVariant =
  | 'primary'
  | 'secondary'
  | 'destructive'
  | 'ghost'
  | 'pill'
  | 'icon'
  | 'terminal'
  | 'wiggle'
type LegacySize = 'sm' | 'md' | 'lg' | 'icon'

// `primary` / `secondary` / `destructive` are the three action looks; the
// rest are legacy names that resolve onto them (or onto icon-only ghost).
const VARIANT = {
  primary: 'default',
  secondary: 'secondary',
  destructive: 'destructive',
  wiggle: 'default',
  pill: 'secondary',
  terminal: 'secondary',
  ghost: 'ghost',
  icon: 'ghost',
} as const

// The legacy heights land on the nearest shadcn step: sm (32px) is the
// shadcn default, md and lg share shadcn lg.
const SIZE = {
  sm: 'default',
  md: 'lg',
  lg: 'lg',
  icon: 'icon',
} as const

const EXTRA: Partial<Record<LegacyVariant, string>> = {
  wiggle: 'wiggle relative',
  terminal: 'justify-start',
  // Icon chrome rests in the faint ink and lifts to full ink on hover.
  ghost: 'text-ink-faint',
  icon: 'text-ink-faint',
}

export function buttonVariants({
  variant = 'primary',
  size = 'md',
}: {
  variant?: LegacyVariant | null
  size?: LegacySize | null
} = {}): string {
  const v = variant ?? 'primary'
  const s = size ?? 'md'
  return cn(
    shadcnButtonVariants({ variant: VARIANT[v], size: SIZE[s] }),
    EXTRA[v],
    // Icon buttons keep their 30px square so toolbars do not reflow.
    s === 'icon' && 'size-[30px] p-0',
  )
}

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: LegacyVariant | null
  size?: LegacySize | null
  asChild?: boolean
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild, children, ...props }, ref) => {
    const Comp: React.ElementType = asChild ? Slot : 'button'
    return (
      <Comp
        ref={ref}
        data-slot="button"
        className={cn(buttonVariants({ variant, size }), className)}
        {...props}
      >
        {children}
      </Comp>
    )
  },
)
Button.displayName = 'Button'
