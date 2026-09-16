import { Search, X } from 'lucide-react'
import * as React from 'react'
import { cn } from '@/lib/utils'
import { Input } from './Input'

export interface SearchFieldProps {
  value: string
  onChange: (next: string) => void
  placeholder?: string
  /** Visible label above the field; otherwise pass `aria-label`. */
  label?: React.ReactNode
  'aria-label'?: string
  autoFocus?: boolean
  className?: string
  id?: string
  name?: string
  onKeyDown?: React.KeyboardEventHandler<HTMLInputElement>
}

/**
 * The search box: leading magnifier, clear button while non-empty, Escape
 * clears (and stops there, so an enclosing dialog stays open).
 */
export const SearchField = React.forwardRef<HTMLInputElement, SearchFieldProps>(
  (
    {
      value,
      onChange,
      placeholder,
      label,
      'aria-label': ariaLabel,
      autoFocus,
      className,
      id,
      name,
      onKeyDown,
    },
    ref,
  ) => {
    const generatedId = React.useId()
    const inputId = id ?? generatedId
    return (
      <div className={cn('flex min-w-0 flex-col gap-1', className)}>
        {label ? (
          <label
            htmlFor={inputId}
            className="font-sans text-xs font-medium text-ink-faint"
          >
            {label}
          </label>
        ) : null}
        <div className="relative">
          <Search
            aria-hidden
            className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-ink-ghost"
          />
          <Input
            ref={ref}
            id={inputId}
            name={name}
            type="search"
            value={value}
            onChange={onChange}
            placeholder={placeholder}
            aria-label={label ? undefined : (ariaLabel ?? placeholder)}
            autoFocus={autoFocus}
            className="pl-8 pr-8 [&::-webkit-search-cancel-button]:hidden"
            onKeyDown={(event) => {
              onKeyDown?.(event)
              if (event.defaultPrevented) return
              if (event.key === 'Escape' && value) {
                event.stopPropagation()
                onChange('')
              }
            }}
          />
          {value ? (
            <button
              type="button"
              aria-label="Clear"
              onClick={() => onChange('')}
              className="absolute right-1.5 top-1/2 flex size-6 -translate-y-1/2 cursor-pointer items-center justify-center rounded-sm text-ink-ghost hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
            >
              <X className="size-4" aria-hidden />
            </button>
          ) : null}
        </div>
      </div>
    )
  },
)
SearchField.displayName = 'SearchField'
