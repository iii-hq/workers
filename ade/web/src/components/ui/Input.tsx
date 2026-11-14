import * as React from 'react'
import { Input as BaseInput } from '@/components/shadcn/input'
import { cn } from '@/lib/utils'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

export interface InputProps
  extends Omit<React.ComponentProps<typeof BaseInput>, 'onChange' | 'value'> {
  value: string
  /** The next value (the console's string-first API over the shadcn input). */
  onChange: (next: string) => void
  /**
   * @deprecated Inputs now preserve the entered case by default. Retained so
   * older injected UIs keep compiling while they remove the prop.
   */
  preserveCase?: boolean
}

/** The shadcn (Base UI) input with the console's touch size on phones. */
export const Input = React.forwardRef<HTMLInputElement, InputProps>(
  (
    { className, value, onChange, preserveCase: _preserveCase, ...rest },
    ref,
  ) => (
    <BaseInput
      ref={ref}
      value={value}
      onChange={(event) => onChange(event.currentTarget.value)}
      className={cn('h-12 font-sans sm:h-9', className)}
      {...rest}
    />
  ),
)
Input.displayName = 'Input'
