import * as DropdownMenuPrimitive from '@radix-ui/react-dropdown-menu'
import {
  ArrowLeft,
  Check,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Plus,
  RefreshCw,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { BottomSheet, BottomSheetContent } from '@/components/ui/BottomSheet'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
import { useMediaQuery } from '@/hooks/use-media-query'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import type { ProviderListEntry } from '@/lib/models-catalog'
import { cn } from '@/lib/utils'
import { useUnsavedGuard } from '@/pages/Configuration/tabs/WorkersTab/useUnsavedGuard'
import {
  type ModelId,
  type ModelOption,
  type ReasoningEffortOption,
  THINKING_LEVELS,
  type ThinkingLevel,
} from '@/types/chat'
import { AddProviderPanel } from './AddProviderPanel'
import {
  formatModelLabel,
  formatProviderLabel,
  providerForModel,
} from './model-picker-presentation'
import { ProviderConfigurationPanel } from './ProviderConfigurationPanel'
import { ProviderIcon } from './ProviderIcon'
import { ReasoningEffortSlider } from './ReasoningEffortSlider'

const DEFAULT_EFFORT: ReasoningEffortOption = {
  effort: 'default',
  description: 'use the model default',
}

const FILTER_KEYS = ['ArrowDown', 'ArrowUp', 'Home', 'End', 'Enter', 'Escape']
const MODEL_PICKER_PAGE_TRANSITION_MS = 250
/** After a rail tap the scroll spy stays quiet until the smooth scroll lands. */
const RAIL_JUMP_SETTLE_MS = 700

export interface ModelPickerProps {
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
  /** Copy shown when models exist but this optional field has no selection. */
  placeholder?: string
  /** Worker/profile pickers can hide provider setup owned by chat settings. */
  showProviderConfiguration?: boolean
  /** Worker/profile pickers can choose a model without configuring effort. */
  showReasoningEffort?: boolean
  className?: string
  /** Styling hook for compact placements such as the chat composer. */
  triggerClassName?: string
  /**
   * `subtle` renders the trigger as quiet text (no chevron, no chrome) that
   * only gains a background on hover — the chat composer's model label.
   */
  triggerAppearance?: 'default' | 'subtle'
}

interface PickerSubpageHeaderProps {
  title: string
  description: string
  onBack?: () => void
}

function PickerSubpageHeader({
  title,
  description,
  onBack,
}: PickerSubpageHeaderProps) {
  return (
    <div className="flex shrink-0 items-start gap-2 px-4 py-3 pr-12">
      {onBack ? (
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
      ) : null}
      <div className={cn('min-w-0 flex-1', onBack && 'pt-0.5')}>
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

function providerDisplayName(
  provider: ProviderListEntry | undefined,
  id: string,
): string {
  return provider?.display_name ?? formatProviderLabel(id) ?? id
}

type PickerPage = 'models' | 'provider' | 'add-provider'

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
  placeholder = 'Choose model',
  showProviderConfiguration = true,
  showReasoningEffort = true,
  className,
  triggerClassName,
  triggerAppearance = 'default',
}: ModelPickerProps) {
  const ctx = useConversationsCtxOptional()
  const mobileSheet = useMediaQuery('(max-width: 767px)')
  const [open, setOpen] = useState(false)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const consumedOpenRequestRef = useRef(openRequest)
  const [configurationProvider, setConfigurationProvider] = useState<
    string | null
  >(null)
  const [renderedConfigurationProvider, setRenderedConfigurationProvider] =
    useState<string | null>(null)
  const [addProviderOpen, setAddProviderOpen] = useState(false)
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
  const subtle = triggerAppearance === 'subtle'
  const showEffort = selectedEfforts.length > 1 && thinkingLevel !== 'default'
  const triggerLabel = selected
    ? subtle
      ? formatModelLabel(selected.label)
      : selected.label
    : loading
      ? 'Loading…'
      : options.length > 0
        ? placeholder
        : 'No models'

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
    setAddProviderOpen(false)
    setOpen(true)
  }, [openRequest, pickerDisabled])

  useEffect(() => {
    if (pickerDisabled && open) setOpen(false)
  }, [pickerDisabled, open])

  useEffect(() => {
    if (configurationProvider !== null) return
    if (renderedConfigurationProvider === null) return
    const timeout = window.setTimeout(
      () => setRenderedConfigurationProvider(null),
      MODEL_PICKER_PAGE_TRANSITION_MS,
    )
    return () => window.clearTimeout(timeout)
  }, [configurationProvider, renderedConfigurationProvider])

  const activePage: PickerPage = configurationProvider
    ? 'provider'
    : addProviderOpen
      ? 'add-provider'
      : 'models'

  function handleOpenChange(nextOpen: boolean) {
    if (nextOpen) {
      setOpen(true)
      return
    }
    configurationGuard.tryNavigate(() => {
      setOpen(false)
      setConfigurationProvider(null)
      setAddProviderOpen(false)
    })
  }

  function openProviderConfiguration(providerId: string) {
    setAddProviderOpen(false)
    setRenderedConfigurationProvider(providerId)
    setConfigurationProvider(providerId)
  }

  const sheetHeading =
    activePage === 'models'
      ? 'Model'
      : activePage === 'add-provider'
        ? 'Add a provider'
        : renderedConfigurationProvider
          ? (formatProviderLabel(renderedConfigurationProvider) ??
            renderedConfigurationProvider)
          : 'Provider configuration'

  const pickerTrigger = (
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
      aria-haspopup={mobileSheet ? 'dialog' : 'menu'}
      aria-expanded={open}
      data-state={open ? 'open' : 'closed'}
      disabled={pickerDisabled}
      onClick={mobileSheet ? () => handleOpenChange(!open) : undefined}
      title={subtle && selected ? selected.label : undefined}
      className={cn(
        subtle
          ? // Quiet text that reads as part of the composer surface; the
            // hover/open chip is background only, no border, no chevron.
            'flex h-10 min-w-0 items-center gap-x-1 rounded-full border border-transparent bg-transparent px-2.5 font-sans text-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus data-[state=open]:bg-surface-hover data-[state=open]:text-ink sm:h-8 sm:text-[13px]'
          : 'flex h-12 min-w-0 flex-1 items-center justify-between gap-x-2 rounded-sm border border-transparent bg-transparent px-3 font-sans text-base text-ink-faint hover:bg-surface-hover hover:text-ink focus:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus data-[state=open]:bg-surface data-[state=open]:text-ink sm:h-9 sm:text-[13px]',
        pickerDisabled && 'pointer-events-none opacity-40',
        triggerClassName,
      )}
    >
      <span className="flex min-w-0 flex-1 items-baseline gap-1.5 overflow-hidden whitespace-nowrap text-left">
        <span
          className={cn(
            'min-w-0 flex-1 truncate',
            !selected && 'text-ink-faint',
          )}
        >
          {triggerLabel}
        </span>
        {showEffort ? (
          subtle ? (
            <span className="shrink-0 capitalize">{thinkingLevel}</span>
          ) : (
            <span className="shrink-0 text-[11px] text-ink-faint">
              · {thinkingLevel}
            </span>
          )
        ) : null}
      </span>
      {subtle ? null : open ? (
        <ChevronUp size={16} aria-hidden />
      ) : (
        <ChevronDown size={16} aria-hidden />
      )}
    </button>
  )

  const pickerPages = (
    <div className="relative min-h-0 flex-1 overflow-hidden">
      <div
        data-active={activePage === 'models'}
        aria-hidden={activePage !== 'models'}
        inert={activePage !== 'models'}
        className={cn(
          'iii-ui-motion-picker-page absolute inset-0 flex min-h-0 flex-col [--picker-page-offset:calc(var(--distance-base)*-1)]',
          !mobileSheet && 'pt-4',
        )}
      >
        <ModelPickerPanel
          value={value}
          options={options}
          thinkingLevel={thinkingLevel}
          onChange={onChange}
          onThinkingLevelChange={onThinkingLevelChange}
          onConfigureProvider={
            showProviderConfiguration ? openProviderConfiguration : undefined
          }
          onAddProvider={
            showProviderConfiguration
              ? () => setAddProviderOpen(true)
              : undefined
          }
          showReasoningEffort={showReasoningEffort}
          disabled={disabled}
          loading={loading}
          autoFocusFilter={!mobileSheet}
        />
      </div>

      <div
        data-active={activePage === 'add-provider'}
        aria-hidden={activePage !== 'add-provider'}
        inert={activePage !== 'add-provider'}
        className="iii-ui-motion-picker-page absolute inset-0 flex min-h-0 flex-col [--picker-page-offset:var(--distance-base)]"
      >
        <PickerSubpageHeader
          title="Add a provider"
          description="Provider workers from the workers registry."
          onBack={() => setAddProviderOpen(false)}
        />
        {addProviderOpen ? (
          <AddProviderPanel
            disabled={disabled}
            onConfigureProvider={
              showProviderConfiguration ? openProviderConfiguration : undefined
            }
          />
        ) : null}
      </div>

      <div
        data-active={activePage === 'provider'}
        aria-hidden={activePage !== 'provider'}
        inert={activePage !== 'provider'}
        className="iii-ui-motion-picker-page absolute inset-0 flex min-h-0 flex-col [--picker-page-offset:var(--distance-base)]"
      >
        {renderedConfigurationProvider ? (
          <>
            <PickerSubpageHeader
              title={
                formatProviderLabel(renderedConfigurationProvider) ??
                renderedConfigurationProvider
              }
              description="Credentials and provider-specific settings."
              onBack={() =>
                configurationGuard.tryNavigate(() =>
                  setConfigurationProvider(null),
                )
              }
            />
            <ProviderConfigurationPanel
              providerId={renderedConfigurationProvider}
              onDirtyChange={configurationGuard.setDirty}
            />
          </>
        ) : null}
      </div>
    </div>
  )

  return (
    <span className={cn('flex min-w-0 items-center gap-1', className)}>
      {mobileSheet ? (
        <>
          {pickerTrigger}
          <BottomSheet open={open} onOpenChange={handleOpenChange}>
            <BottomSheetContent
              heading={sheetHeading}
              headerClassName={activePage === 'models' ? undefined : 'sr-only'}
              closeLabel="Close model picker"
              className="h-[min(82dvh,720px)]"
            >
              {pickerPages}
            </BottomSheetContent>
          </BottomSheet>
        </>
      ) : (
        <DropdownMenuPrimitive.Root open={open} onOpenChange={handleOpenChange}>
          <DropdownMenuPrimitive.Trigger asChild disabled={pickerDisabled}>
            {pickerTrigger}
          </DropdownMenuPrimitive.Trigger>

          <DropdownMenuPrimitive.Portal>
            <DropdownMenuPrimitive.Content
              sideOffset={4}
              // The composer's quiet trigger is right-aligned and grows to the
              // left as the effort label changes, so anchoring its end edge
              // keeps the open menu still while the slider moves.
              align={subtle ? 'end' : 'start'}
              collisionPadding={12}
              className="iii-ui-motion-dropdown z-50 flex h-[min(60vh,540px)] w-[min(400px,calc(100vw-24px))] flex-col overflow-hidden rounded-lg border border-edge bg-panel-raised text-ink shadow-floating"
            >
              {pickerPages}
            </DropdownMenuPrimitive.Content>
          </DropdownMenuPrimitive.Portal>
        </DropdownMenuPrimitive.Root>
      )}

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

interface ProviderRailProps {
  groups: readonly { label: string }[]
  providerById: ReadonlyMap<string, ProviderListEntry>
  activeGroup: string | null
  onJump: (label: string) => void
  onAdd?: () => void
  disabled?: boolean
}

/**
 * One glyph per provider group, in list order, plus the add affordance at
 * the end. Tapping a glyph scrolls the list to that group; the highlighted
 * glyph follows the group under the top of the list as it scrolls.
 */
function ProviderRail({
  groups,
  providerById,
  activeGroup,
  onJump,
  onAdd,
  disabled,
}: ProviderRailProps) {
  return (
    <nav
      aria-label="providers"
      className="flex w-14 shrink-0 flex-col items-center gap-1 overflow-y-auto pb-3 pl-2 pr-1 sm:w-12"
    >
      {groups.map((group) => {
        const provider = providerById.get(group.label)
        const label = providerDisplayName(provider, group.label)
        const active = group.label === activeGroup
        return (
          <Tooltip key={group.label}>
            <TooltipTrigger asChild>
              <button
                type="button"
                aria-label={label}
                aria-current={active ? 'true' : undefined}
                data-provider-rail={group.label}
                onClick={() => onJump(group.label)}
                className={cn(
                  'flex size-11 shrink-0 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus sm:size-9',
                  active &&
                    'bg-surface-selected text-ink hover:bg-surface-selected',
                  provider?.available === false && 'opacity-50',
                )}
              >
                <ProviderIcon
                  iconSvg={provider?.icon_svg}
                  label={label}
                  className="size-[18px] sm:size-4"
                />
              </button>
            </TooltipTrigger>
            <TooltipContent side="right">{label}</TooltipContent>
          </Tooltip>
        )
      })}
      {onAdd ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              aria-label="Add a provider"
              disabled={disabled}
              onClick={onAdd}
              className={cn(
                'flex size-11 shrink-0 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40 sm:size-9',
                groups.length > 0 && 'mt-1',
              )}
            >
              <Plus className="size-[18px] sm:size-4" aria-hidden />
            </button>
          </TooltipTrigger>
          <TooltipContent side="right">Add a provider</TooltipContent>
        </Tooltip>
      ) : null}
    </nav>
  )
}

interface ModelPickerPanelProps {
  value: ModelId | null
  options: ModelOption[]
  thinkingLevel: ThinkingLevel
  onChange: (next: ModelId) => void
  onThinkingLevelChange: (next: ThinkingLevel) => void
  onConfigureProvider?: (providerId: string) => void
  /** Opens the registry page; absent hides the add affordance. */
  onAddProvider?: () => void
  /** Render the effort slider under the list. Defaults on. */
  showReasoningEffort?: boolean
  /** Providers declared to the router; defaults to the chat context. */
  providers?: ProviderListEntry[]
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
  onAddProvider,
  showReasoningEffort = true,
  providers,
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
  // `null` follows the selected model, so the picker opens with one highlight
  // (the choice itself) and arrows walk from there; typing or hovering pins
  // an explicit row.
  const [activeIndex, setActiveIndex] = useState<number | null>(null)
  const [activeGroup, setActiveGroup] = useState<string | null>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const scrollFrameRef = useRef<number | null>(null)
  const railJumpRef = useRef<{ label: string; until: number } | null>(null)
  const optionsById = useMemo(
    () => new Map(options.map((option) => [option.id, option])),
    [options],
  )
  const safeValue = value != null && optionsById.has(value) ? value : undefined
  const selected = safeValue ? optionsById.get(safeValue) : undefined
  const selectedEfforts = useMemo(() => effortOptionsFor(selected), [selected])
  const presentProviders = providers ?? ctx?.presentProviders ?? []
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
    // A provider with no chat models is listed only while it still needs
    // setup: a configured one without chat models serves another modality
    // (speech, embeddings) and has nothing to offer this picker.
    ...presentIds
      .filter((id) => !grouped.has(id))
      .filter((id) => providerById.get(id)?.configured !== true)
      .map((id) => ({ label: id, options: [] })),
  ]
    .filter((group) => filterWords.length === 0 || group.options.length > 0)
    .sort((a, b) => a.label.localeCompare(b.label))
  const groupsKey = groups.map((group) => group.label).join(' ')
  const visibleIds = groups.flatMap((group) =>
    group.options.map((option) => option.id),
  )
  const resolvedActiveIndex =
    activeIndex ?? Math.max(0, safeValue ? visibleIds.indexOf(safeValue) : 0)
  const activeId =
    visibleIds[Math.min(resolvedActiveIndex, visibleIds.length - 1)]

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

  // The rail highlights the group whose heading sits at (or just above) the
  // top of the list — or the last group once the list is scrolled to the end,
  // since a short trailing group can never reach the top on its own.
  const syncActiveGroup = useCallback(() => {
    const list = listRef.current
    if (!list) return
    const pinned = railJumpRef.current
    if (pinned && Date.now() < pinned.until) {
      setActiveGroup(pinned.label)
      return
    }
    railJumpRef.current = null
    const sections = Array.from(
      list.querySelectorAll<HTMLElement>('[data-provider-group]'),
    )
    if (sections.length === 0) {
      setActiveGroup(null)
      return
    }
    const scrollable = list.scrollHeight > list.clientHeight + 2
    const atEnd =
      scrollable && list.scrollTop + list.clientHeight >= list.scrollHeight - 2
    let current = sections[0]
    if (atEnd) {
      current = sections[sections.length - 1]
    } else {
      const threshold = list.scrollTop + 16
      for (const section of sections) {
        if (section.offsetTop <= threshold) current = section
        else break
      }
    }
    setActiveGroup(current.dataset.providerGroup ?? null)
  }, [])

  // biome-ignore lint/correctness/useExhaustiveDependencies: the spy reads the DOM, so it re-runs whenever the rendered groups (filter, catalog, providers, loading) change
  useEffect(() => {
    syncActiveGroup()
  }, [syncActiveGroup, groupsKey, loading])

  useEffect(
    () => () => {
      if (scrollFrameRef.current !== null) {
        window.cancelAnimationFrame(scrollFrameRef.current)
      }
    },
    [],
  )

  const onListScroll = () => {
    if (scrollFrameRef.current !== null) return
    scrollFrameRef.current = window.requestAnimationFrame(() => {
      scrollFrameRef.current = null
      syncActiveGroup()
    })
  }

  const jumpToGroup = (label: string) => {
    const list = listRef.current
    const section = list?.querySelector<HTMLElement>(
      `[data-provider-group="${CSS.escape(label)}"]`,
    )
    if (!list || !section) return
    const reduceMotion =
      typeof window.matchMedia === 'function' &&
      window.matchMedia('(prefers-reduced-motion: reduce)').matches
    railJumpRef.current = { label, until: Date.now() + RAIL_JUMP_SETTLE_MS }
    setActiveGroup(label)
    list.scrollTo({
      top: section.offsetTop,
      behavior: reduceMotion ? 'auto' : 'smooth',
    })
  }

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
      (Math.min(resolvedActiveIndex, visibleIds.length - 1) +
        step +
        visibleIds.length) %
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

  const showRail = onAddProvider !== undefined || groups.length > 0

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
      <div className="flex min-h-0 flex-1">
        {showRail ? (
          <ProviderRail
            groups={groups}
            providerById={providerById}
            activeGroup={activeGroup}
            onJump={jumpToGroup}
            onAdd={onAddProvider}
            disabled={disabled}
          />
        ) : null}
        <div
          id="model-picker-list"
          ref={listRef}
          onScroll={onListScroll}
          className={cn(
            'relative min-h-0 flex-1 space-y-4 overflow-y-auto pb-3 pr-3',
            showRail ? 'pl-1' : 'pl-3',
            contentClassName,
          )}
        >
          {loading ? (
            <div className="rounded-lg bg-surface px-3 py-4 font-sans text-base text-ink-faint sm:text-sm">
              Loading model catalog…
            </div>
          ) : groups.length === 0 ? (
            <div className="rounded-lg bg-surface px-3 py-4 font-sans text-base text-ink-faint sm:text-sm">
              {filterWords.length > 0
                ? `No model matches "${filter.trim()}".`
                : onAddProvider
                  ? 'No providers yet. Add one from the registry to get models.'
                  : 'No models are configured.'}
            </div>
          ) : (
            groups.map((group) => {
              const provider = providerById.get(group.label)
              const providerLabel = providerDisplayName(provider, group.label)
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
                <section
                  key={group.label}
                  aria-label={providerLabel}
                  data-provider-group={group.label}
                >
                  <div className="flex items-center justify-between gap-3 px-1 pb-2">
                    <h3 className="flex min-w-0 items-center gap-2 font-sans text-[11px] font-medium text-ink-ghost">
                      <ProviderIcon
                        iconSvg={provider?.icon_svg}
                        label={providerLabel}
                        className="text-ink-faint"
                      />
                      <span className="min-w-0 truncate">{providerLabel}</span>
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
                        className="flex min-h-16 w-full min-w-0 items-center gap-3 px-3 py-2 text-left font-sans text-base text-ink hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40 sm:min-h-12 sm:text-sm"
                      >
                        <span className="flex min-w-0 flex-1 flex-col">
                          <span className="font-medium">
                            Configure provider
                          </span>
                          <span className="text-sm leading-relaxed text-ink-faint sm:text-[12px]">
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
                              // Hover and keyboard focus share the lighter wash;
                              // the selected row keeps the stronger one, so the
                              // choice stays legible while the pointer roams.
                              'flex min-h-14 w-full min-w-0 items-center gap-3 px-3 py-2 text-left font-sans text-base text-ink hover:bg-surface-hover active:bg-surface-active focus-visible:outline-none disabled:pointer-events-none disabled:opacity-40 sm:min-h-10 sm:text-sm',
                              selectedOption
                                ? 'bg-surface-selected hover:bg-surface-selected'
                                : highlighted && 'bg-surface-hover',
                            )}
                          >
                            <span className="min-w-0 flex-1 truncate font-medium">
                              {formatModelLabel(option.label)}
                            </span>
                            {selectedOption ? (
                              <Check
                                className="size-5 shrink-0 text-ink sm:size-4"
                                aria-hidden
                              />
                            ) : null}
                          </button>
                        )
                      })
                    ) : (
                      <div className="px-3 py-4 font-sans text-base text-ink-faint sm:text-sm">
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
      </div>

      {showReasoningEffort ? (
        <div className="shrink-0 bg-surface px-4 py-3 sm:py-2.5">
          {selected && selectedEfforts.length > 1 ? (
            <ReasoningEffortSlider
              options={selectedEfforts}
              value={thinkingLevel}
              onChange={onThinkingLevelChange}
              disabled={disabled || loading}
            />
          ) : (
            <div className="flex min-h-6 items-baseline justify-between gap-3">
              <span className="font-sans text-sm font-medium text-ink-faint sm:text-[13px]">
                Reasoning effort
              </span>
              <span className="font-sans text-sm text-ink-ghost sm:text-[12px]">
                {selected ? 'Fixed for this model' : 'Choose a model to adjust'}
              </span>
            </div>
          )}
        </div>
      ) : null}
    </div>
  )
}
