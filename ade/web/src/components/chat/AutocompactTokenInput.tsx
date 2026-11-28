import { type ComponentProps, forwardRef } from 'react'
import { cn } from '@/lib/utils'

export interface AutocompactTokenInputProps
  extends Omit<ComponentProps<'input'>, 'value' | 'onChange' | 'type'> {
  /** The text as typed: `400000`, `400k`, `1.2M` (`parseTokens`). */
  value: string
  onChange: (text: string) => void
  /** Enter, or leaving the field: the caller parses and writes. */
  onCommit?: () => void
  invalid?: boolean
}

/**
 * The auto-compact size field (chat-projects §3.3): mono, as wide as `1.2M`
 * needs, with the dialogs' field fill. The callers parse with
 * `parseTokens` and show "Between 20k and 2M tokens" under it.
 */
export const AutocompactTokenInput = forwardRef<
  HTMLInputElement,
  AutocompactTokenInputProps
>(function AutocompactTokenInput(
  { value, onChange, onCommit, invalid, className, onKeyDown, onBlur, ...rest },
  ref,
) {
  return (
    <input
      ref={ref}
      type="text"
      inputMode="decimal"
      autoComplete="off"
      spellCheck={false}
      value={value}
      aria-invalid={invalid ? true : undefined}
      onChange={(e) => onChange(e.currentTarget.value)}
      onKeyDown={(e) => {
        onKeyDown?.(e)
        if (e.defaultPrevented || !onCommit) return
        if (e.key === 'Enter') {
          e.preventDefault()
          onCommit()
        }
      }}
      onBlur={(e) => {
        onBlur?.(e)
        onCommit?.()
      }}
      className={cn(
        'h-9 w-24 rounded-lg bg-muted px-3 font-mono text-sm text-foreground tabular-nums ring-1 ring-transparent outline-none transition-shadow duration-(--motion-duration-control) placeholder:text-ink-ghost focus-visible:ring-ring/50 disabled:opacity-50 aria-invalid:ring-alert/60',
        className,
      )}
      {...rest}
    />
  )
})
