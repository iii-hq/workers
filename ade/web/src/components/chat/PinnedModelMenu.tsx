import {
  Check,
  ChevronRight,
  GripVertical,
  LayoutGrid,
  Plus,
  Search,
  Star,
} from 'lucide-react'
import {
  type DragControls,
  Reorder,
  useDragControls,
  useReducedMotion,
} from 'motion/react'
import {
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode,
  useEffect,
  useRef,
  useState,
} from 'react'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import type { ProviderListEntry } from '@/lib/models-catalog'
import {
  isPinned,
  movePinnedModel,
  PINNED_MODELS_MAX,
  type PinnedModel,
  pinModel,
  unpinModel,
} from '@/lib/pinned-models'
import { cn } from '@/lib/utils'
import type {
  ModelId,
  ModelOption,
  ReasoningEffortOption,
  ThinkingLevel,
} from '@/types/chat'
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
} from './model-picker-presentation'
import { ProviderIcon } from './ProviderIcon'
import { ReasoningEffortSlider } from './ReasoningEffortSlider'
import { XaiDiscoveryStatus } from './XaiDiscoveryStatus'

/** The chord the composer listens for; the footer spells it. */
export const CYCLE_EFFORT_BINDING = 'Mod+Shift+/'

const NAV_KEYS = ['ArrowDown', 'ArrowUp', 'Home', 'End', 'Enter', 'Escape']
/** How long the "favorites are full" notice stays up. */
const LIMIT_NOTICE_MS = 4_000

interface PinnedModelMenuProps {
  value: ModelId | null
  options: ModelOption[]
  providers: ProviderListEntry[]
  pinned: PinnedModel[]
  thinkingLevel: ThinkingLevel
  /** The effort levels the selected model offers; fewer than two = fixed. */
  efforts: ReasoningEffortOption[]
  disabled?: boolean
  loading?: boolean
  /** Pick a model; `effort` is the favorite's, `undefined` lets the picker remember. */
  onSelect: (id: ModelId, effort: ThinkingLevel | undefined) => void
  onThinkingLevelChange: (next: ThinkingLevel) => void
  onPinnedChange: (next: PinnedModel[]) => void
  /** Open a provider's configuration; absent hides the Configure links. */
  onConfigureProvider?: (providerId: string) => void
  /** Open the registry page; absent hides the footer's Add provider. */
  onAddProvider?: () => void
  /** Open the library dialog (All models): every model, the five slots. */
  onEdit?: () => void
  /** Put the caret in the search on mount (the dropdown does). */
  autoFocusSearch?: boolean
}

/** The favorites in a new order of ids; ids the list does not hold are dropped. */
export function reorderPinned(
  list: readonly PinnedModel[],
  ids: readonly ModelId[],
): PinnedModel[] {
  const byId = new Map(list.map((entry) => [entry.id, entry]))
  return ids.flatMap((id) => {
    const entry = byId.get(id)
    return entry ? [entry] : []
  })
}

/** The notice shown when a sixth star is attempted. */
export function favoritesFullNotice(label: string): string {
  return `Favorites are full (${PINNED_MODELS_MAX} of ${PINNED_MODELS_MAX}). Unstar one to add ${label}.`
}

/**
 * The grip on a favorite's row: drag it to move the row (the list slides
 * out of the way), or use the arrow keys, Home and End.
 */
function PinnedGrip({
  label,
  controls,
  onStep,
}: {
  label: string
  controls: DragControls
  onStep: (direction: 'up' | 'down' | 'first' | 'last') => void
}) {
  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    const step =
      event.key === 'ArrowUp'
        ? 'up'
        : event.key === 'ArrowDown'
          ? 'down'
          : event.key === 'Home'
            ? 'first'
            : event.key === 'End'
              ? 'last'
              : null
    if (!step) return
    event.preventDefault()
    event.stopPropagation()
    onStep(step)
  }
  return (
    <button
      type="button"
      aria-label={`Move ${label}`}
      title="Drag, or use the arrow keys"
      onPointerDown={(event: PointerEvent<HTMLButtonElement>) => {
        event.preventDefault()
        controls.start(event)
      }}
      onKeyDown={onKeyDown}
      className="flex size-8 shrink-0 cursor-grab touch-none items-center justify-center rounded-md text-ink-faint hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus active:cursor-grabbing"
    >
      <GripVertical aria-hidden className="size-4" />
    </button>
  )
}

/** One favorite's row, draggable by its grip; the list animates around it. */
function PinnedRow({
  id,
  label,
  reduceMotion,
  onStep,
  children,
}: {
  id: ModelId
  label: string
  reduceMotion: boolean
  onStep: (direction: 'up' | 'down' | 'first' | 'last') => void
  children: ReactNode
}) {
  const controls = useDragControls()
  const [dragging, setDragging] = useState(false)
  return (
    <Reorder.Item
      as="div"
      value={id}
      dragListener={false}
      dragControls={controls}
      onDragStart={() => setDragging(true)}
      onDragEnd={() => setDragging(false)}
      layout={reduceMotion ? undefined : true}
      transition={{
        layout: reduceMotion
          ? { duration: 0 }
          : { duration: 0.18, ease: [0.2, 0, 0, 1] },
      }}
      whileDrag={reduceMotion ? undefined : { scale: 1.01 }}
      className={cn(
        'relative flex items-center',
        dragging && 'z-10 rounded-lg bg-panel-raised shadow-raised',
      )}
    >
      <PinnedGrip label={label} controls={controls} onStep={onStep} />
      {children}
    </Reorder.Item>
  )
}

/**
 * The star beside a row: filled when the model is a favorite, hollow when
 * it is not. It is its own button, apart from the row's select button, so
 * starring never changes the selection and selecting never stars. When the
 * five are full a hollow star still answers — with the notice, not a swap.
 */
function StarButton({
  label,
  starred,
  full,
  disabled,
  onToggle,
}: {
  label: string
  starred: boolean
  full: boolean
  disabled?: boolean
  onToggle: () => void
}) {
  const title = starred
    ? `Unstar ${label}`
    : full
      ? `Favorites are full (${PINNED_MODELS_MAX} of ${PINNED_MODELS_MAX})`
      : `Star ${label}`
  return (
    <button
      type="button"
      aria-label={starred ? `Unstar ${label}` : `Star ${label}`}
      aria-pressed={starred}
      data-model-star={starred ? 'on' : 'off'}
      title={title}
      disabled={disabled}
      onClick={(event) => {
        event.stopPropagation()
        onToggle()
      }}
      className={cn(
        'flex size-8 shrink-0 items-center justify-center rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40',
        starred
          ? 'text-ink hover:bg-surface-hover'
          : // A hollow star only shows while the row is hovered or focused
            // within, so the catalog reads as a list of models, not of
            // stars; the filled ones stay put as the favorites' mark.
            'text-ink-faint opacity-0 hover:bg-surface-hover hover:text-ink focus-visible:opacity-100 group-focus-within:opacity-100 group-hover:opacity-100',
      )}
    >
      <Star aria-hidden className={cn('size-4', starred && 'fill-current')} />
    </button>
  )
}

/**
 * The composer's model menu: a search, up to five starred models (drag
 * the grip to reorder, the current one checked, each with the effort it
 * was last used at), the effort slider for the current model, and a
 * footer with All models (the library dialog: every model, the five slots
 * arranged by drag) and Add provider (the registry page). Typing searches
 * the whole catalog: the list becomes every matching model grouped under
 * its provider's mark and name, each with a star to add it to (or drop it
 * from) the favorites and a Configure link per provider — so a model
 * outside the five is one filter away.
 */
export function PinnedModelMenu({
  value,
  options,
  providers,
  pinned,
  thinkingLevel,
  efforts,
  disabled,
  loading,
  onSelect,
  onThinkingLevelChange,
  onPinnedChange,
  onConfigureProvider,
  onAddProvider,
  onEdit,
  autoFocusSearch = true,
}: PinnedModelMenuProps) {
  const ctx = useConversationsCtxOptional()
  const [filter, setFilter] = useState('')
  const [activeIndex, setActiveIndex] = useState<number | null>(null)
  const [limitNotice, setLimitNotice] = useState<string | null>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const reduceMotion = useReducedMotion() ?? false
  const optionsById = new Map(options.map((option) => [option.id, option]))
  const providerById = new Map(
    providers.map((provider) => [provider.id, provider]),
  )
  const providerOf = (id: ModelId) =>
    providerById.get(providerForModel(id) ?? '')
  const providerLabelOf = (id: ModelId) => {
    const providerId = providerForModel(id)
    return (
      providerOf(id)?.display_name ??
      formatProviderLabel(providerId) ??
      providerId ??
      ''
    )
  }
  const selectable = (id: ModelId) => {
    const group = providerGroupState(
      { label: providerForModel(id) ?? '—', options: [] },
      providerOf(id),
    )
    return !group.unavailable
  }

  const words = filterWordsOf(filter)
  const searching = words.length > 0
  // A typed filter is a question about every model, not about the five:
  // the list becomes the catalog, grouped by provider, each row starred.
  const showCatalog = searching
  const modelless = useModellessProviders(
    modellessCandidates(providers, options),
  )
  const groups = showCatalog
    ? buildProviderGroups({ options, providers, modelless, words })
    : []

  // The current model is always in the favorites list: when it is not one
  // of the five (an agent profile picked it, say) it leads them, unstarred.
  const favoriteRows: Array<{
    id: ModelId
    effort: ThinkingLevel | undefined
    pinned: boolean
  }> = showCatalog
    ? []
    : [
        ...(value &&
        optionsById.has(value) &&
        !pinned.some((p) => p.id === value)
          ? [{ id: value, effort: undefined, pinned: false }]
          : []),
        ...pinned
          .filter((entry) => optionsById.has(entry.id))
          .map((entry) => ({
            id: entry.id,
            effort: entry.effort,
            pinned: true,
          })),
      ]
  const visibleIds = showCatalog
    ? groups.flatMap((group) =>
        providerGroupState(group, providerById.get(group.label)).catalogIsUsable
          ? group.options.map((option) => option.id)
          : [],
      )
    : favoriteRows.map((row) => row.id)
  const resolvedActiveIndex =
    activeIndex ?? Math.max(0, value ? visibleIds.indexOf(value) : 0)
  const activeId =
    visibleIds[Math.min(resolvedActiveIndex, visibleIds.length - 1)]

  useEffect(() => {
    if (!activeId) return
    listRef.current
      ?.querySelector(`[data-model-option="${CSS.escape(activeId)}"]`)
      ?.scrollIntoView({ block: 'nearest' })
  }, [activeId])

  useEffect(() => {
    if (limitNotice === null) return
    const timer = window.setTimeout(() => setLimitNotice(null), LIMIT_NOTICE_MS)
    return () => window.clearTimeout(timer)
  }, [limitNotice])

  const full = pinned.length >= PINNED_MODELS_MAX

  const pick = (id: ModelId, effort: ThinkingLevel | undefined) => {
    if (disabled || loading || !selectable(id)) return
    onSelect(id, effort)
  }

  /** The effort a new favorite starts at: the live one for the current model. */
  const effortFor = (id: ModelId): ThinkingLevel =>
    id === value ? thinkingLevel : 'default'

  const toggleStar = (id: ModelId) => {
    if (disabled) return
    const label = formatModelLabel(optionsById.get(id)?.label ?? id)
    if (isPinned(pinned, id)) {
      setLimitNotice(null)
      onPinnedChange(unpinModel(pinned, id))
      return
    }
    if (full) {
      setLimitNotice(favoritesFullNotice(label))
      return
    }
    setLimitNotice(null)
    onPinnedChange(pinModel(pinned, { id, effort: effortFor(id) }))
  }

  const onSearchKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (!NAV_KEYS.includes(event.key)) return
    if (event.key === 'Escape') {
      if (filter === '') return
      event.preventDefault()
      event.stopPropagation()
      setFilter('')
      setActiveIndex(null)
      return
    }
    event.preventDefault()
    event.stopPropagation()
    if (event.key === 'Enter') {
      if (!activeId) return
      const favorite = favoriteRows.find((row) => row.id === activeId)
      pick(activeId, favorite?.effort)
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

  // The hovered row takes a lighter wash than the chosen one, so the two
  // never read as two selections.
  const rowClass = (id: ModelId) =>
    id === value
      ? 'bg-surface-selected'
      : id === activeId
        ? 'bg-surface-hover/50'
        : undefined

  /** The select button of a row: mark, name, the effort (favorites) and the check. */
  const renderSelect = (
    id: ModelId,
    rowEffort: ThinkingLevel | undefined,
    trailing?: ReactNode,
  ) => {
    const option = optionsById.get(id)
    if (!option) return null
    const current = id === value
    const provider = providerOf(id)
    const label = providerLabelOf(id)
    const effort = current ? thinkingLevel : rowEffort
    const effortLabel =
      effort === undefined
        ? ''
        : effort === 'default'
          ? 'Default'
          : formatEffortLabel(effort)
    return (
      <button
        type="button"
        id={`model-option-${id}`}
        data-model-option={id}
        disabled={disabled || !selectable(id)}
        aria-pressed={current}
        onClick={() => pick(id, rowEffort)}
        onMouseEnter={() => setActiveIndex(visibleIds.indexOf(id))}
        className="flex h-10 min-w-0 flex-1 items-center gap-2.5 rounded-md text-left focus-visible:outline-none disabled:opacity-40"
      >
        <ModelMark
          modelId={id}
          modelLabel={option.label}
          providerIconSvg={provider?.icon_svg}
          providerLabel={label}
          providerId={providerForModel(id)}
          className="text-ink-faint"
        />
        <span className="truncate">{formatModelLabel(option.label)}</span>
        {trailing}
        {effortLabel ? (
          <span className="truncate text-ink-faint">{effortLabel}</span>
        ) : null}
        <span className="ml-auto flex size-6 shrink-0 items-center justify-center">
          {current ? <Check className="size-4 text-ink" aria-hidden /> : null}
        </span>
      </button>
    )
  }

  const renderStar = (id: ModelId) => (
    <StarButton
      label={formatModelLabel(optionsById.get(id)?.label ?? id)}
      starred={isPinned(pinned, id)}
      full={full}
      disabled={disabled}
      onToggle={() => toggleStar(id)}
    />
  )

  const effortFixed = efforts.length < 2

  const favoritesEmpty = favoriteRows.length === 0
  const catalogEmpty = groups.length === 0

  return (
    <div className="flex min-h-0 flex-1 flex-col font-inter text-sm text-ink">
      <div className="flex h-11 shrink-0 items-center gap-2 border-b border-edge px-3">
        <Search className="size-4 shrink-0 text-ink-faint" aria-hidden />
        <input
          type="search"
          value={filter}
          onChange={(event) => {
            setFilter(event.target.value)
            setActiveIndex(0)
          }}
          onKeyDown={onSearchKeyDown}
          // biome-ignore lint/a11y/noAutofocus: the menu opened to pick a model; typing is the fastest way to one
          autoFocus={autoFocusSearch}
          placeholder="Search models"
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
          className="h-9 min-w-0 flex-1 bg-transparent text-sm text-ink outline-none placeholder:text-ink-faint"
        />
      </div>

      <div
        id="model-picker-list"
        ref={listRef}
        data-model-menu-view={showCatalog ? 'catalog' : 'favorites'}
        className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto p-1.5"
      >
        {loading ? (
          <div className="px-2 py-3 text-ink-faint">Loading model catalog…</div>
        ) : showCatalog ? (
          <>
            {catalogEmpty ? (
              <div className="px-2 py-3 text-ink-faint">
                {searching
                  ? `No model matches "${filter.trim()}".`
                  : onAddProvider
                    ? 'No providers yet. Add one to get models.'
                    : 'No models are configured.'}
              </div>
            ) : (
              groups.map((group) => {
                const provider = providerById.get(group.label)
                const state = providerGroupState(group, provider)
                return (
                  <section
                    key={group.label}
                    aria-label={state.providerLabel}
                    data-provider-group={group.label}
                    className="pt-1.5 first:pt-0"
                  >
                    {/* The group label: the provider's mark and its own
                        name (`Anthropic`, `Claude Code (subscription)`),
                        so the same model family under two credentials
                        reads as two. */}
                    <div className="flex h-8 items-center justify-between gap-3 px-2">
                      <h3 className="flex min-w-0 items-center gap-2 text-xs font-medium text-ink-faint">
                        <ProviderIcon
                          iconSvg={provider?.icon_svg}
                          label={state.providerLabel}
                          providerId={group.label}
                          className="text-ink-faint"
                        />
                        <span className="min-w-0 truncate">
                          {state.providerLabel}
                        </span>
                      </h3>
                      {(state.configured || state.catalogIsUsable) &&
                      onConfigureProvider ? (
                        <button
                          type="button"
                          onClick={() => onConfigureProvider(group.label)}
                          className="shrink-0 rounded-sm text-xs font-medium text-ink-faint hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
                        >
                          Configure
                        </button>
                      ) : null}
                    </div>
                    {group.label === 'xai' && provider?.discovery ? (
                      <XaiDiscoveryStatus
                        status={provider.discovery}
                        onRetry={
                          ctx ? () => ctx.refreshModels('xai') : undefined
                        }
                        unavailable={provider.available === false}
                      />
                    ) : null}
                    {!state.catalogIsUsable &&
                    !state.configured &&
                    onConfigureProvider ? (
                      <button
                        type="button"
                        disabled={disabled}
                        onClick={() => onConfigureProvider(group.label)}
                        className="flex h-10 w-full min-w-0 items-center gap-3 rounded-md px-2 text-left text-ink hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40"
                      >
                        <span className="min-w-0 flex-1 truncate">
                          Configure provider
                        </span>
                        <ChevronRight
                          className="size-4 shrink-0 text-ink-faint"
                          aria-hidden
                        />
                      </button>
                    ) : state.catalogIsUsable ? (
                      group.options.map((option) => (
                        <div
                          key={option.id}
                          className={cn(
                            'group flex h-10 shrink-0 items-center rounded-lg pr-1 pl-2 transition-colors duration-100',
                            rowClass(option.id),
                          )}
                        >
                          {renderSelect(option.id, undefined)}
                          {renderStar(option.id)}
                        </div>
                      ))
                    ) : group.label === 'xai' && provider?.discovery ? null : (
                      <div className="px-2 py-2 text-ink-faint">
                        {state.unavailable
                          ? 'Provider not loaded.'
                          : (provider?.credential_error ??
                            (state.configured
                              ? 'No models with this key. Configure to check it.'
                              : 'No models available.'))}
                      </div>
                    )}
                  </section>
                )
              })
            )}
          </>
        ) : favoritesEmpty ? (
          <div className="px-2 py-3 text-ink-faint">
            {options.length === 0
              ? 'No models yet. Add a provider to get some.'
              : 'No favorites yet. Star models from the search or All models.'}
          </div>
        ) : (
          <>
            {favoriteRows
              .filter((row) => !row.pinned)
              .map((row) => (
                <div
                  key={row.id}
                  className={cn(
                    'group flex h-10 shrink-0 items-center rounded-lg pr-1 transition-colors duration-100',
                    rowClass(row.id),
                  )}
                >
                  <span className="size-8 shrink-0" aria-hidden />
                  {renderSelect(row.id, row.effort)}
                  {renderStar(row.id)}
                </div>
              ))}
            <Reorder.Group
              as="div"
              axis="y"
              className="flex flex-col gap-0.5"
              values={pinned.map((entry) => entry.id)}
              onReorder={(ids: ModelId[]) =>
                onPinnedChange(reorderPinned(pinned, ids))
              }
            >
              {favoriteRows
                .filter((row) => row.pinned)
                .map((row) => {
                  const option = optionsById.get(row.id)
                  const label = option ? formatModelLabel(option.label) : row.id
                  const at = pinned.findIndex((entry) => entry.id === row.id)
                  return (
                    <div
                      key={row.id}
                      className={cn(
                        'group h-10 shrink-0 rounded-lg pr-1 transition-colors duration-100',
                        rowClass(row.id),
                      )}
                    >
                      {disabled ? (
                        <div className="flex h-10 items-center">
                          <span className="size-8 shrink-0" aria-hidden />
                          {renderSelect(row.id, row.effort)}
                          {renderStar(row.id)}
                        </div>
                      ) : (
                        <PinnedRow
                          id={row.id}
                          label={label}
                          reduceMotion={reduceMotion}
                          onStep={(direction) => {
                            const to =
                              direction === 'up'
                                ? at - 1
                                : direction === 'down'
                                  ? at + 1
                                  : direction === 'first'
                                    ? 0
                                    : pinned.length - 1
                            if (to === at || to < 0 || to >= pinned.length) {
                              return
                            }
                            onPinnedChange(movePinnedModel(pinned, row.id, to))
                          }}
                        >
                          {renderSelect(row.id, row.effort)}
                          {renderStar(row.id)}
                        </PinnedRow>
                      )}
                    </div>
                  )
                })}
            </Reorder.Group>
          </>
        )}
      </div>

      {limitNotice ? (
        <p
          role="status"
          data-favorites-limit-notice
          className="shrink-0 px-3 pb-2 text-xs text-ink-faint"
        >
          {limitNotice}
        </p>
      ) : null}

      <div className="shrink-0 border-t border-edge px-3 py-2.5">
        {value && !effortFixed ? (
          <ReasoningEffortSlider
            options={efforts}
            value={thinkingLevel}
            onChange={onThinkingLevelChange}
            disabled={disabled || loading}
            layout="stacked"
          />
        ) : (
          <div className="flex min-h-6 items-baseline justify-between gap-3">
            <span className="font-medium text-ink">Reasoning effort</span>
            <span className="text-ink-faint">
              {value ? 'Fixed for this model' : 'Choose a model to adjust'}
            </span>
          </div>
        )}
      </div>

      <div className="flex h-11 shrink-0 items-center justify-between gap-1 border-t border-edge px-1.5">
        {onEdit ? (
          <button
            type="button"
            title="Every model, and the five favorite slots to arrange"
            disabled={disabled}
            onClick={onEdit}
            className="flex h-8 items-center gap-1.5 rounded-lg px-2 text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40"
          >
            <LayoutGrid className="size-4" aria-hidden />
            All models
          </button>
        ) : (
          <span />
        )}
        {onAddProvider ? (
          <button
            type="button"
            aria-label="Add a provider"
            disabled={disabled}
            onClick={onAddProvider}
            className="flex h-8 items-center gap-1.5 rounded-lg px-2 text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40"
          >
            <Plus className="size-4" aria-hidden />
            Add provider
          </button>
        ) : null}
      </div>
    </div>
  )
}
