import * as React from 'react'
import { cn } from '@/lib/utils'

interface InputProps
  extends Omit<
    React.InputHTMLAttributes<HTMLInputElement>,
    'onChange' | 'value'
  > {
  value: string
  onChange: (next: string) => void
  /**
   * Opt out of the default `lowercase` text-transform. Use for fields that
   * carry verbatim user input (API keys, URLs with mixed case) where visual
   * lowercasing would mislead even though the underlying value is preserved.
   */
  preserveCase?: boolean
}

export const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, value, onChange, preserveCase, ...rest }, ref) => (
    <input
      ref={ref}
      value={value}
      onChange={(e) => onChange(e.currentTarget.value)}
      className={cn(
        'w-full rounded-sm border border-transparent bg-surface px-3 h-9 text-ink font-mono text-[13px]',
        !preserveCase && 'lowercase',
        'placeholder:text-ink-ghost',
        'hover:bg-surface-hover focus:outline-none focus:border-rule-focus focus:ring-[3px] focus:ring-accent/10 transition-[border-color,box-shadow,background-color]',
        'disabled:opacity-40 disabled:pointer-events-none',
        className,
      )}
      {...rest}
    />
  ),
)
Input.displayName = 'Input'
