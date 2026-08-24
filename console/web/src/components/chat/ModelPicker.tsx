import * as DropdownMenuPrimitive from '@radix-ui/react-dropdown-menu'
import {
  ArrowLeft,
  Check,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  RefreshCw,
} from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { cn } from '@/lib/utils'
import { useUnsavedGuard } from '@/pages/Configuration/tabs/WorkersTab/useUnsavedGuard'
import {
  type ModelId,
  type ModelOption,
  type ReasoningEffortOption,
  THINKING_LEVELS,
  type ThinkingLevel,
} from '@/types/chat'
import {
  formatModelLabel,
  formatProviderLabel,
  providerForModel,
} from './model-picker-presentation'
import { ProviderConfigurationPanel } from './ProviderConfigurationPanel'

const DEFAULT_EFFORT: ReasoningEffortOption = {
  effort: 'default',
  description: 'use the model default',
}

const FILTER_KEYS = ['ArrowDown', 'ArrowUp', 'Home', 'End', 'Enter', 'Escape']

interface ModelPickerProps {
  value: ModelId | null
  options: ModelOption[]
  /** Incremented by a parent CTA to open this picker when its trigger is visible. */
  openRequest?: number
  thinkingLevel: ThinkingLevel
  onChange: (next: ModelId) => void
  onThinkingLevelChange: (next: ThinkingLevel) => void
  disabled?: boolean
  loading?: boolean
  showRefresh?: boolean
  className?: string
}

interface PickerSubpageHeaderProps {
  title: string
  description: string
  onBack: () => void
}

function PickerSubpageHeader({
  title,
  description,
  onBack,
}: PickerSubpageHeaderProps) {
  return (
    <div className="flex shrink-0 items-start gap-2 px-4 py-3 pr-12">
      <button
        type="button"
        aria-label="back to models"
        onClick={onBack}
        className="relative flex size-8 shrink-0 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
      >
        <span
          className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
          aria-hidden="true"
        />
        <ArrowLeft className="size-4 shrink-0" aria-hidden />
      </button>
      <div className="min-w-0 flex-1 pt-0.5">
        <h2 className="font-sans text-lg font-semibold text-ink">{title}</h2>
        <p className="font-sans text-sm text-pretty text-ink-faint">
          {description}
        </p>
      </div>
    </div>
  )
}

interface ModelGroup {
  label: string
  options: ModelOption[]
}

function groupByProvider(options: ModelOption[]): ModelGroup[] {
  const byProvider = new Map<string, ModelOption[]>()
  for (const opt of options) {
    const provider = providerForModel(opt.id) ?? '—'
    const bucket = byProvider.get(provider) ?? []
    bucket.push(opt)
    byProvider.set(provider, bucket)
  }
  return [...byProvider.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([label, opts]) => ({ label, options: opts }))
}

function effortOptionsFor(
  model: ModelOption | undefined,
): ReasoningEffortOption[] {
  if (!model) return []
  if (model.reasoningEfforts && model.reasoningEfforts.length > 0) {
    return [
      DEFAULT_EFFORT,
      ...model.reasoningEfforts.filter((option) => option.effort !== 'default'),
    ]
  }
  if (!model.supportsThinking) return []
  return THINKING_LEVELS.map((effort) => ({ effort }))
}

function effortSupported(
  options: ReasoningEffortOption[],
  effort: ThinkingLevel,
): boolean {
  return options.some((option) => option.effort === effort)
}

export function ModelPicker({
  value,
  options,
  openRequest,
  thinkingLevel,
  onChange,
  onThinkingLevelChange,
  disabled,
  loading,
  showRefresh = true,
  className,
}: ModelPickerProps) {
  const ctx = useConversationsCtxOptional()
  const [open, setOpen] = useState(false)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const consumedOpenRequestRef = useRef(openRequest)
  const [configurationProvider, setConfigurationProvider] = useState<
    string | null
  >(null)
  const [reasoningOpen, setReasoningOpen] = useState(false)
  const configurationGuard = useUnsavedGuard()

  const optionsById = useMemo(
    () => new Map(options.map((option) => [option.id, option])),
    [options],
  )

  const safeValue =
    value != null && options.some((option) => option.id === value)
      ? value
      : undefined
  const selected = safeValue ? optionsById.get(safeValue) : undefined
  const selectedEfforts = useMemo(() => effortOptionsFor(selected), [selected])
  const pickerDisabled = disabled || loading

  useEffect(() => {
    if (
      openRequest === undefined ||
      consumedOpenRequestRef.current === openRequest
    ) {
      return
    }
    consumedOpenRequestRef.current = openRequest
    if (pickerDisabled || triggerRef.current?.getClientRects().length === 0) {
      return
    }
    setConfigurationProvider(null)
    setReasoningOpen(false)
    setOpen(true)
  }, [openRequest, pickerDisabled])

  useEffect(() => {
    if (pickerDisabled && open) setOpen(false)
  }, [pickerDisabled, open])

  function handleOpenChange(nextOpen: boolean) {
    if (nextOpen) {
      setOpen(true)
      return
    }
    configurationGuard.tryNavigate(() => {
      setOpen(false)
      setConfigurationProvider(null)
      setReasoningOpen(false)
    })
  }

  return (
    <span className={cn('flex min-w-0 items-center gap-1', className)}>
      <DropdownMenuPrimitive.Root open={open} onOpenChange={handleOpenChange}>
        <DropdownMenuPrimitive.Trigger asChild disabled={pickerDisabled}>
          <button
            ref={triggerRef}
            type="button"
            aria-label={
              loading
                ? 'model (loading catalog)'
                : selected
                  ? `model: ${selected.label}, reasoning effort: ${thinkingLevel}`
                  : 'model'
            }
            aria-busy={loading || undefined}
            className={cn(
              'flex h-12 min-w-0 flex-1 items-center justify-between gap-x-2 rounded-sm border border-transparent bg-transparent px-3 font-sans text-base text-ink-faint hover:bg-surface-hover hover:text-ink focus:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus data-[state=open]:bg-surface data-[state=open]:text-ink sm:h-9 sm:text-[13px]',
              pickerDisabled && 'pointer-events-none opacity-40',
            )}
          >
            <span className="flex min-w-0 flex-1 items-baseline gap-1.5 overflow-hidden whitespace-nowrap text-left">
              <span
                className={cn(
                  'min-w-0 flex-1 truncate',
                  !selected && 'text-ink-faint',
                )}
              >
                {selected?.label ?? (loading ? 'Loading…' : 'No models')}
              </span>
              {selectedEfforts.length > 1 && thinkingLevel !== 'default' ? (
                <span className="shrink-0 text-[11px] text-ink-faint">
                  · {thinkingLevel}
                </span>
              ) : null}
            </span>
            {open ? (
              <ChevronUp size={16} aria-hidden />
            ) : (
              <ChevronDown size={16} aria-hidden />
            )}
          </button>
        </DropdownMenuPrimitive.Trigger>

        <DropdownMenuPrimitive.Portal>
          <DropdownMenuPrimitive.Content
            sideOffset={4}
            align="start"
            collisionPadding={12}
            className="z-50 flex h-[min(72vh,720px)] w-[min(480px,calc(100vw-24px))] flex-col overflow-hidden rounded-lg border border-edge bg-panel-raised text-ink shadow-floating"
          >
            {configurationProvider ? (
              <div className="flex min-h-0 flex-1 flex-col">
                <PickerSubpageHeader
                  title={
                    formatProviderLabel(configurationProvider) ??
                    configurationProvider
                  }
                  description="Credentials and provider-specific settings."
                  onBack={() =>
                    configurationGuard.tryNavigate(() =>
                      setConfigurationProvider(null),
                    )
                  }
                />
                <ProviderConfigurationPanel
                  providerId={configurationProvider}
                  onDirtyChange={configurationGuard.setDirty}
                />
              </div>
            ) : reasoningOpen ? (
              <div className="flex min-h-0 flex-1 flex-col">
                <PickerSubpageHeader
                  title="Reasoning effort"
                  description="Choose how much reasoning this model should use."
                  onBack={() => setReasoningOpen(false)}
                />
                <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-3">
                  <ReasoningEffortPanel
                    model={selected}
                    value={thinkingLevel}
                    onChange={(next) => {
                      onThinkingLevelChange(next)
                      setReasoningOpen(false)
                    }}
                    disabled={disabled}
                  />
                </div>
              </div>
            ) : (
              <div className="flex min-h-0 flex-1 flex-col pt-4">
                <ModelPickerPanel
                  value={value}
                  options={options}
                  thinkingLevel={thinkingLevel}
                  onChange={onChange}
                  onThinkingLevelChange={onThinkingLevelChange}
                  onConfigureProvider={(providerId) => {
                    setReasoningOpen(false)
                    setConfigurationProvider(providerId)
                  }}
                  onOpenReasoning={() => setReasoningOpen(true)}
                  disabled={disabled}
                  loading={loading}
                  contentClassName="space-y-4 px-3 pb-3"
                  autoFocusFilter
                />
              </div>
            )}
          </DropdownMenuPrimitive.Content>
        </DropdownMenuPrimitive.Portal>
      </DropdownMenuPrimitive.Root>

      {ctx && showRefresh ? (
        <button
          type="button"
          aria-label="refresh model list"
          title="refresh model list from providers"
          disabled={ctx.refreshingModels || disabled}
          onClick={() => {
            void ctx.refreshModels()
          }}
          className="inline-flex size-12 shrink-0 items-center justify-center text-ink-ghost hover:text-ink focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50 sm:size-9"
        >
          <RefreshCw
            className={cn(
              'size-4 shrink-0',
              ctx.refreshingModels && 'animate-spin',
            )}
            aria-hidden
          />
        </button>
      ) : null}
    </span>
  )
}

interface ModelPickerPanelProps {
  value: ModelId | null
  options: ModelOption[]
  thinkingLevel: ThinkingLevel
  onChange: (next: ModelId) => void
  onThinkingLevelChange: (next: ThinkingLevel) => void
  onConfigureProvider?: (providerId: string) => void
  onOpenReasoning?: () => void
  disabled?: boolean
  loading?: boolean
  className?: string
  contentClassName?: string
  /** Put the caret in the filter on mount: the dropdown does, a sheet page
      that opens under a finger does not. */
  autoFocusFilter?: boolean
}

/**
 * Inline model selector for an existing sheet page. It deliberately renders
 * no portal or overlay; the same catalog and effort rules power ModelPicker's
 * desktop dropdown and this mobile navigation surface.
 */
export function ModelPickerPanel({
  value,
  options,
  thinkingLevel,
  onChange,
  onThinkingLevelChange,
  onConfigureProvider,
  onOpenReasoning,
  disabled,
  loading,
  className,
  contentClassName,
  autoFocusFilter = false,
}: ModelPickerPanelProps) {
  const ctx = useConversationsCtxOptional()
  const effortByModel = useRef(new Map<ModelId, ThinkingLevel>())
  // Type to filter, arrows to move, Enter to pick: the caret stays in the
  // filter the whole time, the way a command palette behaves, because forty
  // models across eight providers is a list you search, not one you scroll.
  const [filter, setFilter] = useState('')
  const [activeIndex, setActiveIndex] = useState(0)
  const listRef = useRef<HTMLDivElement>(null)
  const optionsById = useMemo(
    () => new Map(options.map((option) => [option.id, option])),
    [options],
  )
  const safeValue = value != null && optionsById.has(value) ? value : undefined
  const selected = safeValue ? optionsById.get(safeValue) : undefined
  const selectedEfforts = useMemo(() => effortOptionsFor(selected), [selected])
  const presentProviders = ctx?.presentProviders ?? []
  const presentIds = presentProviders.map((provider) => provider.id)
  const providerById = new Map(
    presentProviders.map((provider) => [provider.id, provider]),
  )
  const modelGroups = groupByProvider(options)
  const grouped = new Set(modelGroups.map((group) => group.label))
  const filterWords = filter.toLowerCase().split(/\s+/).filter(Boolean)
  const matchesFilter = (option: ModelOption, provider: string) => {
    if (filterWords.length === 0) return true
    const hay =
      `${formatModelLabel(option.label)} ${option.id} ${formatProviderLabel(provider) ?? provider}`.toLowerCase()
    return filterWords.every((word) => hay.includes(word))
  }
  const groups = [
    ...modelGroups.map((group) => ({
      label: group.label,
      options: group.options.filter((option) =>
        matchesFilter(option, group.label),
      ),
    })),
    ...presentIds
      .filter((id) => !grouped.has(id))
      .map((id) => ({ label: id, options: [] })),
  ]
    .filter((group) => filterWords.length === 0 || group.options.length > 0)
    .sort((a, b) => a.label.localeCompare(b.label))
  const visibleIds = groups.flatMap((group) =>
    group.options.map((option) => option.id),
  )
  const activeId = visibleIds[Math.min(activeIndex, visibleIds.length - 1)]

  useEffect(() => {
    if (!safeValue) return
    if (effortSupported(selectedEfforts, thinkingLevel)) {
      effortByModel.current.set(safeValue, thinkingLevel)
      return
    }
    if (thinkingLevel !== 'default') onThinkingLevelChange('default')
    effortByModel.current.set(safeValue, 'default')
  }, [safeValue, selectedEfforts, thinkingLevel, onThinkingLevelChange])

  useEffect(() => {
    if (!activeId) return
    listRef.current
      ?.querySelector(`[data-model-option="${CSS.escape(activeId)}"]`)
      ?.scrollIntoView({ block: 'nearest' })
  }, [activeId])

  const onFilterKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (!FILTER_KEYS.includes(event.key)) return
    // Escape with a filter typed clears it; the menu closes on the next one.
    if (event.key === 'Escape') {
      if (filter === '') return
      event.preventDefault()
      event.stopPropagation()
      setFilter('')
      setActiveIndex(0)
      return
    }
    event.preventDefault()
    event.stopPropagation()
    if (event.key === 'Enter') {
      if (activeId) selectModel(activeId)
      return
    }
    if (event.key === 'Home' || event.key === 'End') {
      setActiveIndex(event.key === 'Home' ? 0 : visibleIds.length - 1)
      return
    }
    if (visibleIds.length === 0) return
    const step = event.key === 'ArrowDown' ? 1 : -1
    setActiveIndex(
      (current) =>
        (Math.min(current, visibleIds.length - 1) + step + visibleIds.length) %
        visibleIds.length,
    )
  }

  function selectModel(next: ModelId) {
    if (disabled || loading) return
    const nextModel = optionsById.get(next)
    const nextEfforts = effortOptionsFor(nextModel)
    const remembered = effortByModel.current.get(next) ?? 'default'
    const nextEffort = effortSupported(nextEfforts, remembered)
      ? remembered
      : 'default'
    onChange(next)
    if (nextEffort !== thinkingLevel) onThinkingLevelChange(nextEffort)
  }

  return (
    <div className={cn('flex min-h-0 flex-1 flex-col', className)}>
      <div className="shrink-0 px-3 pb-3">
        <input
          type="search"
          value={filter}
          onChange={(event) => {
            setFilter(event.target.value)
            setActiveIndex(0)
          }}
          onKeyDown={onFilterKeyDown}
          // biome-ignore lint/a11y/noAutofocus: the dropdown opened to pick a model; typing is the fastest way to one
          autoFocus={autoFocusFilter}
          placeholder="Filter models…"
          aria-label="filter models"
          aria-activedescendant={
            activeId ? `model-option-${activeId}` : undefined
          }
          role="combobox"
          aria-expanded="true"
          aria-controls="model-picker-list"
          autoCapitalize="none"
          autoCorrect="off"
          autoComplete="off"
          spellCheck={false}
          className="h-9 w-full rounded-md bg-surface px-3 font-sans text-sm text-ink outline-none ring-1 ring-inset ring-edge placeholder:text-ink-ghost focus-visible:ring-rule-focus"
        />
      </div>
      <div
        id="model-picker-list"
        ref={listRef}
        className={cn(
          'min-h-0 flex-1 space-y-5 overflow-y-auto px-4 pb-4',
          contentClassName,
        )}
      >
        {loading ? (
          <div className="rounded-lg bg-surface px-3 py-4 font-sans text-base text-ink-faint">
            Loading model catalog…
          </div>
        ) : groups.length === 0 ? (
          <div className="rounded-lg bg-surface px-3 py-4 font-sans text-base text-ink-faint">
            {filterWords.length > 0
              ? `No model matches "${filter.trim()}".`
              : 'No models are configured.'}
          </div>
        ) : (
          groups.map((group) => {
            const provider = providerById.get(group.label)
            const unavailable = provider?.available === false
            const hasModels = group.options.length > 0
            const configured = provider?.configured ?? hasModels
            // OAuth/companion-app providers can be authenticated even though
            // the router slice has no API key and reports `configured: false`.
            // A successfully discovered catalog is authoritative for them.
            const providerOwnsAuthentication =
              provider !== undefined &&
              provider.credential_env_var === undefined
            const catalogIsUsable =
              hasModels && (configured || providerOwnsAuthentication)
            return (
              <section key={group.label} aria-label={group.label}>
                <div className="flex items-center justify-between gap-3 px-1 pb-2">
                  <h3 className="min-w-0 truncate font-sans text-[11px] font-medium text-ink-ghost">
                    {formatProviderLabel(group.label)}
                  </h3>
                  {(configured || catalogIsUsable) && onConfigureProvider ? (
                    <button
                      type="button"
                      onClick={() => onConfigureProvider(group.label)}
                      className="shrink-0 rounded-sm font-sans text-xs font-medium text-accent hover:text-accent-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
                    >
                      Configure
                    </button>
                  ) : null}
                </div>
                <div className="divide-y divide-edge overflow-hidden rounded-lg bg-surface ring-1 ring-inset ring-edge">
                  {!catalogIsUsable && !configured && onConfigureProvider ? (
                    <button
                      type="button"
                      disabled={disabled}
                      onClick={() => onConfigureProvider(group.label)}
                      className="flex min-h-16 w-full min-w-0 items-center gap-3 px-3 py-2 text-left font-sans text-base text-ink hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40"
                    >
                      <span className="flex min-w-0 flex-1 flex-col">
                        <span className="font-medium">Configure provider</span>
                        <span className="text-sm leading-relaxed text-ink-faint">
                          Add credentials and provider settings.
                        </span>
                      </span>
                      <ChevronRight
                        className="size-4 shrink-0 text-ink-faint"
                        aria-hidden
                      />
                    </button>
                  ) : catalogIsUsable ? (
                    group.options.map((option) => {
                      const selectedOption = option.id === safeValue
                      const highlighted = option.id === activeId
                      return (
                        <button
                          key={option.id}
                          id={`model-option-${option.id}`}
                          data-model-option={option.id}
                          type="button"
                          disabled={disabled || unavailable}
                          onClick={() => selectModel(option.id)}
                          onMouseEnter={() =>
                            setActiveIndex(visibleIds.indexOf(option.id))
                          }
                          aria-pressed={selectedOption}
                          className={cn(
                            'flex min-h-14 w-full min-w-0 items-center gap-3 px-3 py-2 text-left font-sans text-base text-ink hover:bg-surface-hover active:bg-surface-selected focus-visible:ring-2 focus-visible:ring-rule-focus focus-visible:outline-none focus-visible:ring-inset disabled:pointer-events-none disabled:opacity-40',
                            selectedOption && 'bg-surface-selected',
                            highlighted &&
                              !selectedOption &&
                              'bg-surface-hover',
                            highlighted && 'ring-1 ring-inset ring-rule-focus',
                          )}
                        >
                          <span className="min-w-0 flex-1 truncate font-medium">
                            {formatModelLabel(option.label)}
                          </span>
                          {selectedOption ? (
                            <Check
                              className="size-5 shrink-0 text-ink"
                              aria-hidden
                            />
                          ) : null}
                        </button>
                      )
                    })
                  ) : (
                    <div className="px-3 py-4 font-sans text-base text-ink-faint">
                      {unavailable
                        ? 'Provider not loaded.'
                        : 'No models available.'}
                    </div>
                  )}
                </div>
              </section>
            )
          })
        )}
      </div>

      <button
        type="button"
        disabled={disabled || loading || !selected || !onOpenReasoning}
        onClick={onOpenReasoning}
        className="sticky bottom-0 flex min-h-16 w-full shrink-0 items-center justify-between gap-3 border-t border-edge bg-panel-raised px-4 py-2 text-left font-sans text-base text-ink shadow-[0_-8px_20px_rgba(0,0,0,0.08)] hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40 sm:min-h-14 sm:text-sm"
      >
        <span className="min-w-0 flex-1">
          <span className="block font-medium">Reasoning effort</span>
          <span className="block capitalize text-ink-faint">
            {thinkingLevel}
          </span>
        </span>
        <ChevronRight
          className="size-5 shrink-0 text-ink-faint sm:size-4"
          aria-hidden
        />
      </button>
    </div>
  )
}

interface ReasoningEffortPanelProps {
  model: ModelOption | undefined
  value: ThinkingLevel
  onChange: (next: ThinkingLevel) => void
  disabled?: boolean
  className?: string
}

export function ReasoningEffortPanel({
  model,
  value,
  onChange,
  disabled,
  className,
}: ReasoningEffortPanelProps) {
  const options = effortOptionsFor(model)
  const visibleOptions = options.length > 0 ? options : [DEFAULT_EFFORT]

  return (
    <div
      className={cn(
        'divide-y divide-edge overflow-hidden rounded-lg bg-surface ring-1 ring-inset ring-edge',
        className,
      )}
    >
      {visibleOptions.map((option) => {
        const selected = option.effort === value
        return (
          <button
            key={option.effort}
            type="button"
            aria-pressed={selected}
            title={option.description}
            disabled={disabled}
            onClick={() => onChange(option.effort)}
            className={cn(
              'flex min-h-14 w-full items-center gap-3 px-3 py-2 text-left font-sans text-base text-ink hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-rule-focus focus-visible:outline-none focus-visible:ring-inset disabled:pointer-events-none disabled:opacity-40',
              selected && 'bg-surface-selected',
            )}
          >
            <span className="min-w-0 flex-1">
              <span className="block font-medium capitalize">
                {option.effort}
              </span>
              {option.description ? (
                <span className="block text-base leading-relaxed text-ink-faint sm:text-sm">
                  {option.description}
                </span>
              ) : null}
            </span>
            {selected ? (
              <Check className="size-5 shrink-0 text-ink" aria-hidden />
            ) : null}
          </button>
        )
      })}
    </div>
  )
}
