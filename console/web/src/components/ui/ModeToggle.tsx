import uiClasses from '@iii-dev/console-ui/ui-classes'
import type * as React from 'react'
import { useRef } from 'react'
import { cn } from '@/lib/utils'
import { TabIconSlot } from './TabIcon'

export interface SegmentedControlOption<T extends string> {
  value: T
  label: React.ReactNode
  title?: string
  /** Defaults to a semantic 16px icon inferred from `value`; `false` hides it. */
  icon?: React.ReactNode | false
}

/**
 * `tabs` (default): semantic tablist for view switching (top nav).
 * `radio`: semantic radiogroup for persistent preferences (theme). Radio
 * matches the "one persistent choice" meaning better than tab, which
 * implies switching between panels.
 */
export type SegmentedControlVariant = 'tabs' | 'radio'

export interface SegmentedControlProps<T extends string> {
  value: T
  onChange: (next: T) => void
  options: SegmentedControlOption<T>[]
  className?: string
  /** Migration seam for domain layout; shared state styling remains primary. */
  itemClassName?: string
  activeItemClassName?: string
  variant?: SegmentedControlVariant
  /** Accessible name for the group. Required for `variant="radio"`. */
  'aria-label'?: string
}

export function SegmentedControl<T extends string>({
  value,
  onChange,
  options,
  className,
  itemClassName,
  activeItemClassName,
  variant = 'tabs',
  'aria-label': ariaLabel,
}: SegmentedControlProps<T>) {
  const groupRole = variant === 'radio' ? 'radiogroup' : 'tablist'
  const itemRole = variant === 'radio' ? 'radio' : 'tab'
  const buttonRefs = useRef<(HTMLButtonElement | null)[]>([])

  // Both tabs and radios are one keyboard stop. Arrow keys move and commit;
  // Home/End jump to the edges. Radios additionally accept vertical arrows.
  function handleKeyDown(
    e: React.KeyboardEvent<HTMLButtonElement>,
    idx: number,
  ) {
    let nextIdx: number | null = null
    if (
      e.key === 'ArrowRight' ||
      (variant === 'radio' && e.key === 'ArrowDown')
    ) {
      nextIdx = (idx + 1) % options.length
    } else if (
      e.key === 'ArrowLeft' ||
      (variant === 'radio' && e.key === 'ArrowUp')
    ) {
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
      data-variant={variant}
      className={cn(
        variant === 'tabs' ? uiClasses.tabsList : uiClasses.segmentedControl,
        className,
      )}
    >
      {options.map((opt, idx) => {
        const active = opt.value === value
        const ariaState =
          variant === 'radio'
            ? { 'aria-checked': active }
            : { 'aria-selected': active }
        return (
          <button
            key={opt.value}
            type="button"
            role={itemRole}
            {...ariaState}
            ref={(el) => {
              buttonRefs.current[idx] = el
            }}
            tabIndex={active ? 0 : -1}
            onClick={() => onChange(opt.value)}
            onKeyDown={(e) => handleKeyDown(e, idx)}
            title={opt.title}
            data-selected={active || undefined}
            className={cn(
              variant === 'tabs' ? uiClasses.tab : uiClasses.segmentedItem,
              itemClassName,
              active && activeItemClassName,
            )}
          >
            {variant === 'tabs' ? (
              <TabIconSlot icon={opt.icon} value={opt.value} />
            ) : null}
            <span>{opt.label}</span>
          </button>
        )
      })}
    </div>
  )
}

/** Backwards-compatible local name; new shared consumers use SegmentedControl. */
export const ModeToggle = SegmentedControl
