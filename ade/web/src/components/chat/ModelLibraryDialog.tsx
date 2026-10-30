import { ArrowLeft, Check, GripVertical, Plus, X } from 'lucide-react'
import { motion, useReducedMotion } from 'motion/react'
import {
  type CSSProperties,
  type DragEvent,
  type UIEvent,
  useEffect,
  useState,
} from 'react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/shadcn/dialog'
import { eyebrowClassName } from '@/components/ui/Eyebrow'
import {
  discoveryBlocksModels,
  type ProviderListEntry,
} from '@/lib/models-catalog'
import {
  isPinned,
  PINNED_MODELS_MAX,
  type PinnedModel,
  pinModel,
  unpinModel,
} from '@/lib/pinned-models'
import { cn } from '@/lib/utils'
import { useUnsavedGuard } from '@/pages/Configuration/tabs/WorkersTab/useUnsavedGuard'
import type { ModelId, ModelOption, ThinkingLevel } from '@/types/chat'
import { AddProviderPanel } from './AddProviderPanel'
import { ModelMark } from './ModelMark'
import {
  formatEffortLabel,
  formatModelLabel,
  formatProviderLabel,
  providerForModel,
} from './model-picker-presentation'
import { ProviderConfigurationPanel } from './ProviderConfigurationPanel'
import { ProviderIcon } from './ProviderIcon'

const DRAG_MIME = 'application/x-iii-model'

type LibraryView =
  | { kind: 'library' }
  | { kind: 'provider'; id: string }
  | { kind: 'add-provider' }

interface ModelLibraryDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  value: ModelId | null
  options: ModelOption[]
  providers: ProviderListEntry[]
  pinned: PinnedModel[]
  thinkingLevel: ThinkingLevel
  onPinnedChange: (next: PinnedModel[]) => void
  disabled?: boolean
}

interface ProviderColumn {
  id: string
  label: string
  provider: ProviderListEntry | undefined
  options: ModelOption[]
  /** The catalog can be picked from; otherwise the column offers setup. */
  usable: boolean
  unavailable: boolean
}

function providerColumns(
  options: ModelOption[],
  providers: ProviderListEntry[],
): ProviderColumn[] {
  const byProvider = new Map<string, ModelOption[]>()
  for (const option of options) {
    const id = providerForModel(option.id) ?? '—'
    byProvider.set(id, [...(byProvider.get(id) ?? []), option])
  }
  const providerById = new Map(providers.map((p) => [p.id, p]))
  const ids = new Set([...byProvider.keys(), ...providers.map((p) => p.id)])
  return [...ids]
    .sort((a, b) => a.localeCompare(b))
    .map((id) => {
      const provider = providerById.get(id)
      const models = byProvider.get(id) ?? []
      const configured = provider?.configured ?? models.length > 0
      const ownsAuthentication =
        provider !== undefined && provider.credential_env_var === undefined
      return {
        id,
        label: provider?.display_name ?? formatProviderLabel(id) ?? id,
        provider,
        options: models,
        usable: models.length > 0 && (configured || ownsAuthentication),
        unavailable:
          provider?.available === false || discoveryBlocksModels(provider),
      }
    })
}

/**
 * The library behind the quick menu's All models: the five pinned slots
 * across the top (the first is the default a new chat starts on; they stay
 * stuck to the top while the catalog scrolls, so every drag ends on screen),
 * every provider's catalog in columns below. Drag a model onto a slot to pin it there, drag
 * a slot onto another to reorder, drag a slot down into the library (or
 * click its X) to unpin; a click on a library row pins or unpins it too.
 * A provider that still needs setup offers its configuration in place.
 */
export function ModelLibraryDialog({
  open,
  onOpenChange,
  value,
  options,
  providers,
  pinned,
  thinkingLevel,
  onPinnedChange,
  disabled,
}: ModelLibraryDialogProps) {
  const [view, setView] = useState<LibraryView>({ kind: 'library' })
  const [dragId, setDragId] = useState<ModelId | null>(null)
  const [overSlot, setOverSlot] = useState<number | null>(null)
  // The catalog has scrolled under the sticky slots: a hairline then marks
  // the slots' edge so the rows visibly slide beneath it.
  const [scrolled, setScrolled] = useState(false)
  const onCatalogScroll = (event: UIEvent<HTMLDivElement>) => {
    const next = event.currentTarget.scrollTop > 0
    setScrolled((prev) => (prev === next ? prev : next))
  }
  const reduceMotion = useReducedMotion() ?? false
  const guard = useUnsavedGuard()

  // A drag ends somewhere this dialog may not hear about: outside the
  // window, on a slot whose source card just unmounted (it was unpinned by
  // the drop), with Escape. The window hears all of them, so the
  // highlight and the faded source never outlive the drag.
  useEffect(() => {
    if (!open) return
    const reset = () => {
      setDragId(null)
      setOverSlot(null)
    }
    window.addEventListener('dragend', reset)
    window.addEventListener('drop', reset)
    return () => {
      window.removeEventListener('dragend', reset)
      window.removeEventListener('drop', reset)
    }
  }, [open])
  const optionsById = new Map(options.map((option) => [option.id, option]))
  const providerById = new Map(providers.map((p) => [p.id, p]))
  const columns = providerColumns(options, providers)

  const effortFor = (id: ModelId): ThinkingLevel =>
    pinned.find((entry) => entry.id === id)?.effort ??
    (id === value ? thinkingLevel : 'default')

  const handleOpenChange = (next: boolean) => {
    if (next) {
      onOpenChange(true)
      return
    }
    guard.tryNavigate(() => {
      setView({ kind: 'library' })
      setDragId(null)
      onOpenChange(false)
    })
  }

  const onDragStart = (event: DragEvent, id: ModelId) => {
    event.dataTransfer.setData(DRAG_MIME, id)
    event.dataTransfer.setData('text/plain', id)
    event.dataTransfer.effectAllowed = 'move'
    setDragId(id)
  }
  const onDragEnd = () => {
    setDragId(null)
    setOverSlot(null)
  }
  const draggedId = (event: DragEvent): ModelId | null =>
    event.dataTransfer.getData(DRAG_MIME) || dragId
  const dropOnSlot = (event: DragEvent, index: number) => {
    event.preventDefault()
    const id = draggedId(event)
    setOverSlot(null)
    setDragId(null)
    if (!id || !optionsById.has(id)) return
    onPinnedChange(pinModel(pinned, { id, effort: effortFor(id) }, index))
  }
  const dropOnLibrary = (event: DragEvent) => {
    const id = draggedId(event)
    setDragId(null)
    setOverSlot(null)
    if (!id || !isPinned(pinned, id)) return
    event.preventDefault()
    onPinnedChange(unpinModel(pinned, id))
  }
  const togglePin = (id: ModelId) => {
    if (disabled) return
    onPinnedChange(
      isPinned(pinned, id)
        ? unpinModel(pinned, id)
        : pinModel(pinned, { id, effort: effortFor(id) }),
    )
  }

  const slots: Array<PinnedModel | null> = Array.from(
    { length: PINNED_MODELS_MAX },
    (_, index) => pinned[index] ?? null,
  )
  const full = pinned.length >= PINNED_MODELS_MAX

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent
        aria-label="model library"
        style={
          {
            // The library spreads out; a provider's settings read best as
            // one comfortable column.
            '--dialog-max-width': view.kind === 'library' ? '76rem' : '44rem',
          } as CSSProperties
        }
        className="@container flex max-h-[min(88dvh,900px)] flex-col gap-0 overflow-hidden p-0"
      >
        {view.kind === 'library' ? (
          <>
            <DialogHeader className="shrink-0 px-6 pt-5 pb-2">
              <DialogTitle>Models</DialogTitle>
              <DialogDescription className="">
                Pin up to five models for the quick menu. Drag to arrange them;
                the first is the default for a new chat.
              </DialogDescription>
            </DialogHeader>
            <div
              className="min-h-0 flex-1 overflow-y-auto px-6 pb-6"
              onScroll={onCatalogScroll}
            >
              {/* The slots stay put while the catalog scrolls under them:
                  a model found far down a long column is still one drag
                  up to a slot that is on screen. The sticky block wears
                  the dialog's own surface, full-bleed, so the rows pass
                  behind it; a hairline appears at its foot once anything
                  has scrolled under. */}
              <div
                data-pinned-slots
                data-scrolled={scrolled || undefined}
                className={cn(
                  'sticky top-0 z-10 -mx-6 bg-popover px-6 pt-4 pb-4',
                  scrolled && 'shadow-[0_1px_0_var(--color-edge)]',
                )}
              >
                <ul
                  aria-label="pinned models"
                  className="grid grid-cols-2 gap-3 @md:grid-cols-3 @3xl:grid-cols-5"
                >
                  {slots.map((slot, index) => {
                    const option = slot ? optionsById.get(slot.id) : undefined
                    const providerId = slot ? providerForModel(slot.id) : null
                    const provider = providerId
                      ? providerById.get(providerId)
                      : undefined
                    const providerLabel =
                      provider?.display_name ??
                      formatProviderLabel(providerId) ??
                      ''
                    const over = overSlot === index
                    return (
                      <li
                        key={slot?.id ?? `empty-${index}`}
                        data-pinned-slot={index}
                        draggable={Boolean(slot) && !disabled}
                        onDragStart={
                          slot
                            ? (event) => onDragStart(event, slot.id)
                            : undefined
                        }
                        onDragEnd={onDragEnd}
                        onDragOver={(event) => {
                          event.preventDefault()
                          event.dataTransfer.dropEffect = 'move'
                          if (overSlot !== index) setOverSlot(index)
                        }}
                        onDragLeave={(event) => {
                          // Leaving for a child of the slot is not leaving.
                          const next = event.relatedTarget as Node | null
                          if (next && event.currentTarget.contains(next)) return
                          if (overSlot === index) setOverSlot(null)
                        }}
                        onDrop={(event) => dropOnSlot(event, index)}
                        className="relative h-28"
                      >
                        {/* The card is what moves: a pin that changes slot
                          slides to its new place. */}
                        <motion.div
                          layoutId={slot ? `pinned-${slot.id}` : undefined}
                          layout={
                            slot && !reduceMotion ? 'position' : undefined
                          }
                          initial={false}
                          transition={{
                            layout: reduceMotion
                              ? { duration: 0 }
                              : { duration: 0.2, ease: [0.2, 0, 0, 1] },
                          }}
                          className={cn(
                            'group absolute inset-0 flex flex-col items-center justify-center gap-1.5 rounded-xl p-3 text-center transition-colors',
                            slot
                              ? 'cursor-grab bg-panel-raised border border-edge  active:cursor-grabbing'
                              : 'border border-dashed border-edge',
                            over && 'bg-surface-hover ring-rule-focus',
                            slot && dragId === slot.id && 'opacity-50',
                          )}
                        >
                          {index === 0 ? (
                            <span
                              className={cn(
                                eyebrowClassName,
                                'absolute top-2 left-2 rounded-sm bg-surface px-1.5 py-0.5 text-ink-faint',
                              )}
                            >
                              Default
                            </span>
                          ) : null}
                          {slot && option ? (
                            <>
                              <ModelMark
                                modelId={option.id}
                                modelLabel={option.label}
                                providerIconSvg={provider?.icon_svg}
                                providerLabel={providerLabel}
                                providerId={providerId}
                                className="size-7 text-ink"
                              />
                              <span className="max-w-full truncate text-sm text-ink">
                                {formatModelLabel(option.label)}
                              </span>
                              <span
                                className={cn(
                                  eyebrowClassName,
                                  'text-ink-faint',
                                )}
                              >
                                {slot.effort === 'default'
                                  ? 'Model default'
                                  : formatEffortLabel(slot.effort)}
                              </span>
                              <span className="sr-only">{providerLabel}</span>
                              {!disabled ? (
                                <button
                                  type="button"
                                  aria-label={`Unpin ${formatModelLabel(option.label)}`}
                                  onClick={() =>
                                    onPinnedChange(unpinModel(pinned, slot.id))
                                  }
                                  className="absolute top-1.5 right-1.5 flex size-7 items-center justify-center rounded-md text-ink-faint opacity-0 transition-opacity hover:bg-surface-hover hover:text-ink focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus group-hover:opacity-100"
                                >
                                  <X className="size-4" aria-hidden />
                                </button>
                              ) : null}
                            </>
                          ) : (
                            <span className="text-sm text-ink-faint">
                              Drop a model here
                            </span>
                          )}
                        </motion.div>
                      </li>
                    )
                  })}
                </ul>
              </div>

              <section
                aria-label="model catalog"
                onDragOver={(event) => {
                  if (dragId && isPinned(pinned, dragId)) event.preventDefault()
                }}
                onDrop={dropOnLibrary}
                className="mt-4 grid gap-x-6 gap-y-8 [grid-template-columns:repeat(auto-fill,minmax(14rem,1fr))]"
              >
                {columns.map((column) => (
                  <section
                    key={column.id}
                    aria-label={column.label}
                    data-provider-group={column.id}
                    className="min-w-0"
                  >
                    <h3 className="mb-3 flex h-6 items-center gap-2 text-sm text-ink-faint">
                      <ProviderIcon
                        iconSvg={column.provider?.icon_svg}
                        label={column.label}
                        providerId={column.id}
                        className="text-ink-faint"
                      />
                      <span className="min-w-0 truncate">{column.label}</span>
                      {column.usable ? (
                        <button
                          type="button"
                          onClick={() =>
                            setView({ kind: 'provider', id: column.id })
                          }
                          className="ml-auto shrink-0 rounded-sm text-xs font-medium text-ink-faint hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
                        >
                          Configure
                        </button>
                      ) : null}
                    </h3>
                    {column.usable ? (
                      <ul className="flex flex-col gap-2">
                        {column.options.map((option) => {
                          const pinnedHere = isPinned(pinned, option.id)
                          const label = formatModelLabel(option.label)
                          const canPin = !disabled && !column.unavailable
                          return (
                            <li
                              key={option.id}
                              draggable={canPin}
                              onDragStart={(event) =>
                                onDragStart(event, option.id)
                              }
                              onDragEnd={onDragEnd}
                              className={cn(
                                'flex h-11 items-center gap-2 rounded-lg bg-panel-raised pr-1.5 pl-3 ring-1 ring-edge',
                                canPin && 'cursor-grab active:cursor-grabbing',
                                dragId === option.id && 'opacity-50',
                                column.unavailable && 'opacity-50',
                              )}
                            >
                              <button
                                type="button"
                                data-model-option={option.id}
                                disabled={!canPin}
                                aria-pressed={pinnedHere}
                                title={
                                  pinnedHere
                                    ? `Unpin ${label}`
                                    : full
                                      ? `Pin ${label} in place of the last pin`
                                      : `Pin ${label}`
                                }
                                onClick={() => togglePin(option.id)}
                                className="flex h-full min-w-0 flex-1 items-center gap-2 text-left text-sm text-ink focus-visible:outline-none disabled:cursor-default"
                              >
                                <span className="truncate">{label}</span>
                                {pinnedHere ? (
                                  <Check
                                    className="size-4 shrink-0 text-ink"
                                    aria-hidden
                                  />
                                ) : null}
                              </button>
                              <GripVertical
                                className="size-4 shrink-0 text-ink-ghost"
                                aria-hidden
                              />
                            </li>
                          )
                        })}
                      </ul>
                    ) : (
                      <button
                        type="button"
                        disabled={disabled}
                        onClick={() =>
                          setView({ kind: 'provider', id: column.id })
                        }
                        className="flex h-11 w-full items-center justify-center rounded-lg px-3 text-sm text-ink-faint border border-dashed border-edge hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40"
                      >
                        {column.unavailable
                          ? `${column.label} not loaded`
                          : `Configure ${column.label}`}
                      </button>
                    )}
                  </section>
                ))}
                <section aria-label="add a provider" className="min-w-0">
                  <h3 className="mb-3 flex h-6 items-center text-sm text-ink-faint">
                    More providers
                  </h3>
                  <button
                    type="button"
                    aria-label="Add a provider"
                    disabled={disabled}
                    onClick={() => setView({ kind: 'add-provider' })}
                    className="flex h-11 w-full items-center justify-center gap-2 rounded-lg px-3 text-sm text-ink-faint border border-dashed border-edge hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40"
                  >
                    <Plus className="size-4" aria-hidden />
                    Add a provider
                  </button>
                </section>
              </section>
            </div>
          </>
        ) : (
          <div className="flex min-h-0 flex-1 flex-col font-inter">
            <div className="flex shrink-0 items-center gap-3 px-5 pt-4 pb-3 pr-14">
              <button
                type="button"
                aria-label="back to models"
                onClick={() =>
                  guard.tryNavigate(() => setView({ kind: 'library' }))
                }
                className="flex size-9 shrink-0 items-center justify-center rounded-lg text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
              >
                <ArrowLeft className="size-4" aria-hidden />
              </button>
              {view.kind === 'provider' ? (
                <ProviderIcon
                  iconSvg={providerById.get(view.id)?.icon_svg}
                  label={
                    providerById.get(view.id)?.display_name ??
                    formatProviderLabel(view.id) ??
                    view.id
                  }
                  providerId={view.id}
                  className="size-6 text-ink"
                />
              ) : null}
              <div className="min-w-0 flex-1">
                <DialogTitle className="text-[17px] font-semibold">
                  {view.kind === 'provider'
                    ? (providerById.get(view.id)?.display_name ??
                      formatProviderLabel(view.id) ??
                      view.id)
                    : 'Add a provider'}
                </DialogTitle>
                <DialogDescription className="text-[13px] text-ink-faint">
                  {view.kind === 'provider'
                    ? 'Credentials and provider-specific settings.'
                    : 'Provider workers from the workers registry.'}
                </DialogDescription>
              </div>
            </div>
            <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
              {view.kind === 'provider' ? (
                <ProviderConfigurationPanel
                  providerId={view.id}
                  onDirtyChange={guard.setDirty}
                />
              ) : (
                <AddProviderPanel
                  disabled={disabled}
                  onConfigureProvider={(id) =>
                    setView({ kind: 'provider', id })
                  }
                />
              )}
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
