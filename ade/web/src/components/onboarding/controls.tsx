import { Check } from 'lucide-react'
import type { ComponentProps, InputHTMLAttributes, ReactNode } from 'react'
import { Button as SharedButton } from '@/components/ui/Button'
import { cn } from '@/lib/utils'

type SetupButtonVariant = 'primary' | 'outline' | 'ghost'

/**
 * Setup's three pill buttons on the shared base: `primary` (solid ink) for
 * the one action that moves on, `outline` (a soft neutral fill, no stroke)
 * for going back, `ghost` for skipping. All three share one 36px height, a
 * semibold label, a focus ring with an offset so it reads on the solid
 * primary too, and a 0.96 press.
 */
export function Button({
  className,
  variant = 'primary',
  size,
  ...props
}: Omit<ComponentProps<typeof SharedButton>, 'variant'> & {
  variant?: SetupButtonVariant
}) {
  return (
    <SharedButton
      {...props}
      variant={variant === 'primary' ? 'primary' : 'ghost'}
      size={size}
      className={cn(
        'h-9 rounded-xl px-6 text-[13px] font-semibold tracking-[-0.005em] transition-[background-color,color,border-color,transform] duration-150 ease-[var(--motion-ease-standard)] active:scale-[0.96] focus-visible:ring-2 focus-visible:ring-rule-focus focus-visible:ring-offset-2 focus-visible:ring-offset-white motion-reduce:active:scale-100 dark:focus-visible:ring-offset-neutral-950',
        size === 'sm' && 'h-8 px-3 text-xs',
        // Fills step through the console's surface ramp so a hover is the
        // same one-step change in both themes, on any base.
        variant === 'primary' && 'bg-ink text-bg hover:bg-ink/85',
        variant === 'outline' &&
          'bg-neutral-100 text-ink hover:bg-neutral-200 hover:text-ink dark:bg-neutral-800 dark:hover:bg-neutral-700',
        variant === 'ghost' &&
          'px-4 text-ink-faint hover:bg-surface-hover hover:text-ink',
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
