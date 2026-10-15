import { Search, X } from 'lucide-react'
import * as React from 'react'
import { cn } from '@/lib/utils'
import { Input } from './Input'

export interface SearchFieldProps
  extends Omit<
    React.InputHTMLAttributes<HTMLInputElement>,
    'value' | 'onChange' | 'type' | 'className'
  > {
  value: string
  onChange: (next: string) => void
  /** Visible label above the field; otherwise pass `aria-label`. */
  label?: React.ReactNode
  /** Applies to the wrapper; every other prop (`data-*`, `aria-*`, `autoFocus`…) lands on the input. */
  className?: string
}

/**
 * The search box: leading magnifier, clear button while non-empty, Escape
 * clears (and stops there, so an enclosing dialog stays open).
 *
 * A Radix Dialog or BottomSheet listens for Escape on the document in the
 * capture phase, before any React handler runs, so stopping the event in
 * `onKeyDown` comes too late: the dialog has already closed. The field
 * claims the key on the window, earlier still, while there is a query to
 * clear; `preventDefault` is what tells the dialog to stay.
 */
export const SearchField = React.forwardRef<HTMLInputElement, SearchFieldProps>(
  (
    {
      value,
      onChange,
      placeholder,
      label,
      'aria-label': ariaLabel,
      className,
      id,
      onKeyDown,
      ...rest
    },
    ref,
  ) => {
    const generatedId = React.useId()
    const inputId = id ?? generatedId
    const input = React.useRef<HTMLInputElement | null>(null)
    const clear = React.useRef(onChange)
    clear.current = onChange
    const claimed = React.useRef<Event | null>(null)
    React.useEffect(() => {
      const node = input.current
      const view = node?.ownerDocument.defaultView
      if (!node || !view) return
      const claim = (event: KeyboardEvent) => {
        if (event.key !== 'Escape' || event.target !== node || !node.value) return
        event.preventDefault()
        claimed.current = event
        clear.current('')
      }
      view.addEventListener('keydown', claim, true)
      return () => view.removeEventListener('keydown', claim, true)
    }, [])
    const setRef = React.useCallback(
      (node: HTMLInputElement | null) => {
        input.current = node
        if (typeof ref === 'function') ref(node)
        else if (ref) ref.current = node
      },
      [ref],
    )
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
            ref={setRef}
            {...rest}
            id={inputId}
            type="search"
            value={value}
            onChange={onChange}
            placeholder={placeholder}
            aria-label={label ? undefined : (ariaLabel ?? placeholder)}
            className="pl-8 pr-8 [&::-webkit-search-cancel-button]:hidden"
            onKeyDown={(event) => {
              // Already cleared on the way down; keep it from React parents.
              if (event.nativeEvent === claimed.current) {
                event.stopPropagation()
                return
              }
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
              onClick={(event) => {
                event.currentTarget.parentElement
                  ?.querySelector('input')
                  ?.focus({ preventScroll: true })
                onChange('')
              }}
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
