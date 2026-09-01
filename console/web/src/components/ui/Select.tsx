import * as SelectPrimitive from '@radix-ui/react-select'
import { Check, ChevronDown, ChevronUp } from 'lucide-react'
import type * as React from 'react'
import { useEffect, useId, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { useMediaQuery } from '@/hooks/use-media-query'
import { PortalScope } from '@/lib/ui-scope'
import { cn } from '@/lib/utils'
import {
  BottomSheet,
  BottomSheetContent,
  useBottomSheetPageNavigation,
} from './BottomSheet'
import { SheetPage } from './SheetNavigation'

interface SelectOption<T extends string> {
  value: T
  label: string
  /** Optional hover tooltip on the option row. */
  title?: string
  /** Supporting copy shown below the label in menus and mobile sheets. */
  description?: string
  disabled?: boolean
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
  /** ID applied to the visible trigger so a `<label htmlFor>` can target it. */
  id?: string
  /** Form name. The controlled value is mirrored through a hidden input. */
  name?: string
  /** Stable configuration path applied to the visible trigger for deep-link focus. */
  'data-field'?: string
  /** Compact text-only trigger used in inline sentences and setup controls. */
  appearance?: 'default' | 'inline'
  /** Overrides the trigger appearance's default chevron visibility. */
  showChevron?: boolean
  /** Forwarded to the trigger for screen-readers. */
  'aria-label'?: string
  /** Forwarded to the trigger; used to indicate the option list is still loading. */
  'aria-busy'?: boolean
  /** Forwarded to the trigger when the selected value failed validation. */
  'aria-invalid'?: React.AriaAttributes['aria-invalid']
  /** IDs of validation or supporting text associated with the trigger. */
  'aria-describedby'?: string
  /** Optional placeholder shown when no `value` matches an option. */
  placeholder?: string
  /** Heading used by the mobile sheet or in-sheet drill-in page. */
  sheetTitle?: React.ReactNode
  /** Optional supporting copy under the mobile sheet heading. */
  sheetDescription?: React.ReactNode
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
 * Adaptive select: a Radix popper on desktop, a bottom sheet on mobile, or an
 * in-sheet detail page when it already lives inside `BottomSheetContent`.
 * Every presentation keeps the console's interface type and color tokens
 * instead of inheriting the OS' native-select treatment.
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
  id,
  name,
  'data-field': dataField,
  appearance = 'default',
  showChevron,
  placeholder,
  sheetTitle,
  sheetDescription,
  allowEmpty,
  emptyLabel,
  onClear,
  renderGroupHeader,
  ...aria
}: SelectProps<T>) {
  const [open, setOpen] = useState(false)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const embeddedPageRef = useRef<HTMLDivElement>(null)
  const pageId = useId()
  const pageTitleId = `${pageId}-title`
  const pageDescriptionId = `${pageId}-description`
  const mobileSheet = useMediaQuery('(max-width: 767px)')
  const pageNavigation = useBottomSheetPageNavigation()
  const embeddedInSheet = mobileSheet && pageNavigation !== null
  const closeSheetPage = pageNavigation?.closePage

  // Pre-flatten so the trigger can resolve `value -> label` without an extra
  // children-walk inside Radix.
  const flatOptions: SelectOption<T>[] = groups
    ? groups.flatMap((g) => g.options)
    : (options ?? [])
  const selected = flatOptions.find((o) => o.value === value)
  // Feed Radix `""` when nothing matches so it shows the placeholder instead
  // of rendering the raw (possibly `undefined`) token.
  const rootValue = selected ? (value as string) : ''

  const heading = sheetTitle ?? aria['aria-label'] ?? 'Select option'
  const chevronVisible = showChevron ?? appearance === 'default'
  const triggerClassName = cn(
    appearance === 'inline'
      ? 'relative inline-flex min-h-5.5 min-w-0 items-center border-dashed border-b border-ink-faint/50 px-0.5 font-sans text-base font-medium text-ink hover:border-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus data-[state=open]:border-ink sm:text-[0.8125rem]'
      : 'inline-flex h-12 max-w-full min-w-0 items-center justify-between gap-x-2 rounded-sm border border-transparent bg-surface px-3 font-sans text-base text-ink hover:bg-surface-hover focus:border-rule-focus focus:outline-none data-[state=open]:border-rule-focus data-[placeholder]:text-ink-ghost sm:h-9 sm:text-[13px]',
    disabled && 'pointer-events-none opacity-40',
    className,
  )

  function handleValueChange(next: string) {
    if (allowEmpty && next === EMPTY_VALUE) {
      onClear?.()
      return
    }
    onChange(next as T)
  }

  function handleOpenChange(nextOpen: boolean) {
    setOpen(nextOpen)
    if (!embeddedInSheet || !pageNavigation) return
    if (nextOpen) {
      pageNavigation.openPage(
        pageId,
        pageTitleId,
        sheetDescription ? pageDescriptionId : undefined,
      )
    } else pageNavigation.closePage(pageId)
  }

  function closeMobilePicker(restoreFocus: boolean) {
    setOpen(false)
    if (embeddedInSheet) pageNavigation?.closePage(pageId)
    if (restoreFocus) {
      window.requestAnimationFrame(() => triggerRef.current?.focus())
    }
  }

  function handleMobileValueChange(next: string) {
    handleValueChange(next)
    closeMobilePicker(embeddedInSheet)
  }

  useEffect(() => {
    if (!disabled || !open) return
    setOpen(false)
    closeSheetPage?.(pageId)
  }, [closeSheetPage, disabled, open, pageId])

  useEffect(
    () => () => {
      closeSheetPage?.(pageId)
    },
    [closeSheetPage, pageId],
  )

  useEffect(() => {
    if (!embeddedInSheet || !open || pageNavigation?.activePageId !== pageId) {
      return
    }
    const frame = window.requestAnimationFrame(() => {
      embeddedPageRef.current?.querySelector('button')?.focus()
    })
    return () => window.cancelAnimationFrame(frame)
  }, [embeddedInSheet, open, pageId, pageNavigation?.activePageId])

  const mobilePanel = (
    <SelectSheetPanel
      value={rootValue}
      options={options}
      groups={groups}
      allowEmpty={allowEmpty}
      emptyLabel={emptyLabel}
      disabled={disabled}
      label={typeof heading === 'string' ? heading : aria['aria-label']}
      onChange={handleMobileValueChange}
    />
  )

  if (mobileSheet) {
    const trigger = (
      <button
        ref={triggerRef}
        id={id}
        type="button"
        data-field={dataField}
        aria-label={aria['aria-label']}
        aria-busy={aria['aria-busy']}
        aria-invalid={aria['aria-invalid']}
        aria-describedby={aria['aria-describedby']}
        aria-haspopup={embeddedInSheet ? undefined : 'dialog'}
        aria-expanded={open}
        data-state={open ? 'open' : 'closed'}
        data-placeholder={selected ? undefined : ''}
        disabled={disabled}
        onClick={() => handleOpenChange(!open)}
        className={triggerClassName}
      >
        <span className="min-w-0 flex-1 truncate text-left">
          {selected?.label ?? placeholder}
        </span>
        {chevronVisible ? (
          <span aria-hidden className="shrink-0 text-ink-faint">
            <ChevronDown size={16} strokeWidth={1} />
          </span>
        ) : (
          <span
            className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
            aria-hidden="true"
          />
        )}
      </button>
    )

    if (embeddedInSheet) {
      const pagePortal =
        pageNavigation?.pageHost && pageNavigation.renderedPageId === pageId
          ? createPortal(
              <div
                ref={embeddedPageRef}
                className="flex min-h-0 flex-1 flex-col"
              >
                <SheetPage
                  title={heading}
                  description={sheetDescription}
                  titleId={pageTitleId}
                  descriptionId={
                    sheetDescription ? pageDescriptionId : undefined
                  }
                  dialogSemantics={false}
                  onBack={() => closeMobilePicker(true)}
                  backLabel={`Back from ${typeof heading === 'string' ? heading : 'options'}`}
                  contentClassName="px-3 pb-1"
                >
                  {mobilePanel}
                </SheetPage>
              </div>,
              pageNavigation.pageHost,
            )
          : null

      return (
        <>
          {name ? (
            <input
              type="hidden"
              name={name}
              value={value ?? ''}
              disabled={disabled}
            />
          ) : null}
          {trigger}
          {pagePortal}
        </>
      )
    }

    return (
      <>
        {name ? (
          <input
            type="hidden"
            name={name}
            value={value ?? ''}
            disabled={disabled}
          />
        ) : null}
        {trigger}
        <BottomSheet open={open} onOpenChange={handleOpenChange}>
          <BottomSheetContent
            heading={heading}
            description={sheetDescription}
            closeLabel={`Close ${typeof heading === 'string' ? heading : 'select'}`}
          >
            <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-1">
              {mobilePanel}
            </div>
          </BottomSheetContent>
        </BottomSheet>
      </>
    )
  }

  return (
    <>
      {name ? (
        <input
          type="hidden"
          name={name}
          value={value ?? ''}
          disabled={disabled}
        />
      ) : null}
      <SelectPrimitive.Root
        value={rootValue}
        onValueChange={handleValueChange}
        open={open}
        onOpenChange={handleOpenChange}
        disabled={disabled}
      >
        <SelectPrimitive.Trigger
          ref={triggerRef}
          id={id}
          data-field={dataField}
          aria-label={aria['aria-label']}
          aria-busy={aria['aria-busy']}
          aria-invalid={aria['aria-invalid']}
          aria-describedby={aria['aria-describedby']}
          className={triggerClassName}
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
          {chevronVisible ? (
            <SelectPrimitive.Icon asChild>
              <span aria-hidden className="shrink-0 text-ink-faint">
                <ChevronDown size={16} strokeWidth={1} />
              </span>
            </SelectPrimitive.Icon>
          ) : (
            <span
              className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
              aria-hidden="true"
            />
          )}
        </SelectPrimitive.Trigger>

        <SelectPrimitive.Portal>
          <PortalScope>
            <SelectPrimitive.Content
              position="popper"
              sideOffset={4}
              collisionPadding={8}
              className={cn(
                'iii-ui-motion-dropdown z-50 min-w-[var(--radix-select-trigger-width)] overflow-hidden rounded-md bg-panel-raised font-sans text-base text-ink shadow-floating sm:text-[13px]',
              )}
            >
              <SelectPrimitive.ScrollUpButton className="flex items-center justify-center h-5 text-ink-faint cursor-default">
                <ChevronUp size={16} strokeWidth={1} aria-hidden="true" />
              </SelectPrimitive.ScrollUpButton>
              <SelectPrimitive.Viewport className="p-1 max-h-[60vh]">
                {allowEmpty ? (
                  <SelectItem
                    value={EMPTY_VALUE}
                    label={emptyLabel ?? 'None'}
                  />
                ) : null}
                {groups
                  ? groups.map((g) => (
                      <SelectPrimitive.Group key={g.label}>
                        {renderGroupHeader ? (
                          renderGroupHeader(g)
                        ) : (
                          <SelectPrimitive.Label className="px-3 pt-2 pb-1 text-[12px] font-semibold text-ink-faint">
                            {g.label}
                          </SelectPrimitive.Label>
                        )}
                        {g.options.map((opt) => (
                          <SelectItem
                            key={opt.value}
                            value={opt.value}
                            label={opt.label}
                            title={opt.title}
                            description={opt.description}
                            disabled={opt.disabled}
                          />
                        ))}
                      </SelectPrimitive.Group>
                    ))
                  : (options ?? []).map((opt) => (
                      <SelectItem
                        key={opt.value}
                        value={opt.value}
                        label={opt.label}
                        title={opt.title}
                        description={opt.description}
                        disabled={opt.disabled}
                      />
                    ))}
              </SelectPrimitive.Viewport>
              <SelectPrimitive.ScrollDownButton className="flex items-center justify-center h-5 text-ink-faint cursor-default">
                <ChevronDown size={16} strokeWidth={1} aria-hidden="true" />
              </SelectPrimitive.ScrollDownButton>
            </SelectPrimitive.Content>
          </PortalScope>
        </SelectPrimitive.Portal>
      </SelectPrimitive.Root>
    </>
  )
}

interface SelectSheetPanelProps<T extends string> {
  value: string
  options?: SelectOption<T>[]
  groups?: SelectGroup<T>[]
  allowEmpty?: boolean
  emptyLabel?: string
  disabled?: boolean
  label?: string
  onChange: (next: string) => void
}

function SelectSheetPanel<T extends string>({
  value,
  options,
  groups,
  allowEmpty,
  emptyLabel,
  disabled,
  label = 'Options',
  onChange,
}: SelectSheetPanelProps<T>) {
  const name = useId()
  const emptyOption: SelectOption<string> = {
    value: EMPTY_VALUE,
    label: emptyLabel ?? 'None',
  }

  function optionRow(option: SelectOption<string>) {
    const selected =
      option.value === EMPTY_VALUE ? value === '' : option.value === value
    return (
      <label
        key={option.value}
        className={cn(
          'relative flex min-h-14 w-full min-w-0 cursor-pointer items-center gap-3 px-3 py-2 text-left font-sans text-base text-ink hover:bg-surface-hover has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-inset has-[:focus-visible]:ring-rule-focus',
          (disabled || option.disabled) &&
            'pointer-events-none cursor-default opacity-40',
        )}
      >
        <input
          type="radio"
          name={name}
          value={option.value}
          checked={selected}
          disabled={disabled || option.disabled}
          onChange={() => onChange(option.value)}
          className="sr-only"
        />
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="font-medium">{option.label}</span>
          {option.description || option.title ? (
            <span className="text-sm leading-relaxed text-ink-faint">
              {option.description ?? option.title}
            </span>
          ) : null}
        </span>
        {selected ? (
          <Check className="size-5 shrink-0 text-ink" aria-hidden />
        ) : null}
      </label>
    )
  }

  if (groups) {
    return (
      <div className="space-y-3">
        {allowEmpty ? (
          <fieldset
            disabled={disabled}
            className="divide-y divide-edge overflow-hidden rounded-lg bg-surface ring-1 ring-inset ring-edge"
          >
            <legend className="sr-only">{label}</legend>
            {optionRow(emptyOption)}
          </fieldset>
        ) : null}
        {groups.map((group) => (
          <fieldset key={group.label} disabled={disabled}>
            <legend className="px-1 pb-1 font-sans text-sm font-medium text-ink-faint">
              {group.label}
            </legend>
            <div className="divide-y divide-edge overflow-hidden rounded-lg bg-surface ring-1 ring-inset ring-edge">
              {group.options.map((option) => optionRow(option))}
            </div>
          </fieldset>
        ))}
      </div>
    )
  }

  return (
    <fieldset
      disabled={disabled}
      className="divide-y divide-edge overflow-hidden rounded-lg bg-surface ring-1 ring-inset ring-edge"
    >
      <legend className="sr-only">{label}</legend>
      {allowEmpty ? optionRow(emptyOption) : null}
      {(options ?? []).map((option) => optionRow(option))}
    </fieldset>
  )
}

interface SelectItemProps {
  value: string
  label: string
  title?: string
  description?: string
  disabled?: boolean
}

function SelectItem({
  value,
  label,
  title,
  description,
  disabled,
}: SelectItemProps) {
  return (
    <SelectPrimitive.Item
      value={value}
      title={title}
      disabled={disabled}
      className={cn(
        'relative flex min-h-12 cursor-pointer select-none items-center rounded-xs py-2 pr-3 pl-7 outline-none sm:min-h-0 sm:py-1.5',
        'data-[highlighted]:bg-surface-hover data-[highlighted]:text-ink',
        'data-[state=checked]:text-ink',
        'data-[disabled]:opacity-40 data-[disabled]:pointer-events-none',
      )}
    >
      <SelectPrimitive.ItemIndicator className="absolute left-2 top-1/2 -translate-y-1/2 text-ink">
        <span aria-hidden>✓</span>
      </SelectPrimitive.ItemIndicator>
      <div className="min-w-0">
        <SelectPrimitive.ItemText>{label}</SelectPrimitive.ItemText>
        {description ? (
          <div className="truncate text-[11px] leading-[1.5] text-ink-faint">
            {description}
          </div>
        ) : null}
      </div>
    </SelectPrimitive.Item>
  )
}
