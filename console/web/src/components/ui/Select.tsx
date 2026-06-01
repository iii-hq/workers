import * as SelectPrimitive from '@radix-ui/react-select'
import type * as React from 'react'
import { cn } from '@/lib/utils'

interface SelectOption<T extends string> {
  value: T
  label: string
}

interface SelectGroup<T extends string> {
  label: string
  options: SelectOption<T>[]
}

interface SelectProps<T extends string> {
  /**
   * Current value. `undefined` (or any value that matches no option) renders
   * the `placeholder` rather than leaking the raw token into the trigger.
   */
  value: T | undefined
  options?: SelectOption<T>[]
  groups?: SelectGroup<T>[]
  onChange: (next: T) => void
  disabled?: boolean
  className?: string
  /** Forwarded to the trigger for screen-readers. */
  'aria-label'?: string
  /** Forwarded to the trigger; used to indicate the option list is still loading. */
  'aria-busy'?: boolean
  /** Optional placeholder shown when no `value` matches an option. */
  placeholder?: string
  /**
   * Render a leading option that clears the selection. Picking it calls
   * `onClear` (not `onChange`), so the consumer decides what "empty" means
   * (e.g. set the field back to `undefined`). Off by default so existing
   * single-choice pickers like `ModelPicker` are unaffected.
   */
  allowEmpty?: boolean
  /** Label for the `allowEmpty` option. Defaults to `none`. */
  emptyLabel?: string
  /** Called when the `allowEmpty` option is picked. Required to do anything useful when `allowEmpty` is set. */
  onClear?: () => void
  /**
   * Replace the default `<Label>` rendering for each group. The returned
   * node is mounted inside `<SelectPrimitive.Group>` so labelling
   * semantics are preserved as long as the renderer includes a
   * `SelectPrimitive.Label` (or another labelling element).
   */
  renderGroupHeader?: (group: SelectGroup<T>) => React.ReactNode
}

export type { SelectGroup, SelectOption }

/**
 * Sentinel value for the `allowEmpty` option. Radix forbids an `Item` with
 * an empty-string value (it reserves `""` for the cleared/placeholder state),
 * so the clear affordance rides on a non-colliding marker that we intercept
 * in `onValueChange` before it reaches the consumer's `onChange`.
 */
const EMPTY_VALUE = '\u0000empty'

/**
 * Radix-based dropdown. Replaces the native `<select>` so the panel can be
 * themed consistently with the rest of the console (font-mono, lowercase,
 * ink/bg/rule tokens) instead of inheriting the OS' dark-mode default look.
 *
 * Keeps the same external API the native version had: pass either `options`
 * (flat list) or `groups` (sectioned). `value`, `onChange`, `disabled` and
 * the two aria props flow through unchanged so existing consumers
 * (`ModelPicker`, the primitives gallery) don't need to be touched.
 *
 * A value that matches no option (including `undefined`) shows the
 * `placeholder` instead of the raw token, the trigger truncates long labels,
 * and `allowEmpty` adds a leading clear option.
 */
export function Select<T extends string>({
  value,
  options,
  groups,
  onChange,
  disabled,
  className,
  placeholder,
  allowEmpty,
  emptyLabel,
  onClear,
  renderGroupHeader,
  ...aria
}: SelectProps<T>) {
  // Pre-flatten so the trigger can resolve `value -> label` without an extra
  // children-walk inside Radix.
  const flatOptions: SelectOption<T>[] = groups
    ? groups.flatMap((g) => g.options)
    : (options ?? [])
  const selected = flatOptions.find((o) => o.value === value)
  // Feed Radix `""` when nothing matches so it shows the placeholder instead
  // of rendering the raw (possibly `undefined`) token.
  const rootValue = selected ? (value as string) : ''

  return (
    <SelectPrimitive.Root
      value={rootValue}
      onValueChange={(next) => {
        if (allowEmpty && next === EMPTY_VALUE) {
          onClear?.()
          return
        }
        onChange(next as T)
      }}
      disabled={disabled}
    >
      <SelectPrimitive.Trigger
        aria-label={aria['aria-label']}
        aria-busy={aria['aria-busy']}
        className={cn(
          'inline-flex items-center justify-between gap-x-2 border border-rule bg-bg px-3 h-9 text-ink font-mono text-[13px] lowercase focus:outline-none focus:border-ink data-[state=open]:border-ink transition-colors max-w-full min-w-0 data-[placeholder]:text-ink-faint',
          disabled && 'opacity-40 pointer-events-none',
          className,
        )}
      >
        {/*
         * Radix's `Select.Value` strips `className`/`style` from the span it
         * renders, so truncation has to live on a wrapper we control. The
         * wrapper is the flex item (`flex-1 min-w-0`) and owns the ellipsis;
         * `white-space: nowrap` inherits into Radix's inline span, keeping the
         * label on one line so the trigger stays at its fixed height.
         */}
        <span className="min-w-0 flex-1 truncate text-left">
          <SelectPrimitive.Value placeholder={placeholder}>
            {selected?.label}
          </SelectPrimitive.Value>
        </span>
        <SelectPrimitive.Icon asChild>
          <span aria-hidden className="shrink-0 text-ink-faint">
            <svg
              width="8"
              height="6"
              viewBox="0 0 8 6"
              fill="none"
              stroke="currentColor"
              strokeWidth="1"
              aria-hidden="true"
            >
              <path d="M1 1L4 5L7 1" />
            </svg>
          </span>
        </SelectPrimitive.Icon>
      </SelectPrimitive.Trigger>

      <SelectPrimitive.Portal>
        <SelectPrimitive.Content
          position="popper"
          sideOffset={4}
          className={cn(
            'z-50 min-w-[var(--radix-select-trigger-width)] overflow-hidden border border-rule bg-bg text-ink font-mono text-[13px] lowercase shadow-lg',
            'data-[state=open]:animate-in data-[state=closed]:animate-out',
            'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
          )}
        >
          <SelectPrimitive.ScrollUpButton className="flex items-center justify-center h-5 text-ink-faint cursor-default">
            <svg
              width="8"
              height="6"
              viewBox="0 0 8 6"
              fill="none"
              stroke="currentColor"
              strokeWidth="1"
              aria-hidden="true"
            >
              <path d="M1 5L4 1L7 5" />
            </svg>
          </SelectPrimitive.ScrollUpButton>
          <SelectPrimitive.Viewport className="p-1 max-h-[60vh]">
            {allowEmpty ? (
              <SelectItem value={EMPTY_VALUE} label={emptyLabel ?? 'none'} />
            ) : null}
            {groups
              ? groups.map((g) => (
                  <SelectPrimitive.Group key={g.label}>
                    {renderGroupHeader ? (
                      renderGroupHeader(g)
                    ) : (
                      <SelectPrimitive.Label className="px-3 pt-2 pb-1 text-[11px] uppercase tracking-[0.12em] text-ink-faint">
                        {g.label}
                      </SelectPrimitive.Label>
                    )}
                    {g.options.map((opt) => (
                      <SelectItem
                        key={opt.value}
                        value={opt.value}
                        label={opt.label}
                      />
                    ))}
                  </SelectPrimitive.Group>
                ))
              : (options ?? []).map((opt) => (
                  <SelectItem
                    key={opt.value}
                    value={opt.value}
                    label={opt.label}
                  />
                ))}
          </SelectPrimitive.Viewport>
          <SelectPrimitive.ScrollDownButton className="flex items-center justify-center h-5 text-ink-faint cursor-default">
            <svg
              width="8"
              height="6"
              viewBox="0 0 8 6"
              fill="none"
              stroke="currentColor"
              strokeWidth="1"
              aria-hidden="true"
            >
              <path d="M1 1L4 5L7 1" />
            </svg>
          </SelectPrimitive.ScrollDownButton>
        </SelectPrimitive.Content>
      </SelectPrimitive.Portal>
    </SelectPrimitive.Root>
  )
}

interface SelectItemProps {
  value: string
  label: string
}

function SelectItem({ value, label }: SelectItemProps) {
  return (
    <SelectPrimitive.Item
      value={value}
      className={cn(
        'relative flex items-center pl-7 pr-3 py-1.5 cursor-pointer outline-none select-none',
        'data-[highlighted]:bg-rule data-[highlighted]:text-ink',
        'data-[state=checked]:text-ink',
        'data-[disabled]:opacity-40 data-[disabled]:pointer-events-none',
      )}
    >
      <SelectPrimitive.ItemIndicator className="absolute left-2 top-1/2 -translate-y-1/2 text-ink">
        <span aria-hidden>✓</span>
      </SelectPrimitive.ItemIndicator>
      <SelectPrimitive.ItemText>{label}</SelectPrimitive.ItemText>
    </SelectPrimitive.Item>
  )
}
