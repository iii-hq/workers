import * as DropdownMenuPrimitive from '@radix-ui/react-dropdown-menu'
import {
  ArrowLeft,
  Check,
  ChevronDown,
  ChevronRight,
  Plus,
  RefreshCw,
} from 'lucide-react'
import {
  type KeyboardEvent,
  type RefObject,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { BottomSheet, BottomSheetContent } from '@/components/ui/BottomSheet'
import { useMediaQuery } from '@/hooks/use-media-query'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import {
  discoveryBlocksModels,
  type ProviderListEntry,
} from '@/lib/models-catalog'
import { NEUTRAL_SURFACE } from '@/lib/neutral-surface'
import {
  resolvePinnedModels,
  savePinnedModels,
  usePinnedModels,
  withPinnedEffort,
} from '@/lib/pinned-models'
import { cn } from '@/lib/utils'
import { useUnsavedGuard } from '@/pages/Configuration/tabs/WorkersTab/useUnsavedGuard'
import {
  type ModelId,
  type ModelOption,
  type ReasoningEffortOption,
  THINKING_LEVELS,
  THINKING_OFF,
  type ThinkingLevel,
} from '@/types/chat'
import { AddProviderPanel } from './AddProviderPanel'
import { ModelLibraryDialog } from './ModelLibraryDialog'
import { ModelMark } from './ModelMark'
import {
  buildProviderGroups,
  filterWordsOf,
  modellessCandidates,
  providerGroupState,
  useModellessProviders,
} from './model-picker-catalog'
import {
  formatEffortLabel,
  formatModelLabel,
  formatProviderLabel,
  providerForModel,
  providerPresentationFor,
} from './model-picker-presentation'
import { PinnedModelMenu } from './PinnedModelMenu'
import { ProviderConfigurationPanel } from './ProviderConfigurationPanel'
import { ProviderIcon } from './ProviderIcon'
import { ReasoningEffortSlider } from './ReasoningEffortSlider'
import { XaiDiscoveryStatus } from './XaiDiscoveryStatus'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

const DEFAULT_EFFORT: ReasoningEffortOption = {
  effort: 'default',
  description: 'use the model default',
}

const FILTER_KEYS = ['ArrowDown', 'ArrowUp', 'Home', 'End', 'Enter', 'Escape']
const MODEL_PICKER_PAGE_TRANSITION_MS = 250

export interface ModelPickerProps {
  value: ModelId | null
  options: ModelOption[]
  /** Incremented by a parent CTA to open this picker when its trigger is visible. */
  openRequest?: number
  thinkingLevel: ThinkingLevel
  onChange: (next: ModelId, thinkingLevel?: ThinkingLevel) => unknown
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
  /** Providers declared to the router; defaults to the chat context's list. */
  providers?: ProviderListEntry[]
  className?: string
  /** Styling hook for compact placements such as the chat composer. */
  triggerClassName?: string
  /**
   * `subtle` renders the trigger as quiet text with a disclosure chevron that
   * only gains a background on hover — the chat composer's model label.
   */
  triggerAppearance?: 'default' | 'subtle'
}

interface PickerSubpageHeaderProps {
  title: string
  description: string
  onBack?: () => void
  backButtonRef?: RefObject<HTMLButtonElement | null>
}

function PickerSubpageHeader({
  title,
  description,
  onBack,
  backButtonRef,
}: PickerSubpageHeaderProps) {
  return (
    <div className="flex shrink-0 items-start gap-2 px-4 py-3 pr-12">
      {onBack ? (
        <button
          ref={backButtonRef}
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
        <h2 className="font-inter text-lg font-semibold text-ink">{title}</h2>
        <p className="font-inter text-sm text-pretty text-ink-faint">
          {description}
        </p>
      </div>
    </div>
  )
}

function stopMenuTabPropagation(event: KeyboardEvent<HTMLElement>) {
  if (event.key === 'Tab') event.stopPropagation()
}

/**
 * The Off entry sits right after Default. `supportsThinkingOff` true offers
 * it, `false` hides it (the model always reasons or the provider has no
 * switch), unknown shows it disabled so the gap is visible, not silent.
 */
function offOptionFor(model: ModelOption): ReasoningEffortOption[] {
  if (model.supportsThinkingOff === false) return []
  return [
    {
      effort: THINKING_OFF,
      description:
        model.supportsThinkingOff === true
          ? 'no reasoning'
          : 'not confirmed for this model',
      disabled: model.supportsThinkingOff !== true,
    },
  ]
}

export function effortOptionsFor(
  model: ModelOption | undefined,
): ReasoningEffortOption[] {
  if (!model) return []
  if (model.reasoningEfforts && model.reasoningEfforts.length > 0) {
    return [
      DEFAULT_EFFORT,
      ...offOptionFor(model),
      ...model.reasoningEfforts.filter(
        (option) =>
          option.effort !== 'default' && option.effort !== THINKING_OFF,
      ),
    ]
  }
  if (!model.supportsThinking) return []
  return [
    DEFAULT_EFFORT,
    ...offOptionFor(model),
    ...THINKING_LEVELS.filter((effort) => effort !== 'default').map(
      (effort) => ({ effort }),
    ),
  ]
}

function effortSupported(
  options: ReasoningEffortOption[],
  effort: ThinkingLevel,
): boolean {
  return options.some((option) => option.effort === effort && !option.disabled)
}

/**
 * Hand a pick to the host and apply its effort once the host accepts it:
 * a synchronous host takes both at once, a promise-returning one (a model
 * switch mid-conversation) gets the effort only on `true`.
 */
function commitModelSelection(args: {
  next: ModelId
  effort: ThinkingLevel
  thinkingLevel: ThinkingLevel
  onChange: (next: ModelId, thinkingLevel?: ThinkingLevel) => unknown
  onThinkingLevelChange: (next: ThinkingLevel) => void
}): void {
  const { next, effort, thinkingLevel, onChange, onThinkingLevelChange } = args
  const result = onChange(next, effort)
  const applyEffort = () => {
    if (effort !== thinkingLevel) onThinkingLevelChange(effort)
  }
  if (result instanceof Promise) {
    void result.then(
      (accepted) => {
        if (accepted === true) applyEffort()
      },
      () => undefined,
    )
  } else {
    applyEffort()
  }
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
  providers,
  className,
  triggerClassName,
  triggerAppearance = 'default',
}: ModelPickerProps) {
  const ctx = useConversationsCtxOptional()
  const presentProviders = providers ?? ctx?.presentProviders ?? []
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
  // The library dialog behind All models: every model, the five slots.
  const [libraryOpen, setLibraryOpen] = useState(false)
  const configurationGuard = useUnsavedGuard()
  // The favorites: up to five, remembered per model with the effort they
  // were last used at; starred and unstarred from the menu's catalog view.
  const storedPinned = usePinnedModels()
  const effortByModel = useRef(new Map<ModelId, ThinkingLevel>())
  const providerBackRef = useRef<HTMLButtonElement | null>(null)
  const addProviderBackRef = useRef<HTMLButtonElement | null>(null)
  const configurationOriginRef = useRef<HTMLElement | null>(null)
  const addProviderOriginRef = useRef<HTMLElement | null>(null)
  const returnFocusRef = useRef<HTMLElement | null>(null)

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
  const pinned = useMemo(
    () => resolvePinnedModels(storedPinned, options, safeValue ?? null),
    [storedPinned, options, safeValue],
  )
  const pickerDisabled = disabled || loading
  const subtle = triggerAppearance === 'subtle'
  const showEffort = selectedEfforts.length > 1 && thinkingLevel !== 'default'
  const effortLabel = formatEffortLabel(thinkingLevel)
  // Who serves it: the worker's own name and mark, so an Anthropic API key
  // and a Claude Code subscription read differently on the same model.
  const provider = providerPresentationFor(safeValue, presentProviders)
  const triggerLabel = selected
    ? formatModelLabel(selected.label)
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

  useEffect(() => {
    if (activePage === 'provider') {
      const focusBackButton = () => {
        providerBackRef.current?.focus()
      }
      providerBackRef.current?.focus()
      const frame = window.requestAnimationFrame(focusBackButton)
      return () => window.cancelAnimationFrame(frame)
    }
    if (activePage === 'add-provider') {
      const focusBackButton = () => {
        addProviderBackRef.current?.focus()
      }
      addProviderBackRef.current?.focus()
      const frame = window.requestAnimationFrame(focusBackButton)
      return () => window.cancelAnimationFrame(frame)
    }
    const returnFocus = returnFocusRef.current
    if (activePage === 'models' && returnFocus) {
      returnFocusRef.current = null
      returnFocus.focus()
    }
  }, [activePage])

  function rememberActiveElement(): HTMLElement | null {
    return document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null
  }

  function openAddProvider() {
    addProviderOriginRef.current = rememberActiveElement()
    setAddProviderOpen(true)
  }

  function closeAddProvider() {
    returnFocusRef.current = addProviderOriginRef.current
    setAddProviderOpen(false)
  }

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
    configurationOriginRef.current = rememberActiveElement()
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
            ? `model: ${selected.label}${provider.label ? ` via ${provider.label}` : ''}, reasoning effort: ${thinkingLevel}`
            : 'model'
      }
      aria-busy={loading || undefined}
      aria-haspopup={mobileSheet ? 'dialog' : 'menu'}
      aria-expanded={open}
      data-state={open ? 'open' : 'closed'}
      disabled={pickerDisabled}
      onClick={mobileSheet ? () => handleOpenChange(!open) : undefined}
      title={
        selected
          ? `${selected.label}${provider.label ? ` · ${provider.label}` : ''}`
          : undefined
      }
      className={cn(
        subtle
          ? // Quiet text that reads as part of the composer surface; the
            // hover/open chip is background only, with a disclosure chevron.
            'flex h-10 min-w-0 items-center gap-x-1.5 rounded-lg px-2 font-inter text-sm text-ink hover:bg-surface-hover focus:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus data-[state=open]:bg-surface-hover sm:h-8'
          : 'flex h-12 min-w-0 flex-1 items-center justify-between gap-x-2 rounded-lg px-3 font-inter text-base text-ink hover:bg-surface-hover focus:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus data-[state=open]:bg-surface sm:h-9 sm:text-sm',
        pickerDisabled && 'pointer-events-none opacity-40',
        triggerClassName,
      )}
    >
      {selected ? (
        <ModelMark
          modelId={safeValue}
          modelLabel={selected.label}
          providerIconSvg={provider.iconSvg}
          providerLabel={provider.label ?? provider.id ?? '?'}
          providerId={provider.id}
          className="text-ink-faint"
        />
      ) : null}
      <span className="flex min-w-0 flex-1 items-baseline gap-x-1.5 overflow-hidden whitespace-nowrap text-left">
        <span
          className={cn(
            'min-w-0 truncate',
            !selected && 'text-ink-faint',
            selected && subtle && 'font-medium',
          )}
        >
          {triggerLabel}
        </span>
        {selected && (provider.label || showEffort) ? (
          <span className="min-w-0 truncate text-ink-faint">
            {provider.label}
            {provider.label && showEffort ? ' · ' : ''}
            {showEffort ? effortLabel : ''}
          </span>
        ) : null}
      </span>
      <ChevronDown
        aria-hidden
        className={cn(
          'size-4 shrink-0 text-ink-faint transition-transform duration-[var(--motion-duration-control)]',
          open && 'rotate-180',
        )}
      />
    </button>
  )

  // The models page: the sheet lists the whole catalog on one page; the
  // desktop menu is the favorites, with the catalog behind All models.
  const modelsPage = mobileSheet ? (
    <ModelPickerPanel
      value={value}
      options={options}
      thinkingLevel={thinkingLevel}
      onChange={(next, effort) => {
        const result = onChange(next, effort)
        if (result instanceof Promise) setOpen(false)
        return result
      }}
      onThinkingLevelChange={onThinkingLevelChange}
      onConfigureProvider={
        showProviderConfiguration ? openProviderConfiguration : undefined
      }
      onAddProvider={showProviderConfiguration ? openAddProvider : undefined}
      showReasoningEffort={showReasoningEffort}
      disabled={disabled}
      loading={loading}
      autoFocusFilter={false}
    />
  ) : (
    <PinnedModelMenu
      value={safeValue ?? null}
      options={options}
      providers={presentProviders}
      pinned={pinned}
      thinkingLevel={thinkingLevel}
      efforts={selectedEfforts}
      disabled={disabled}
      loading={loading}
      onSelect={(next, pinEffort) => {
        const nextEfforts = effortOptionsFor(optionsById.get(next))
        const wanted = pinEffort ?? effortByModel.current.get(next) ?? 'default'
        const effort = effortSupported(nextEfforts, wanted) ? wanted : 'default'
        effortByModel.current.set(next, effort)
        commitModelSelection({
          next,
          effort,
          thinkingLevel,
          onChange,
          onThinkingLevelChange,
        })
        setOpen(false)
      }}
      onThinkingLevelChange={(next) => {
        onThinkingLevelChange(next)
        if (safeValue) {
          effortByModel.current.set(safeValue, next)
          // A favorite remembers the effort it is used at.
          if (pinned.some((entry) => entry.id === safeValue)) {
            savePinnedModels(withPinnedEffort(pinned, safeValue, next))
          }
        }
      }}
      onPinnedChange={savePinnedModels}
      onConfigureProvider={
        showProviderConfiguration ? openProviderConfiguration : undefined
      }
      onAddProvider={showProviderConfiguration ? openAddProvider : undefined}
      onEdit={() => {
        setOpen(false)
        setLibraryOpen(true)
      }}
    />
  )

  // The three pages stack in one grid cell: the active one is laid out
  // and the others slide away (`iii-ui-motion-picker-page`). Stacking
  // rather than absolute positioning lets the models page size the
  // desktop popover to its rows; the subpages mount only when open, so
  // they add no height until then.
  const pickerPages = (
    <div className="grid min-h-0 flex-1 grid-cols-1 grid-rows-[minmax(0,1fr)] overflow-hidden">
      <div
        data-active={activePage === 'models'}
        aria-hidden={activePage !== 'models'}
        inert={activePage !== 'models'}
        onKeyDown={mobileSheet ? undefined : stopMenuTabPropagation}
        className="iii-ui-motion-picker-page col-start-1 row-start-1 flex min-h-0 flex-col [--picker-page-offset:calc(var(--distance-base)*-1)]"
      >
        {modelsPage}
      </div>

      <div
        data-active={activePage === 'add-provider'}
        aria-hidden={activePage !== 'add-provider'}
        inert={activePage !== 'add-provider'}
        onKeyDown={stopMenuTabPropagation}
        className="iii-ui-motion-picker-page col-start-1 row-start-1 flex min-h-0 flex-col [--picker-page-offset:var(--distance-base)]"
      >
        {addProviderOpen ? (
          <>
            <PickerSubpageHeader
              title="Add a provider"
              description="Provider workers from the workers registry."
              backButtonRef={addProviderBackRef}
              onBack={closeAddProvider}
            />
            <AddProviderPanel
              disabled={disabled}
              onConfigureProvider={
                showProviderConfiguration
                  ? openProviderConfiguration
                  : undefined
              }
            />
          </>
        ) : null}
      </div>

      <div
        data-active={activePage === 'provider'}
        aria-hidden={activePage !== 'provider'}
        inert={activePage !== 'provider'}
        onKeyDown={stopMenuTabPropagation}
        className="iii-ui-motion-picker-page col-start-1 row-start-1 flex min-h-0 flex-col [--picker-page-offset:var(--distance-base)]"
      >
        {renderedConfigurationProvider ? (
          <>
            <PickerSubpageHeader
              title={
                formatProviderLabel(renderedConfigurationProvider) ??
                renderedConfigurationProvider
              }
              description="Credentials and provider-specific settings."
              backButtonRef={providerBackRef}
              onBack={() =>
                configurationGuard.tryNavigate(() => {
                  returnFocusRef.current = configurationOriginRef.current
                  setConfigurationProvider(null)
                })
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
              sideOffset={6}
              align="start"
              collisionPadding={12}
              className={cn(
                NEUTRAL_SURFACE,
                'iii-ui-motion-dropdown z-50 flex w-[min(360px,calc(100vw-24px))] flex-col overflow-hidden rounded-xl bg-popover font-inter text-ink shadow-md ring-1 ring-foreground/10',
                // Sized to its rows around the favorites; the provider
                // pages take the full height so their lists scroll inside.
                activePage !== 'models'
                  ? 'h-[min(70vh,560px)]'
                  : 'max-h-[min(70vh,560px)]',
              )}
            >
              {pickerPages}
            </DropdownMenuPrimitive.Content>
          </DropdownMenuPrimitive.Portal>
        </DropdownMenuPrimitive.Root>
      )}

      <ModelLibraryDialog
        open={libraryOpen}
        onOpenChange={setLibraryOpen}
        value={safeValue ?? null}
        options={options}
        providers={presentProviders}
        pinned={pinned}
        thinkingLevel={thinkingLevel}
        onPinnedChange={savePinnedModels}
        disabled={disabled}
      />

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
  onChange: (next: ModelId, thinkingLevel?: ThinkingLevel) => unknown
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
  const listRef = useRef<HTMLDivElement>(null)
  const optionsById = useMemo(
    () => new Map(options.map((option) => [option.id, option])),
    [options],
  )
  const safeValue = value != null && optionsById.has(value) ? value : undefined
  const selected = safeValue ? optionsById.get(safeValue) : undefined
  const selectedEfforts = useMemo(() => effortOptionsFor(selected), [selected])
  const presentProviders = providers ?? ctx?.presentProviders ?? []
  const providerById = new Map(
    presentProviders.map((provider) => [provider.id, provider]),
  )
  const modelless = useModellessProviders(
    modellessCandidates(presentProviders, options),
  )
  const filterWords = filterWordsOf(filter)
  const groups = buildProviderGroups({
    options,
    providers: presentProviders,
    modelless,
    words: filterWords,
  })
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
    const provider = providerById.get(providerForModel(next) ?? '')
    if (provider?.available === false || discoveryBlocksModels(provider)) return
    const nextModel = optionsById.get(next)
    const nextEfforts = effortOptionsFor(nextModel)
    const remembered = effortByModel.current.get(next) ?? 'default'
    const nextEffort = effortSupported(nextEfforts, remembered)
      ? remembered
      : 'default'
    commitModelSelection({
      next,
      effort: nextEffort,
      thinkingLevel,
      onChange,
      onThinkingLevelChange,
    })
  }

  return (
    <div className={cn('flex min-h-0 flex-1 flex-col', className)}>
      <div className="shrink-0 px-3 pb-2">
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
          placeholder="Search models…"
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
          className="h-9 w-full rounded-lg bg-surface px-3 font-inter text-sm text-ink outline-none placeholder:text-ink-faint focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-rule-focus"
        />
      </div>
      <div
        id="model-picker-list"
        ref={listRef}
        className={cn(
          'relative min-h-0 flex-1 overflow-y-auto px-2 pb-2',
          contentClassName,
        )}
      >
        {loading ? (
          <div className="px-2 py-3 font-inter text-sm text-ink-faint">
            Loading model catalog…
          </div>
        ) : groups.length === 0 ? (
          <div className="px-2 py-3 font-inter text-sm text-ink-faint">
            {filterWords.length > 0
              ? `No model matches "${filter.trim()}".`
              : onAddProvider
                ? 'No providers yet. Add one from the registry to get models.'
                : 'No models are configured.'}
          </div>
        ) : (
          groups.map((group) => {
            const provider = providerById.get(group.label)
            const { providerLabel, unavailable, configured, catalogIsUsable } =
              providerGroupState(group, provider)
            return (
              <section
                key={group.label}
                aria-label={providerLabel}
                data-provider-group={group.label}
                className="pt-2 first:pt-0"
              >
                {/* The group label: the provider's mark and its own name
                    (`Anthropic`, `Claude Code (subscription)`), so the same
                    model family under two credentials reads as two. */}
                <div className="flex h-8 items-center justify-between gap-3 px-2">
                  <h3 className="flex min-w-0 items-center gap-2 font-inter text-xs font-medium text-ink-faint">
                    <ProviderIcon
                      iconSvg={provider?.icon_svg}
                      label={providerLabel}
                      providerId={group.label}
                      className="text-ink-faint"
                    />
                    <span className="min-w-0 truncate">{providerLabel}</span>
                  </h3>
                  {(configured || catalogIsUsable) && onConfigureProvider ? (
                    <button
                      type="button"
                      onClick={() => onConfigureProvider(group.label)}
                      className="shrink-0 rounded-sm font-inter text-xs font-medium text-ink-faint hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
                    >
                      Configure
                    </button>
                  ) : null}
                </div>
                {group.label === 'xai' && provider?.discovery ? (
                  <XaiDiscoveryStatus
                    status={provider.discovery}
                    onRetry={ctx ? () => ctx.refreshModels('xai') : undefined}
                    unavailable={provider.available === false}
                  />
                ) : null}
                {!catalogIsUsable && !configured && onConfigureProvider ? (
                  <button
                    type="button"
                    disabled={disabled}
                    onClick={() => onConfigureProvider(group.label)}
                    className="flex min-h-14 w-full min-w-0 items-center gap-3 rounded-md px-2 py-2 text-left font-inter text-base text-ink hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40 sm:min-h-12 sm:text-sm"
                  >
                    <span className="flex min-w-0 flex-1 flex-col">
                      <span className="font-medium">Configure provider</span>
                      <span className="text-xs leading-relaxed text-ink-faint">
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
                          'flex min-h-11 w-full min-w-0 items-center gap-3 rounded-md px-2 py-1.5 text-left font-inter text-base text-ink hover:bg-surface-hover active:bg-surface-active focus-visible:outline-none disabled:pointer-events-none disabled:opacity-40 sm:min-h-8 sm:text-sm',
                          selectedOption
                            ? 'bg-surface-selected hover:bg-surface-selected'
                            : highlighted && 'bg-surface-hover',
                        )}
                      >
                        <span className="min-w-0 flex-1 truncate">
                          {formatModelLabel(option.label)}
                        </span>
                        {selectedOption ? (
                          <Check
                            className="size-4 shrink-0 text-ink"
                            aria-hidden
                          />
                        ) : null}
                      </button>
                    )
                  })
                ) : group.label === 'xai' && provider?.discovery ? null : (
                  <div className="px-2 py-2 font-inter text-sm text-ink-faint">
                    {unavailable
                      ? 'Provider not loaded.'
                      : (provider?.credential_error ??
                        (configured
                          ? 'No models with this key. Configure to check it.'
                          : 'No models available.'))}
                  </div>
                )}
              </section>
            )
          })
        )}
        {onAddProvider ? (
          <button
            type="button"
            aria-label="Add a provider"
            disabled={disabled}
            onClick={onAddProvider}
            className={cn(
              'flex min-h-11 w-full items-center gap-3 rounded-md px-2 py-1.5 text-left font-inter text-base text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40 sm:min-h-8 sm:text-sm',
              groups.length > 0 && 'mt-2',
            )}
          >
            <Plus className="size-4 shrink-0" aria-hidden />
            <span className="min-w-0 flex-1 truncate">Add a provider</span>
          </button>
        ) : null}
      </div>

      {showReasoningEffort ? (
        <div className="shrink-0 bg-surface px-3 py-3 sm:py-2.5">
          {selected && selectedEfforts.length > 1 ? (
            <ReasoningEffortSlider
              options={selectedEfforts}
              value={thinkingLevel}
              onChange={onThinkingLevelChange}
              disabled={disabled || loading}
            />
          ) : (
            <div className="flex min-h-6 items-baseline justify-between gap-3">
              <span className="font-inter text-sm font-medium text-ink-faint">
                Reasoning effort
              </span>
              <span className="font-inter text-sm text-ink-faint">
                {selected ? 'Fixed for this model' : 'Choose a model to adjust'}
              </span>
            </div>
          )}
        </div>
      ) : null}
    </div>
  )
}
