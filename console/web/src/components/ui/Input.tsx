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
   * @deprecated Inputs now preserve the entered case by default. Retained so
   * older injected UIs keep compiling while they remove the prop.
   */
  preserveCase?: boolean
}

export const Input = React.forwardRef<HTMLInputElement, InputProps>(
  (
    { className, value, onChange, preserveCase: _preserveCase, ...rest },
    ref,
  ) => (
    <input
      ref={ref}
      value={value}
      onChange={(e) => onChange(e.currentTarget.value)}
      className={cn(
        'h-9 w-full rounded-sm border border-transparent bg-surface px-3 font-sans text-[13px] text-ink',
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
