import { Search, X } from 'lucide-react'
import * as React from 'react'
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from '@/components/shadcn/input-group'
import { cn } from '@/lib/utils'

// viewport: phone chrome — the sm utilities here are the console's
// phone-vs-desktop presentation (48px touch height, 16px text), not pane
// layout; see viewport-breakpoint-conformance.test.ts.

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
 * The search box on the shadcn input group: leading magnifier, clear
 * button while non-empty, Escape clears (and stops there, so an enclosing
 * dialog stays open).
 *
 * A dialog listens for Escape on the document before any React handler
 * runs, so stopping the event in `onKeyDown` comes too late. The field
 * claims the key on the window, earlier still, while there is a query to
 * clear; `preventDefault` is what tells the dialog to stay (see `Dialog`).
 * The clear itself still happens in `onKeyDown`, after the caller's own
 * handler, which can veto it with `preventDefault` as before.
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
    const claimed = React.useRef<Event | null>(null)
    React.useEffect(() => {
      const node = input.current
      const view = node?.ownerDocument.defaultView
      if (!node || !view) return
      const claim = (event: KeyboardEvent) => {
        if (event.key !== 'Escape' || event.target !== node || !node.value)
          return
        event.preventDefault()
        claimed.current = event
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
            className="text-xs font-medium text-muted-foreground"
          >
            {label}
          </label>
        ) : null}
        <InputGroup className="h-12 sm:h-9">
          <InputGroupAddon>
            <Search aria-hidden className="text-ink-ghost" />
          </InputGroupAddon>
          <InputGroupInput
            ref={setRef}
            {...rest}
            id={inputId}
            type="search"
            value={value}
            onChange={(event) => onChange(event.currentTarget.value)}
            placeholder={placeholder}
            aria-label={label ? undefined : (ariaLabel ?? placeholder)}
            className="h-full text-base sm:text-sm [&::-webkit-search-cancel-button]:hidden"
            onKeyDown={(event) => {
              // A claimed Escape arrives already default-prevented (that is
              // what kept the dialog open), so the caller's veto is its own
              // preventDefault call, watched here.
              const ours = event.nativeEvent === claimed.current
              let vetoed = false
              if (ours) {
                const prevent = event.preventDefault.bind(event)
                event.preventDefault = () => {
                  vetoed = true
                  prevent()
                }
              }
              onKeyDown?.(event)
              if (ours ? vetoed : event.defaultPrevented) return
              if (event.key === 'Escape' && value) {
                event.stopPropagation()
                onChange('')
              }
            }}
          />
          {value ? (
            <InputGroupAddon align="inline-end">
              <InputGroupButton
                size="icon-xs"
                aria-label="Clear search"
                onClick={() => {
                  input.current?.focus({ preventScroll: true })
                  onChange('')
                }}
              >
                <X aria-hidden />
              </InputGroupButton>
            </InputGroupAddon>
          ) : null}
        </InputGroup>
      </div>
    )
  },
)
SearchField.displayName = 'SearchField'
