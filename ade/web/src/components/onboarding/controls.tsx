import { Check } from 'lucide-react'
import type { ComponentProps, InputHTMLAttributes, ReactNode } from 'react'
import { Button as SharedButton } from '@/components/shadcn/button'
import { cn } from '@/lib/utils'

type SetupButtonVariant = 'primary' | 'outline' | 'ghost'

const VARIANT = {
  primary: 'default',
  outline: 'secondary',
  ghost: 'ghost',
} as const

/**
 * Setup's buttons are the shared shadcn Button: `primary` for the one
 * action that moves on, `outline` (the shared secondary) for going back,
 * `ghost` for skipping. Setup sits on the 36px `lg` step; `sm` drops to the
 * shared default.
 */
export function Button({
  className,
  variant = 'primary',
  size,
  ...props
}: Omit<ComponentProps<typeof SharedButton>, 'variant' | 'size'> & {
  variant?: SetupButtonVariant
  size?: 'sm'
}) {
  return (
    <SharedButton
      {...props}
      variant={VARIANT[variant]}
      size={size === 'sm' ? 'default' : 'lg'}
      className={cn(
        variant === 'ghost' && 'text-ink-faint hover:text-ink',
        className,
      )}
    />
  )
}

/**
 * A 16px native checkbox: neutral when clear, ink when checked, with the
 * label as its hit target so a row toggles anywhere on its text.
 */
export function Checkbox({
  className,
  label,
  ...props
}: Omit<InputHTMLAttributes<HTMLInputElement>, 'type' | 'className'> & {
  className?: string
  label?: ReactNode
}) {
  return (
    <label
      className={cn(
        'inline-flex min-w-0 cursor-pointer items-center gap-2.5 text-[13px] has-disabled:cursor-not-allowed has-disabled:opacity-50',
        className,
      )}
    >
      <span className="relative flex size-4 shrink-0 items-center justify-center">
        <input
          {...props}
          type="checkbox"
          className="peer size-4 cursor-inherit appearance-none rounded-[4px] border border-neutral-300 bg-white shadow-xs transition-[background-color,border-color] duration-150 checked:border-neutral-900 checked:bg-neutral-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus focus-visible:ring-offset-2 focus-visible:ring-offset-white disabled:cursor-not-allowed dark:border-neutral-600 dark:bg-neutral-950 dark:checked:border-neutral-100 dark:checked:bg-neutral-100 dark:focus-visible:ring-offset-neutral-950"
        />
        <Check
          className="pointer-events-none absolute size-4 p-px scale-25 text-white opacity-0 transition-[opacity,transform] duration-150 ease-[var(--motion-ease-standard)] peer-checked:scale-100 peer-checked:opacity-100 motion-reduce:transition-none dark:text-neutral-950"
          strokeWidth={3}
          aria-hidden
        />
      </span>
      {label != null ? <span className="min-w-0 flex-1">{label}</span> : null}
    </label>
  )
}
