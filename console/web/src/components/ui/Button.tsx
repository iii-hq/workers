import { Slot } from '@radix-ui/react-slot'
import { cva, type VariantProps } from 'class-variance-authority'
import * as React from 'react'
import { cn } from '@/lib/utils'

const buttonVariants = cva(
  'inline-flex items-center justify-center gap-x-2 whitespace-nowrap font-mono lowercase rounded-none transition-[background-color,color,border-color] duration-150 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent disabled:pointer-events-none disabled:opacity-40 select-none',
  {
    variants: {
      variant: {
        primary: 'bg-ink text-bg border border-ink hover:bg-bg hover:text-ink',
        ghost:
          'bg-transparent text-ink border border-transparent hover:bg-ink hover:text-bg',
        pill: 'bg-bg text-ink border border-ink hover:bg-ink hover:text-bg',
        icon: 'bg-bg text-ink-faint border border-rule hover:text-ink',
        terminal: 'bg-bg text-ink border border-rule justify-start',
        wiggle:
          'wiggle bg-ink text-bg border border-ink hover:bg-bg hover:text-ink relative',
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
