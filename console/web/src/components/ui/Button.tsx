import { Slot } from '@radix-ui/react-slot'
import { cva, type VariantProps } from 'class-variance-authority'
import * as React from 'react'
import { cn } from '@/lib/utils'

const buttonVariants = cva(
  'inline-flex items-center justify-center gap-x-2 whitespace-nowrap font-mono lowercase rounded-sm transition-[background-color,color,border-color] duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40 select-none',
  {
    variants: {
      variant: {
        primary:
          'bg-ink text-bg border border-transparent hover:bg-ink/90 rounded-md',
        ghost:
          'bg-transparent text-ink-faint border border-transparent hover:bg-surface-hover hover:text-ink',
        pill: 'bg-surface text-ink border border-transparent hover:bg-surface-hover',
        icon: 'bg-transparent text-ink-faint border border-transparent hover:bg-surface-hover hover:text-ink',
        terminal:
          'bg-surface text-ink border border-transparent justify-start hover:bg-surface-hover',
        wiggle:
          'wiggle bg-ink text-bg border border-transparent hover:bg-ink/90 rounded-md relative',
      },
      size: {
        sm: 'h-8 px-3 text-[13px]',
        md: 'h-9 px-5 text-[13px]',
        lg: 'h-11 px-5 text-[14px]',
        icon: 'size-[30px] p-0',
      },
    },
    defaultVariants: {
      variant: 'primary',
      size: 'md',
    },
  },
)

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild, children, ...props }, ref) => {
    const Comp: React.ElementType = asChild ? Slot : 'button'
    return (
      <Comp
        ref={ref}
        className={cn(buttonVariants({ variant, size }), className)}
        {...props}
      >
        {children}
      </Comp>
    )
  },
)
Button.displayName = 'Button'

export { buttonVariants }
