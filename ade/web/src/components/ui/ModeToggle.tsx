import type * as React from 'react'
import { ToggleGroup, ToggleGroupItem } from '@/components/shadcn/toggle-group'
import { cn } from '@/lib/utils'
import { Hint } from './Hint'
import { TabIconSlot } from './TabIcon'

export interface SegmentedControlOption<T extends string> {
  value: T
  label: React.ReactNode
  title?: string
  /** Tabs default to a semantic 16px icon inferred from `value`; radios show
   *  only an icon passed here. `false` hides it. */
  icon?: React.ReactNode | false
}

/**
 * `tabs` (default): view switching (top nav), spaced pills.
 * `radio`: one persistent preference (theme, a source), joined segments.
 * Both are the shadcn toggle group: one pressed item, arrow keys move.
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
  /** Show only the semantic icon and expose each label through Tooltip. */
  iconOnly?: boolean
  /** Accessible name for the group. */
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
  iconOnly = false,
  'aria-label': ariaLabel,
}: SegmentedControlProps<T>) {
  const renderIconOnly = iconOnly && variant === 'tabs'
  const joined = variant === 'radio'
  return (
    <ToggleGroup
      aria-label={ariaLabel}
      value={[value]}
      // One item is always pressed: releasing the pressed one changes nothing.
      onValueChange={(next) => {
        const picked = next.find((entry) => entry !== value) ?? next[0]
        if (picked !== undefined && picked !== value) onChange(picked as T)
      }}
      variant={joined ? 'outline' : 'default'}
      size="sm"
      spacing={joined ? 0 : 2}
      data-variant={variant}
      data-icon-only={renderIconOnly || undefined}
      className={cn(joined && 'bg-muted', className)}
    >
      {options.map((opt) => {
        const active = opt.value === value
        const accessibleLabel =
          opt.title ?? (typeof opt.label === 'string' ? opt.label : opt.value)
        const item = (
          <ToggleGroupItem
            key={opt.value}
            value={opt.value}
            aria-label={renderIconOnly ? accessibleLabel : undefined}
            title={renderIconOnly ? undefined : opt.title}
            data-selected={active || undefined}
            className={cn(
              'text-muted-foreground aria-pressed:text-foreground',
              joined && 'aria-pressed:bg-background aria-pressed:shadow-sm',
              itemClassName,
              active && activeItemClassName,
            )}
          >
            {variant === 'tabs' || opt.icon ? (
              <TabIconSlot icon={opt.icon} value={opt.value} />
            ) : null}
            {renderIconOnly ? null : <span>{opt.label}</span>}
          </ToggleGroupItem>
        )
        if (!renderIconOnly) return item
        return (
          <Hint key={opt.value} label={opt.label}>
            {item}
          </Hint>
        )
      })}
    </ToggleGroup>
  )
}

/** Backwards-compatible local name; new shared consumers use SegmentedControl. */
export const ModeToggle = SegmentedControl
