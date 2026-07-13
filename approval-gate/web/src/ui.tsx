import * as DialogPrimitive from '@radix-ui/react-dialog'
import * as SelectPrimitive from '@radix-ui/react-select'
import { Slot } from '@radix-ui/react-slot'
import { cva, type VariantProps } from 'class-variance-authority'
import { type ClassValue, clsx } from 'clsx'
import { X } from 'lucide-react'
import * as React from 'react'
import { twMerge } from 'tailwind-merge'

function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

const buttonVariants = cva(
  'inline-flex items-center justify-center gap-x-2 whitespace-nowrap rounded-none font-mono lowercase transition-[background-color,color,border-color] duration-150 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-40 select-none',
  {
    variants: {
      variant: {
        primary: 'border border-ink bg-ink text-bg hover:bg-bg hover:text-ink',
        secondary: 'border border-rule bg-bg text-ink hover:border-ink',
        ghost:
          'border border-transparent bg-transparent text-ink hover:bg-ink hover:text-bg',
        link: 'border-0 bg-transparent p-0 text-ink-faint hover:text-ink',
      },
      size: {
        sm: 'h-8 px-3 text-[12px]',
        md: 'h-9 px-5 text-[13px]',
      },
    },
    defaultVariants: {
      variant: 'primary',
      size: 'sm',
    },
  },
)

interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild, ...props }, ref) => {
    const Component: React.ElementType = asChild ? Slot : 'button'
    return (
      <Component
        ref={ref}
        className={cn(buttonVariants({ variant, size }), className)}
        {...props}
      />
    )
  },
)
Button.displayName = 'Button'

interface SelectOption<T extends string> {
  value: T
  label: string
  title?: string
}

interface SelectProps<T extends string> {
  value: T | undefined
  options: SelectOption<T>[]
  onChange: (next: T) => void
  disabled?: boolean
  className?: string
  'aria-label'?: string
}

export function Select<T extends string>({
  value,
  options,
  onChange,
  disabled,
  className,
  'aria-label': ariaLabel,
}: SelectProps<T>) {
  const selected = options.find((option) => option.value === value)

  return (
    <SelectPrimitive.Root
      value={selected ? value : ''}
      onValueChange={(next) => onChange(next as T)}
      disabled={disabled}
    >
      <SelectPrimitive.Trigger
        aria-label={ariaLabel}
        className={cn(
          'inline-flex h-8 min-w-[92px] items-center justify-between gap-x-2 border border-rule bg-bg px-3 font-mono text-[12px] lowercase text-ink transition-colors focus:border-ink focus:outline-none data-[state=open]:border-ink disabled:pointer-events-none disabled:opacity-40',
          className,
        )}
      >
        <SelectPrimitive.Value>{selected?.label}</SelectPrimitive.Value>
        <SelectPrimitive.Icon asChild>
          <span aria-hidden className="text-ink-faint">
            <svg
              width="8"
              height="6"
              viewBox="0 0 8 6"
              fill="none"
              stroke="currentColor"
              strokeWidth="1"
              aria-hidden="true"
            >
              <path d="M1 1L4 5L7 1" />
            </svg>
          </span>
        </SelectPrimitive.Icon>
      </SelectPrimitive.Trigger>
      <SelectPrimitive.Portal>
        <SelectPrimitive.Content
          position="popper"
          sideOffset={4}
          className="z-[70] min-w-[var(--radix-select-trigger-width)] overflow-hidden border border-rule bg-bg font-mono text-[12px] lowercase text-ink shadow-lg"
        >
          <SelectPrimitive.Viewport className="p-1">
            {options.map((option) => (
              <SelectPrimitive.Item
                key={option.value}
                value={option.value}
                title={option.title}
                className="relative flex cursor-pointer select-none items-center py-1.5 pr-3 pl-7 outline-none data-[disabled]:pointer-events-none data-[highlighted]:bg-rule data-[highlighted]:text-ink data-[disabled]:opacity-40"
              >
                <SelectPrimitive.ItemIndicator className="absolute left-2 top-1/2 -translate-y-1/2">
                  <span aria-hidden>✓</span>
                </SelectPrimitive.ItemIndicator>
                <SelectPrimitive.ItemText>
                  {option.label}
                </SelectPrimitive.ItemText>
              </SelectPrimitive.Item>
            ))}
          </SelectPrimitive.Viewport>
        </SelectPrimitive.Content>
      </SelectPrimitive.Portal>
    </SelectPrimitive.Root>
  )
}

export const Dialog = DialogPrimitive.Root
export const DialogTrigger = DialogPrimitive.Trigger

export const DialogContent = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Content>
>(({ className, children, ...props }, ref) => (
  <DialogPrimitive.Portal>
    <DialogPrimitive.Overlay className="fixed inset-0 z-[60] bg-ink/40" />
    <DialogPrimitive.Content
      ref={ref}
      className={cn(
        'fixed left-1/2 top-1/2 z-[60] max-h-[85vh] w-[min(560px,calc(100vw-32px))] -translate-x-1/2 -translate-y-1/2 overflow-y-auto border border-ink bg-bg p-6 font-mono text-ink focus-visible:outline-none',
        className,
      )}
      {...props}
    >
      {children}
      <DialogPrimitive.Close className="absolute right-4 top-4 text-ink-faint transition-colors hover:text-ink focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring">
        <X size={14} />
        <span className="sr-only">close</span>
      </DialogPrimitive.Close>
    </DialogPrimitive.Content>
  </DialogPrimitive.Portal>
))
DialogContent.displayName = 'DialogContent'

export const DialogTitle = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Title>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Title>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Title
    ref={ref}
    className={cn('font-mono text-[13px] text-ink', className)}
    {...props}
  />
))
DialogTitle.displayName = 'DialogTitle'

export const DialogDescription = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Description>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Description>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Description
    ref={ref}
    className={cn('font-mono text-[12px] text-ink-faint', className)}
    {...props}
  />
))
DialogDescription.displayName = 'DialogDescription'
