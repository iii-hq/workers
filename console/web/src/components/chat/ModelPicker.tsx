import * as DropdownMenuPrimitive from '@radix-ui/react-dropdown-menu'
import {
  Check,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  RefreshCw,
  Settings,
} from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { cn } from '@/lib/utils'
import {
  CATALOG_MODEL_KEY_SEP,
  type ModelId,
  type ModelOption,
  type ReasoningEffortOption,
  THINKING_LEVELS,
  type ThinkingLevel,
} from '@/types/chat'

// Deep link to the llm-router configuration entry in the Workers modal editor,
// where api keys + per-provider settings are now edited (the bespoke
// per-provider dialog was retired in favour of the schema-driven form).
const HARNESS_CONFIG_HASH = '#/workers/configuration/llm-router'

const DEFAULT_EFFORT: ReasoningEffortOption = {
  effort: 'default',
  description: 'use the model default',
}

interface ModelPickerProps {
  value: ModelId | null
  options: ModelOption[]
  thinkingLevel: ThinkingLevel
  onChange: (next: ModelId) => void
  onThinkingLevelChange: (next: ThinkingLevel) => void
  disabled?: boolean
  loading?: boolean
  className?: string
}

interface ModelGroup {
  label: string
  options: ModelOption[]
}

function groupByProvider(options: ModelOption[]): ModelGroup[] {
  const byProvider = new Map<string, ModelOption[]>()
  for (const opt of options) {
    const provider = opt.id.split(CATALOG_MODEL_KEY_SEP)[0] || '—'
    const bucket = byProvider.get(provider) ?? []
    bucket.push(opt)
    byProvider.set(provider, bucket)
  }
  return [...byProvider.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([label, opts]) => ({ label, options: opts }))
}

function providerForModel(modelId: ModelId | undefined): string | null {
  return modelId?.split(CATALOG_MODEL_KEY_SEP)[0] || null
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
  thinkingLevel,
  onChange,
  onThinkingLevelChange,
  disabled,
  loading,
  className,
}: ModelPickerProps) {
  const ctx = useConversationsCtxOptional()
  const [open, setOpen] = useState(false)
  const [expandedProvider, setExpandedProvider] = useState<string | null>(null)
  const [effortFocusRequest, setEffortFocusRequest] = useState(0)
  const effortByModel = useRef(new Map<ModelId, ThinkingLevel>())
  const modelItemRefs = useRef(new Map<ModelId, HTMLElement>())
  const effortItemRefs = useRef(new Map<ThinkingLevel, HTMLElement>())
  const pendingEffortFocus = useRef<{
    model: ModelId
    effort: ThinkingLevel
  } | null>(null)

  const presentIds = ctx?.presentProviders.map((p) => p.id) ?? []
  const presentSet = new Set<string>(presentIds)
  // Providers the router declares but whose worker is not loaded: their
  // catalog models would only fail with `provider_unavailable` at dispatch.
  const unavailableSet = new Set(
    (ctx?.presentProviders ?? [])
      .filter((p) => p.available === false)
      .map((p) => p.id),
  )

  const optionsById = useMemo(
    () => new Map(options.map((option) => [option.id, option])),
    [options],
  )

  const pickerOptions = options
  const safeValue =
    value != null && pickerOptions.some((option) => option.id === value)
      ? value
      : undefined
  const selected = safeValue ? optionsById.get(safeValue) : undefined
  const selectedEfforts = useMemo(() => effortOptionsFor(selected), [selected])

  const modelGroups = groupByProvider(pickerOptions)
  const grouped = new Set(modelGroups.map((group) => group.label))
  const emptyGroups: ModelGroup[] = presentIds
    .filter((id) => !grouped.has(id))
    .map((id) => ({ label: id, options: [] }))
  const groups = [...modelGroups, ...emptyGroups].sort((a, b) =>
    a.label.localeCompare(b.label),
  )

  const pickerDisabled = disabled || loading || groups.length === 0

  useEffect(() => {
    if (pickerDisabled && open) setOpen(false)
  }, [pickerDisabled, open])

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
    if (
      !open ||
      !safeValue ||
      expandedProvider !== providerForModel(safeValue)
    ) {
      return
    }
    const frame = window.requestAnimationFrame(() => {
      modelItemRefs.current.get(safeValue)?.focus()
    })
    return () => window.cancelAnimationFrame(frame)
  }, [expandedProvider, open, safeValue])

  useEffect(() => {
    if (effortFocusRequest === 0) return
    const pending = pendingEffortFocus.current
    if (!open || !pending || pending.model !== safeValue) return
    const target =
      effortItemRefs.current.get(pending.effort) ??
      effortItemRefs.current.get('default')
    if (!target) return
    pendingEffortFocus.current = null
    target.focus()
  }, [effortFocusRequest, open, safeValue])

  function handleModelChange(next: string) {
    const nextModel = optionsById.get(next)
    const nextEfforts = effortOptionsFor(nextModel)
    const remembered = effortByModel.current.get(next) ?? 'default'
    const nextEffort = effortSupported(nextEfforts, remembered)
      ? remembered
      : 'default'
    onChange(next)
    if (nextEffort !== thinkingLevel) onThinkingLevelChange(nextEffort)
    return { nextEfforts, nextEffort }
  }

  function handleEffortChange(next: ThinkingLevel) {
    if (safeValue) effortByModel.current.set(safeValue, next)
    onThinkingLevelChange(next)
  }

  function handleOpenChange(nextOpen: boolean) {
    if (nextOpen) {
      setExpandedProvider(
        providerForModel(safeValue) ?? groups[0]?.label ?? null,
      )
    }
    setOpen(nextOpen)
  }

  return (
    <span className={cn('flex min-w-0 items-center gap-1', className)}>
      <DropdownMenuPrimitive.Root open={open} onOpenChange={handleOpenChange}>
        <DropdownMenuPrimitive.Trigger asChild disabled={pickerDisabled}>
          <button
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
              'flex h-9 min-w-0 flex-1 items-center justify-between gap-x-2 border border-rule bg-bg px-3 font-mono text-[13px] lowercase text-ink transition-colors focus:border-ink focus:outline-none data-[state=open]:border-ink',
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
                {selected?.label ?? (loading ? 'loading…' : 'no models')}
              </span>
              {selectedEfforts.length > 1 && thinkingLevel !== 'default' ? (
                <span className="shrink-0 text-[11px] text-ink-faint">
                  · {thinkingLevel}
                </span>
              ) : null}
            </span>
            {open ? (
              <ChevronUp size={12} aria-hidden />
            ) : (
              <ChevronDown size={12} aria-hidden />
            )}
          </button>
        </DropdownMenuPrimitive.Trigger>

        <DropdownMenuPrimitive.Portal>
          <DropdownMenuPrimitive.Content
            sideOffset={4}
            align="start"
            className="z-50 w-[min(480px,calc(100vw-24px))] overflow-hidden border border-rule bg-bg font-mono text-[13px] lowercase text-ink"
          >
            <DropdownMenuPrimitive.RadioGroup
              value={safeValue ?? ''}
              className="max-h-[50vh] overflow-y-auto p-1"
            >
              {groups.map((group) => {
                const unavailable = unavailableSet.has(group.label)
                const unconfigured = !unavailable && group.options.length === 0
                const expanded = expandedProvider === group.label
                return (
                  <DropdownMenuPrimitive.Group key={group.label}>
                    <div className="flex items-center gap-1 pr-2">
                      <DropdownMenuPrimitive.Item
                        aria-expanded={expanded}
                        onSelect={(event) => {
                          event.preventDefault()
                          setExpandedProvider(expanded ? null : group.label)
                        }}
                        className="flex min-w-0 flex-1 cursor-pointer select-none items-center gap-1.5 px-2 py-2 text-ink-faint outline-none data-[highlighted]:bg-panel data-[highlighted]:text-ink"
                      >
                        {expanded ? (
                          <ChevronDown
                            size={12}
                            className="shrink-0"
                            aria-hidden
                          />
                        ) : (
                          <ChevronRight
                            size={12}
                            className="shrink-0"
                            aria-hidden
                          />
                        )}
                        <span className="truncate text-[11px] uppercase tracking-[0.12em]">
                          {group.label}
                        </span>
                        {unavailable ? (
                          <span className="text-[10px] lowercase tracking-normal text-ink-ghost">
                            not loaded
                          </span>
                        ) : unconfigured ? (
                          <span className="text-[10px] lowercase tracking-normal text-ink-ghost">
                            not configured
                          </span>
                        ) : null}
                      </DropdownMenuPrimitive.Item>
                      {presentSet.has(group.label) ? (
                        <button
                          type="button"
                          aria-label={`configure ${group.label}`}
                          title={`configure ${group.label} in harness configuration`}
                          onPointerDown={(event) => event.stopPropagation()}
                          onClick={(event) => {
                            event.preventDefault()
                            event.stopPropagation()
                            window.location.hash = HARNESS_CONFIG_HASH
                          }}
                          className="-mr-0.5 p-0.5 text-ink-faint transition-colors hover:text-ink"
                        >
                          <Settings size={12} />
                        </button>
                      ) : null}
                    </div>
                    {expanded
                      ? group.options.map((option) => (
                          <DropdownMenuPrimitive.RadioItem
                            key={option.id}
                            ref={(node) => {
                              if (node) {
                                modelItemRefs.current.set(option.id, node)
                              } else {
                                modelItemRefs.current.delete(option.id)
                              }
                            }}
                            value={option.id}
                            disabled={unavailable}
                            onSelect={(event) => {
                              event.preventDefault()
                              const { nextEfforts, nextEffort } =
                                handleModelChange(option.id)
                              if (nextEfforts.length > 1) {
                                pendingEffortFocus.current = {
                                  model: option.id,
                                  effort: nextEffort,
                                }
                                setEffortFocusRequest((request) => request + 1)
                              }
                            }}
                            className={cn(
                              'relative flex min-w-0 cursor-pointer select-none items-center py-1.5 pl-7 pr-3 outline-none',
                              'data-[highlighted]:bg-panel data-[highlighted]:text-ink',
                              'data-[state=checked]:text-ink',
                              'data-[disabled]:cursor-default data-[disabled]:opacity-40',
                            )}
                          >
                            <DropdownMenuPrimitive.ItemIndicator className="absolute left-2 top-1/2 -translate-y-1/2 text-ink">
                              <Check size={12} aria-hidden />
                            </DropdownMenuPrimitive.ItemIndicator>
                            <span className="truncate">{option.label}</span>
                          </DropdownMenuPrimitive.RadioItem>
                        ))
                      : null}
                  </DropdownMenuPrimitive.Group>
                )
              })}
            </DropdownMenuPrimitive.RadioGroup>

            <div className="flex min-h-10 items-center gap-1 border-t border-rule bg-panel px-3 py-2">
              <span className="shrink-0 text-[10px] uppercase tracking-[0.12em] text-ink-faint">
                effort
              </span>
              {selectedEfforts.length > 1 ? (
                <DropdownMenuPrimitive.RadioGroup
                  value={thinkingLevel}
                  className="flex min-w-0 items-center whitespace-nowrap"
                >
                  {selectedEfforts.map((option) => (
                    <DropdownMenuPrimitive.RadioItem
                      key={option.effort}
                      value={option.effort}
                      ref={(node) => {
                        if (node) {
                          effortItemRefs.current.set(option.effort, node)
                        } else {
                          effortItemRefs.current.delete(option.effort)
                        }
                      }}
                      aria-label={option.effort}
                      title={option.description}
                      onSelect={() => {
                        handleEffortChange(option.effort)
                        setOpen(false)
                      }}
                      className={cn(
                        'cursor-pointer select-none border px-1.5 py-1 text-[11px] outline-none transition-colors data-[highlighted]:bg-paper-2 data-[highlighted]:text-ink',
                        'border-transparent text-ink-faint data-[state=checked]:border-ink data-[state=checked]:bg-ink data-[state=checked]:text-bg',
                      )}
                    >
                      {option.effort}
                    </DropdownMenuPrimitive.RadioItem>
                  ))}
                </DropdownMenuPrimitive.RadioGroup>
              ) : (
                <span
                  role="status"
                  aria-live="polite"
                  className="truncate text-[11px] text-ink-faint"
                >
                  {selected
                    ? 'this model uses its default effort'
                    : 'select a model to see effort options'}
                </span>
              )}
            </div>
          </DropdownMenuPrimitive.Content>
        </DropdownMenuPrimitive.Portal>
      </DropdownMenuPrimitive.Root>

      {ctx ? (
        <button
          type="button"
          aria-label="refresh model list"
          title="refresh model list from providers"
          disabled={ctx.refreshingModels || disabled}
          onClick={() => {
            void ctx.refreshModels()
          }}
          className="inline-flex size-9 shrink-0 items-center justify-center text-ink-ghost transition-colors hover:text-ink focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50"
        >
          <RefreshCw
            size={12}
            className={cn(ctx.refreshingModels && 'animate-spin')}
            aria-hidden
          />
        </button>
      ) : null}
    </span>
  )
}
