/**
 * Segmented control for view / filter switching — a self-contained port of the
 * console's `ModeToggle` (tabs variant). `@iii-dev/console-ui` exports no
 * segmented control, so the handful of styles live in styles.css scoped under
 * `[data-iii-ui="github"]`. Semantic `tablist` so screen readers announce the
 * options as tabs.
 */

import type { ReactNode } from 'react'

export interface ModeToggleOption<T extends string> {
  value: T
  label: ReactNode
  title?: string
}

interface ModeToggleProps<T extends string> {
  value: T
  onChange: (next: T) => void
  options: ModeToggleOption<T>[]
  'aria-label'?: string
}

export function ModeToggle<T extends string>({
  value,
  onChange,
  options,
  'aria-label': ariaLabel,
}: ModeToggleProps<T>) {
  return (
    <div className="gh-modes" role="tablist" aria-label={ariaLabel}>
      {options.map((opt) => {
        const active = opt.value === value
        return (
          <button
            key={opt.value}
            type="button"
            role="tab"
            aria-selected={active}
            title={opt.title}
            className={`gh-mode${active ? ' active' : ''}`}
            onClick={() => onChange(opt.value)}
          >
            {opt.label}
          </button>
        )
      })}
    </div>
  )
}
