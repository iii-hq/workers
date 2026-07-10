import type * as React from 'react'
import { useRef } from 'react'
import { cn } from '@/lib/utils'

interface ModeToggleOption<T extends string> {
  value: T
  label: React.ReactNode
  title?: string
}

/**
 * `tabs` (default): semantic tablist for view switching (top nav).
 * `radio`: semantic radiogroup for persistent preferences (theme). Radio
 * matches the "one persistent choice" meaning better than tab, which
 * implies switching between panels.
 */
type ModeToggleVariant = 'tabs' | 'radio'

interface ModeToggleProps<T extends string> {
  value: T
  onChange: (next: T) => void
  options: ModeToggleOption<T>[]
  className?: string
  variant?: ModeToggleVariant
  /** Accessible name for the group. Required for `variant="radio"`. */
  'aria-label'?: string
}

export function ModeToggle<T extends string>({
  value,
  onChange,
  options,
  className,
  variant = 'tabs',
  'aria-label': ariaLabel,
}: ModeToggleProps<T>) {
  const groupRole = variant === 'radio' ? 'radiogroup' : 'tablist'
  const itemRole = variant === 'radio' ? 'radio' : 'tab'
  const buttonRefs = useRef<(HTMLButtonElement | null)[]>([])

  // WAI-ARIA radiogroup pattern: arrow keys move between options AND
  // update the selected value (radios commit on focus). Home/End jump
  // to first/last. Tab focus is roving — only the selected radio is
  // tabbable; others get tabIndex={-1} so Tab moves past the whole
  // group as one stop. Tabs variant keeps Tab-between-options.
  function handleRadioKeyDown(
    e: React.KeyboardEvent<HTMLButtonElement>,
    idx: number,
  ) {
    let nextIdx: number | null = null
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown') {
      nextIdx = (idx + 1) % options.length
    } else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') {
      nextIdx = (idx - 1 + options.length) % options.length
    } else if (e.key === 'Home') {
      nextIdx = 0
    } else if (e.key === 'End') {
      nextIdx = options.length - 1
    }
    if (nextIdx !== null) {
      e.preventDefault()
      onChange(options[nextIdx].value)
      buttonRefs.current[nextIdx]?.focus()
    }
  }

  return (
    // biome-ignore lint/a11y/useAriaPropsSupportedByRole: groupRole is always "tablist" or "radiogroup", both of which support aria-label; Biome can't resolve the dynamic role and falls back to the generic <div> role
    <div
      role={groupRole}
      aria-label={ariaLabel}
      className={cn('inline-flex border border-rule p-[2px]', className)}
    >
      {options.map((opt, idx) => {
        const active = opt.value === value
        const ariaState =
          variant === 'radio'
            ? { 'aria-checked': active }
            : { 'aria-selected': active }
        const isRadio = variant === 'radio'
        return (
          <button
            key={opt.value}
            type="button"
            role={itemRole}
            {...ariaState}
            ref={(el) => {
              buttonRefs.current[idx] = el
            }}
            tabIndex={isRadio ? (active ? 0 : -1) : undefined}
            onClick={() => onChange(opt.value)}
            onKeyDown={isRadio ? (e) => handleRadioKeyDown(e, idx) : undefined}
            title={opt.title}
            className={cn(
              'font-mono text-[13px] px-3 py-1 transition-colors lowercase',
              active
                ? 'bg-ink text-bg'
                : 'bg-transparent text-ink-faint hover:text-ink',
            )}
          >
            {opt.label}
          </button>
        )
      })}
    </div>
  )
}
