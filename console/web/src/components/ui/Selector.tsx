import uiClasses from '@iii-dev/console-ui/ui-classes'
import * as PopoverPrimitive from '@radix-ui/react-popover'
import { Check, ChevronDown, LoaderCircle, Search } from 'lucide-react'
import * as React from 'react'
import { PortalScope } from '@/lib/ui-scope'
import { cn } from '@/lib/utils'

export interface SelectorOption<T extends string = string> {
  value: T
  label: string
  description?: string
  keywords?: readonly string[]
  disabled?: boolean
}

export interface SelectorGroup<T extends string = string> {
  label: string
  options: readonly SelectorOption<T>[]
}

export interface SelectorProps<T extends string = string> {
  value: T | undefined
  options?: readonly SelectorOption<T>[]
  groups?: readonly SelectorGroup<T>[]
  onChange: (next: T) => void
  open?: boolean
  onOpenChange?: (open: boolean) => void
  query?: string
  onQueryChange?: (query: string) => void
  /** Disable local filtering when the caller owns a remote/async search. */
  shouldFilter?: boolean
  loading?: boolean
  error?: React.ReactNode
  validationMessage?: React.ReactNode
  disabled?: boolean
  invalid?: boolean
  className?: string
  contentClassName?: string
  placeholder?: string
  searchPlaceholder?: string
  emptyMessage?: React.ReactNode
  loadingMessage?: React.ReactNode
  allowEmpty?: boolean
  emptyLabel?: string
  onClear?: () => void
  /** Optional free-form commit for selectors such as arbitrary attribute keys. */
  onCreate?: (query: string) => void
  createOptionLabel?: (query: string) => React.ReactNode
  triggerIcon?: React.ReactNode
  'aria-label': string
  'aria-describedby'?: string
}

interface IndexedOption<T extends string> {
  option: SelectorOption<T>
  index: number
}

interface IndexedGroup<T extends string> {
  label?: string
  options: IndexedOption<T>[]
}

function optionMatchesQuery<T extends string>(
  option: SelectorOption<T>,
  normalizedQuery: string,
): boolean {
  if (!normalizedQuery) return true
  return [option.label, option.description, ...(option.keywords ?? [])]
    .filter(Boolean)
    .join(' ')
    .toLocaleLowerCase()
    .includes(normalizedQuery)
}

/** Pure filtering/indexing seam covered by unit tests. */
export function filterSelectorGroups<T extends string>({
  options,
  groups,
  query,
  shouldFilter = true,
}: Pick<SelectorProps<T>, 'options' | 'groups' | 'shouldFilter'> & {
  query: string
}): IndexedGroup<T>[] {
  const source: readonly SelectorGroup<T>[] = groups ?? [
    { label: '', options: options ?? [] },
  ]
  const normalizedQuery = query.trim().toLocaleLowerCase()
  let index = 0
  return source
    .map((group) => ({
      label: group.label || undefined,
      options: group.options
        .filter(
          (option) =>
            !shouldFilter || optionMatchesQuery(option, normalizedQuery),
        )
        .map((option) => ({ option, index: index++ })),
    }))
    .filter((group) => group.options.length > 0)
}

function allSelectorOptions<T extends string>(
  options?: readonly SelectorOption<T>[],
  groups?: readonly SelectorGroup<T>[],
): readonly SelectorOption<T>[] {
  return groups ? groups.flatMap((group) => group.options) : (options ?? [])
}

export function Selector<T extends string>({
  value,
  options,
  groups,
  onChange,
  open: controlledOpen,
  onOpenChange,
  query: controlledQuery,
  onQueryChange,
  shouldFilter = true,
  loading,
  error,
  validationMessage,
  disabled,
  invalid,
  className,
  contentClassName,
  placeholder = 'Select…',
  searchPlaceholder = 'Search…',
  emptyMessage = 'No options found',
  loadingMessage = 'Loading options…',
  allowEmpty,
  emptyLabel = 'None',
  onClear,
  onCreate,
  createOptionLabel,
  triggerIcon,
  'aria-label': ariaLabel,
  'aria-describedby': ariaDescribedBy,
}: SelectorProps<T>) {
  const [internalOpen, setInternalOpen] = React.useState(false)
  const [internalQuery, setInternalQuery] = React.useState('')
  const [activeIndex, setActiveIndex] = React.useState<number | null>(null)
  const inputRef = React.useRef<HTMLInputElement>(null)
  const listboxId = React.useId()
  const statusId = React.useId()
  const validationId = React.useId()
  const open = controlledOpen ?? internalOpen
  const query = controlledQuery ?? internalQuery
  const flatOptions = allSelectorOptions(options, groups)
  const selected = flatOptions.find((option) => option.value === value)
  const filteredGroups = React.useMemo(
    () => filterSelectorGroups({ options, groups, query, shouldFilter }),
    [groups, options, query, shouldFilter],
  )
  const visibleOptions = React.useMemo(
    () => filteredGroups.flatMap((group) => group.options),
    [filteredGroups],
  )
  const enabledOptions = React.useMemo(
    () => visibleOptions.filter(({ option }) => !option.disabled),
    [visibleOptions],
  )
  const active = visibleOptions.find((entry) => entry.index === activeIndex)
  const trimmedQuery = query.trim()
  const canCreate = Boolean(
    onCreate &&
      trimmedQuery &&
      !flatOptions.some(
        (option) =>
          option.value.toLocaleLowerCase() ===
            trimmedQuery.toLocaleLowerCase() ||
          option.label.toLocaleLowerCase() === trimmedQuery.toLocaleLowerCase(),
      ),
  )
  const describedBy = [ariaDescribedBy, validationMessage && validationId]
    .filter(Boolean)
    .join(' ')

  function setQuery(next: string) {
    if (controlledQuery === undefined) setInternalQuery(next)
    onQueryChange?.(next)
  }

  function setOpen(next: boolean) {
    if (controlledOpen === undefined) setInternalOpen(next)
    onOpenChange?.(next)
    if (!next) {
      setQuery('')
      setActiveIndex(null)
      return
    }
    const selectedEntry = visibleOptions.find(
      ({ option }) => option.value === value && !option.disabled,
    )
    setActiveIndex(
      selectedEntry?.index ??
        (allowEmpty && value === undefined ? -1 : enabledOptions[0]?.index) ??
        (allowEmpty ? -1 : null),
    )
  }

  function commit(option: SelectorOption<T>) {
    if (option.disabled) return
    onChange(option.value)
    setOpen(false)
  }

  function commitCreate() {
    if (!canCreate || !onCreate) return
    onCreate(trimmedQuery)
    setOpen(false)
  }

  function moveActive(direction: 1 | -1) {
    const indexes = [
      ...(allowEmpty ? [-1] : []),
      ...enabledOptions.map((entry) => entry.index),
      ...(canCreate ? [-2] : []),
    ]
    if (indexes.length === 0) return
    const current = indexes.indexOf(activeIndex ?? Number.NaN)
    const next =
      current < 0
        ? direction > 0
          ? 0
          : indexes.length - 1
        : (current + direction + indexes.length) % indexes.length
    setActiveIndex(indexes[next])
  }

  function handleSearchKeyDown(event: React.KeyboardEvent<HTMLInputElement>) {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      moveActive(event.key === 'ArrowDown' ? 1 : -1)
      return
    }
    if (event.key === 'Home' || event.key === 'End') {
      event.preventDefault()
      if (event.key === 'Home' && allowEmpty) {
        setActiveIndex(-1)
      } else {
        const option =
          event.key === 'Home'
            ? enabledOptions[0]
            : enabledOptions[enabledOptions.length - 1]
        setActiveIndex(option?.index ?? (allowEmpty ? -1 : null))
      }
      return
    }
    if (event.key === 'Enter' && activeIndex === -1 && allowEmpty) {
      event.preventDefault()
      onClear?.()
      setOpen(false)
      return
    }
    if (event.key === 'Enter' && activeIndex === -2 && canCreate) {
      event.preventDefault()
      commitCreate()
      return
    }
    if (event.key === 'Enter' && active) {
      event.preventDefault()
      commit(active.option)
    }
  }

  React.useEffect(() => {
    if (!open) return
    const activeStillVisible =
      (allowEmpty && activeIndex === -1) ||
      (canCreate && activeIndex === -2) ||
      enabledOptions.some((entry) => entry.index === activeIndex)
    if (!activeStillVisible) {
      setActiveIndex(
        enabledOptions[0]?.index ?? (canCreate ? -2 : allowEmpty ? -1 : null),
      )
    }
  }, [activeIndex, allowEmpty, canCreate, enabledOptions, open])

  React.useEffect(() => {
    if (activeIndex === null) return
    document
      .getElementById(`${listboxId}-option-${activeIndex}`)
      ?.scrollIntoView({ block: 'nearest' })
  }, [activeIndex, listboxId])

  return (
    <div className={cn('min-w-0', className)}>
      <PopoverPrimitive.Root open={open} onOpenChange={setOpen}>
        <PopoverPrimitive.Trigger asChild>
          <button
            type="button"
            disabled={disabled}
            aria-label={ariaLabel}
            aria-haspopup="listbox"
            aria-expanded={open}
            aria-controls={open ? listboxId : undefined}
            aria-invalid={invalid || undefined}
            aria-describedby={describedBy || undefined}
            className={cn(
              'iii-ui-motion-control inline-flex h-12 w-full min-w-0 items-center justify-between gap-2 rounded-sm border border-transparent bg-surface px-3 font-sans text-base text-ink hover:bg-surface-hover focus:border-rule-focus focus:outline-none disabled:pointer-events-none disabled:opacity-40 sm:h-9 sm:text-[13px]',
              open && 'border-rule-focus',
              invalid && 'border-alert',
            )}
          >
            {triggerIcon ? (
              <span className="inline-flex shrink-0 text-ink-faint" aria-hidden>
                {triggerIcon}
              </span>
            ) : null}
            <span
              className={cn(
                'min-w-0 flex-1 truncate text-left',
                !selected && 'text-ink-ghost',
              )}
            >
              {selected?.label ?? placeholder}
            </span>
            <ChevronDown
              className="size-4 shrink-0 text-ink-faint"
              aria-hidden
            />
          </button>
        </PopoverPrimitive.Trigger>

        <PopoverPrimitive.Portal>
          <PortalScope>
            <PopoverPrimitive.Content
              align="start"
              sideOffset={4}
              collisionPadding={8}
              onOpenAutoFocus={(event) => {
                event.preventDefault()
                inputRef.current?.focus()
              }}
              className={cn(
                uiClasses.panel,
                'iii-ui-motion-dropdown z-50 flex max-h-[min(28rem,calc(100dvh-1rem))] w-(--radix-popover-trigger-width) max-w-[calc(100vw-1rem)] min-w-[min(18rem,calc(100vw-1rem))] flex-col bg-panel-raised font-sans shadow-floating',
                contentClassName,
              )}
            >
              <div className="relative shrink-0 p-2">
                <Search
                  className="pointer-events-none absolute left-5 top-1/2 size-4 -translate-y-1/2 text-ink-faint"
                  aria-hidden
                />
                <input
                  ref={inputRef}
                  role="combobox"
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  onKeyDown={handleSearchKeyDown}
                  placeholder={searchPlaceholder}
                  aria-label={searchPlaceholder}
                  aria-autocomplete="list"
                  aria-expanded="true"
                  aria-controls={listboxId}
                  aria-activedescendant={
                    activeIndex === null
                      ? undefined
                      : `${listboxId}-option-${activeIndex}`
                  }
                  aria-describedby={statusId}
                  className="h-10 w-full rounded-sm border border-transparent bg-surface py-2 pr-3 pl-10 text-[13px] text-ink outline-none placeholder:text-ink-ghost focus:border-rule-focus"
                />
              </div>

              <div
                id={listboxId}
                role="listbox"
                aria-label={ariaLabel}
                aria-busy={loading || undefined}
                className={cn(uiClasses.list, 'min-h-0 overflow-y-auto p-1')}
              >
                {allowEmpty ? (
                  <div
                    id={`${listboxId}-option--1`}
                    role="option"
                    tabIndex={-1}
                    aria-selected={value === undefined}
                    data-selected={value === undefined ? true : undefined}
                    data-highlighted={activeIndex === -1 || undefined}
                    className={cn(uiClasses.listItem, 'cursor-pointer')}
                    onPointerDown={(event) => event.preventDefault()}
                    onPointerMove={() => setActiveIndex(-1)}
                    onKeyDown={(event) => {
                      if (event.key !== 'Enter' && event.key !== ' ') return
                      event.preventDefault()
                      onClear?.()
                      setOpen(false)
                    }}
                    onClick={() => {
                      onClear?.()
                      setOpen(false)
                    }}
                  >
                    <span className={uiClasses.listItemContent}>
                      <span className={uiClasses.listItemTitle}>
                        {emptyLabel}
                      </span>
                    </span>
                    {value === undefined ? (
                      <Check className="size-4 text-ink" aria-hidden />
                    ) : null}
                  </div>
                ) : null}

                {filteredGroups.map((group) => (
                  // biome-ignore lint/a11y/useSemanticElements: ARIA listbox groups are not form fieldsets
                  <div
                    key={group.label ?? '__ungrouped'}
                    role="group"
                    aria-label={group.label}
                    className={uiClasses.listGroup}
                  >
                    {group.label ? (
                      <div className={uiClasses.listGroupLabel}>
                        {group.label}
                      </div>
                    ) : null}
                    {group.options.map(({ option, index }) => {
                      const optionSelected = option.value === value
                      const highlighted = index === activeIndex
                      return (
                        <div
                          key={option.value}
                          id={`${listboxId}-option-${index}`}
                          role="option"
                          tabIndex={-1}
                          aria-selected={optionSelected}
                          aria-disabled={option.disabled || undefined}
                          data-selected={optionSelected || undefined}
                          data-highlighted={highlighted || undefined}
                          className={cn(
                            uiClasses.listItem,
                            !option.disabled && 'cursor-pointer',
                          )}
                          onPointerDown={(event) => event.preventDefault()}
                          onPointerMove={() => {
                            if (!option.disabled) setActiveIndex(index)
                          }}
                          onKeyDown={(event) => {
                            if (event.key !== 'Enter' && event.key !== ' ')
                              return
                            event.preventDefault()
                            commit(option)
                          }}
                          onClick={() => commit(option)}
                        >
                          <span className={uiClasses.listItemContent}>
                            <span className={uiClasses.listItemTitle}>
                              {option.label}
                            </span>
                            {option.description ? (
                              <span className={uiClasses.listItemDescription}>
                                {option.description}
                              </span>
                            ) : null}
                          </span>
                          {optionSelected ? (
                            <Check
                              className="size-4 shrink-0 text-ink"
                              aria-hidden
                            />
                          ) : null}
                        </div>
                      )
                    })}
                  </div>
                ))}

                {canCreate ? (
                  <div
                    id={`${listboxId}-option--2`}
                    role="option"
                    tabIndex={-1}
                    aria-selected="false"
                    data-highlighted={activeIndex === -2 || undefined}
                    className={cn(uiClasses.listItem, 'cursor-pointer')}
                    onPointerDown={(event) => event.preventDefault()}
                    onPointerMove={() => setActiveIndex(-2)}
                    onKeyDown={(event) => {
                      if (event.key !== 'Enter' && event.key !== ' ') return
                      event.preventDefault()
                      commitCreate()
                    }}
                    onClick={commitCreate}
                  >
                    <span className={uiClasses.listItemContent}>
                      <span className={uiClasses.listItemTitle}>
                        {createOptionLabel?.(trimmedQuery) ??
                          `use “${trimmedQuery}”`}
                      </span>
                    </span>
                  </div>
                ) : null}

                {loading ? (
                  <SelectorStatus id={statusId}>
                    <LoaderCircle className="size-4 animate-spin" aria-hidden />
                    {loadingMessage}
                  </SelectorStatus>
                ) : error ? (
                  <SelectorStatus id={statusId} tone="danger" alert>
                    {error}
                  </SelectorStatus>
                ) : visibleOptions.length === 0 && !canCreate ? (
                  <SelectorStatus id={statusId}>{emptyMessage}</SelectorStatus>
                ) : (
                  <span id={statusId} className="sr-only" role="status">
                    {visibleOptions.length + (canCreate ? 1 : 0)} options
                  </span>
                )}
              </div>
            </PopoverPrimitive.Content>
          </PortalScope>
        </PopoverPrimitive.Portal>
      </PopoverPrimitive.Root>
      {validationMessage ? (
        <p id={validationId} className={uiClasses.fieldError}>
          {validationMessage}
        </p>
      ) : null}
    </div>
  )
}

function SelectorStatus({
  id,
  children,
  tone = 'neutral',
  alert,
}: {
  id: string
  children: React.ReactNode
  tone?: 'neutral' | 'danger'
  alert?: boolean
}) {
  return (
    <div
      id={id}
      role={alert ? 'alert' : 'status'}
      className={cn(
        'flex min-h-12 items-center justify-center gap-2 px-3 py-2 text-center text-[12px]',
        tone === 'danger' ? 'text-alert' : 'text-ink-faint',
      )}
    >
      {children}
    </div>
  )
}
