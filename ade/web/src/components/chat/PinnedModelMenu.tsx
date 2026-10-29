import {
  Check,
  ChevronRight,
  GripVertical,
  LayoutGrid,
  Plus,
  Search,
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
import { movePinnedModel, type PinnedModel } from '@/lib/pinned-models'
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
  /** Open the library dialog (Manage models): every model, the five
   *  favorite slots — where favorites are pinned, unpinned and arranged. */
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

/** One navigable row: arrows walk these, Enter picks the active one. A
 *  model can sit in the favorites AND under All models, so the key — not
 *  the model id — names the row. */
interface NavRow {
  key: string
  id: ModelId
  effort: ThinkingLevel | undefined
}

/**
 * The composer's model menu: a search, the favorites (up to five, drag the
 * grip to reorder, the current one checked, each with the effort it was
 * last used at), a collapsible All models section with the whole catalog
 * grouped under each provider's mark and name, the effort slider for the
 * current model, and a footer with Manage models (the library dialog,
 * where the five favorites are pinned and arranged) and Add provider (the
 * registry page). All models opens collapsed under the favorites and
 * expanded when there are none, so the menu stays short without hiding
 * the catalog. Typing searches the whole catalog: the list becomes every
 * matching model grouped by provider, with a Configure link per provider.
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
  const listRef = useRef<HTMLDivElement>(null)
  const allModelsRef = useRef<HTMLButtonElement>(null)
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
  // the list becomes the catalog, grouped by provider.
  const showCatalog = searching
  const modelless = useModellessProviders(
    modellessCandidates(providers, options),
  )
  const groups = showCatalog
    ? buildProviderGroups({ options, providers, modelless, words })
    : []
  // All models: the same grouped catalog, unfiltered, under the favorites.
  const allGroups = showCatalog
    ? []
    : buildProviderGroups({ options, providers, modelless, words: [] })

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
  const favoritesEmpty = favoriteRows.length === 0
  // Collapsed under the favorites; open from the start when there are none
  // to show, so the menu never opens on an empty list.
  const [allModelsOpen, setAllModelsOpen] = useState(favoritesEmpty)
  const showAllModels = !showCatalog && allGroups.length > 0

  const catalogRows = (list: typeof groups, prefix: string): NavRow[] =>
    list.flatMap((group) =>
      providerGroupState(group, providerById.get(group.label)).catalogIsUsable
        ? group.options.map((option) => ({
            key: `${prefix}${option.id}`,
            id: option.id,
            effort: undefined,
          }))
        : [],
    )
  const navRows: NavRow[] = showCatalog
    ? catalogRows(groups, '')
    : [
        ...favoriteRows.map((row) => ({
          key: `fav:${row.id}`,
          id: row.id,
          effort: row.effort,
        })),
        ...(showAllModels && allModelsOpen
          ? catalogRows(allGroups, 'all:')
          : []),
      ]
  const navKeys = navRows.map((row) => row.key)
  // Unpinned by default the highlight follows the current model's first row.
  const resolvedActiveIndex =
    activeIndex ??
    Math.max(0, value ? navRows.findIndex((row) => row.id === value) : 0)
  const activeRow = navRows[Math.min(resolvedActiveIndex, navRows.length - 1)]
  const activeKey = activeRow?.key

  useEffect(() => {
    if (!activeKey) return
    listRef.current
      ?.querySelector(`[data-model-row="${CSS.escape(activeKey)}"]`)
      ?.scrollIntoView({ block: 'nearest' })
  }, [activeKey])

  const toggleAllModels = () => {
    const next = !allModelsOpen
    setAllModelsOpen(next)
    setActiveIndex(null)
    // Opening scrolls the section to the top of the list, so the models it
    // reveals are in view rather than below the fold.
    if (next) {
      window.requestAnimationFrame(() =>
        allModelsRef.current?.scrollIntoView({
          block: 'start',
          behavior: reduceMotion ? 'auto' : 'smooth',
        }),
      )
    }
  }

  const pick = (id: ModelId, effort: ThinkingLevel | undefined) => {
    if (disabled || loading || !selectable(id)) return
    onSelect(id, effort)
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
      if (activeRow) pick(activeRow.id, activeRow.effort)
      return
    }
    if (event.key === 'Home' || event.key === 'End') {
      setActiveIndex(event.key === 'Home' ? 0 : navRows.length - 1)
      return
    }
    if (navRows.length === 0) return
    const step = event.key === 'ArrowDown' ? 1 : -1
    setActiveIndex(
      (Math.min(resolvedActiveIndex, navRows.length - 1) +
        step +
        navRows.length) %
        navRows.length,
    )
  }

  // The hovered row takes a lighter wash than the chosen one, so the two
  // never read as two selections.
  const rowClass = (key: string, id: ModelId) =>
    id === value
      ? 'bg-surface-selected'
      : key === activeKey
        ? 'bg-surface-hover/50'
        : undefined

  /** The select button of a row: mark, name, the effort (favorites) and the check. */
  const renderSelect = (
    key: string,
    id: ModelId,
    rowEffort: ThinkingLevel | undefined,
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
        id={`model-option-${key}`}
        data-model-option={id}
        data-model-row={key}
        disabled={disabled || !selectable(id)}
        aria-pressed={current}
        onClick={() => pick(id, rowEffort)}
        onMouseEnter={() => setActiveIndex(navKeys.indexOf(key))}
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
        {effortLabel ? (
          <span className="truncate text-ink-faint">{effortLabel}</span>
        ) : null}
        <span className="ml-auto flex size-6 shrink-0 items-center justify-center">
          {current ? <Check className="size-4 text-ink" aria-hidden /> : null}
        </span>
      </button>
    )
  }

  /** The catalog under each provider's mark and name — the search results
   *  and the All models section; `prefix` keys the rows apart. */
  const renderGroups = (list: typeof groups, prefix: string) =>
    list.map((group) => {
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
              <span className="min-w-0 truncate">{state.providerLabel}</span>
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
              onRetry={ctx ? () => ctx.refreshModels('xai') : undefined}
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
                  'flex h-10 shrink-0 items-center rounded-lg px-2 transition-colors duration-100',
                  rowClass(`${prefix}${option.id}`, option.id),
                )}
              >
                {renderSelect(`${prefix}${option.id}`, option.id, undefined)}
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

  const effortFixed = efforts.length < 2
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
            activeKey ? `model-option-${activeKey}` : undefined
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
        className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto overscroll-contain p-1.5"
      >
        {loading ? (
          <div className="px-2 py-3 text-ink-faint">Loading model catalog…</div>
        ) : showCatalog ? (
          catalogEmpty ? (
            <div className="px-2 py-3 text-ink-faint">
              {searching
                ? `No model matches "${filter.trim()}".`
                : onAddProvider
                  ? 'No providers yet. Add one to get models.'
                  : 'No models are configured.'}
            </div>
          ) : (
            renderGroups(groups, '')
          )
        ) : (
          <>
            {favoritesEmpty ? (
              <div className="px-2 py-3 text-ink-faint">
                {options.length === 0
                  ? 'No models yet. Add a provider to get some.'
                  : onEdit
                    ? 'No favorites yet. Pin up to five in Manage models.'
                    : 'No favorites yet.'}
              </div>
            ) : (
              <>
                {favoriteRows
                  .filter((row) => !row.pinned)
                  .map((row) => (
                    <div
                      key={row.id}
                      className={cn(
                        'flex h-10 shrink-0 items-center rounded-lg pr-2 transition-colors duration-100',
                        rowClass(`fav:${row.id}`, row.id),
                      )}
                    >
                      <span className="size-8 shrink-0" aria-hidden />
                      {renderSelect(`fav:${row.id}`, row.id, row.effort)}
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
                      const label = option
                        ? formatModelLabel(option.label)
                        : row.id
                      const at = pinned.findIndex(
                        (entry) => entry.id === row.id,
                      )
                      return (
                        <div
                          key={row.id}
                          className={cn(
                            'h-10 shrink-0 rounded-lg pr-2 transition-colors duration-100',
                            rowClass(`fav:${row.id}`, row.id),
                          )}
                        >
                          {disabled ? (
                            <div className="flex h-10 items-center">
                              <span className="size-8 shrink-0" aria-hidden />
                              {renderSelect(
                                `fav:${row.id}`,
                                row.id,
                                row.effort,
                              )}
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
                                if (
                                  to === at ||
                                  to < 0 ||
                                  to >= pinned.length
                                ) {
                                  return
                                }
                                onPinnedChange(
                                  movePinnedModel(pinned, row.id, to),
                                )
                              }}
                            >
                              {renderSelect(
                                `fav:${row.id}`,
                                row.id,
                                row.effort,
                              )}
                            </PinnedRow>
                          )}
                        </div>
                      )
                    })}
                </Reorder.Group>
              </>
            )}

            {/* All models: the whole catalog, grouped by provider, folded
                under the favorites so the menu stays short; a model is a
                click away without leaving the menu. The wrapper is
                `display: contents` so the header is a direct child of the
                scrolling list and can stick to either edge: at the bottom
                while folded (visible even when the favorites overflow a
                short menu), at the top while the catalog scrolls under it. */}
            {showAllModels ? (
              <div
                data-all-models={allModelsOpen ? 'open' : 'closed'}
                className="contents"
              >
                <button
                  ref={allModelsRef}
                  type="button"
                  aria-expanded={allModelsOpen}
                  aria-controls="model-picker-all-models"
                  onClick={toggleAllModels}
                  className={cn(
                    'sticky -top-1.5 -bottom-1.5 z-10 flex h-9 w-full shrink-0 items-center gap-2 bg-popover px-2 text-xs font-medium text-ink-faint hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-rule-focus',
                    !favoritesEmpty && 'mt-1 border-t border-edge',
                  )}
                >
                  <ChevronRight
                    aria-hidden
                    className={cn(
                      'size-4 shrink-0 transition-transform duration-[var(--motion-duration-control)]',
                      allModelsOpen && 'rotate-90',
                    )}
                  />
                  <span className="min-w-0 flex-1 truncate text-left">
                    All models
                  </span>
                  <span className="shrink-0 tabular-nums">
                    {options.length}
                  </span>
                </button>
                {allModelsOpen ? (
                  <section
                    id="model-picker-all-models"
                    aria-label="All models"
                    className="flex flex-col gap-0.5"
                  >
                    {renderGroups(allGroups, 'all:')}
                  </section>
                ) : null}
              </div>
            ) : null}
          </>
        )}
      </div>

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
            title="Pin up to five favorites and arrange them"
            disabled={disabled}
            onClick={onEdit}
            className="flex h-8 items-center gap-1.5 rounded-lg px-2 text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40"
          >
            <LayoutGrid className="size-4" aria-hidden />
            Manage models
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
