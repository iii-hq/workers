import * as SelectPrimitive from '@radix-ui/react-select'
import {
  Check,
  ChevronDown,
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

function EffortPicker({
  value,
  options,
  onChange,
  disabled,
}: {
  value: ThinkingLevel
  options: ReasoningEffortOption[]
  onChange: (next: ThinkingLevel) => void
  disabled?: boolean
}) {
  const [open, setOpen] = useState(false)

  return (
    <SelectPrimitive.Root
      value={value}
      open={open}
      onOpenChange={setOpen}
      onValueChange={onChange}
      disabled={disabled}
    >
      <SelectPrimitive.Trigger
        aria-label={`reasoning effort: ${value}`}
        className={cn(
          'inline-flex h-9 min-w-0 items-center justify-between gap-x-2 border border-rule bg-bg px-3 font-mono text-[13px] lowercase text-ink transition-colors focus:border-ink focus:outline-none data-[state=open]:border-ink',
          disabled && 'pointer-events-none opacity-40',
        )}
      >
        <span className="truncate text-left">effort: {value}</span>
        <SelectPrimitive.Icon asChild>
          {open ? (
            <ChevronUp size={12} aria-hidden />
          ) : (
            <ChevronDown size={12} aria-hidden />
          )}
        </SelectPrimitive.Icon>
      </SelectPrimitive.Trigger>

      <SelectPrimitive.Portal>
        <SelectPrimitive.Content
          position="popper"
          sideOffset={4}
          align="end"
          className="z-50 min-w-[min(360px,calc(100vw-24px))] overflow-hidden border border-rule bg-bg font-mono text-[13px] lowercase text-ink"
        >
          <SelectPrimitive.Viewport className="max-h-[60vh] p-1">
            {options.map((option) => (
              <SelectPrimitive.Item
                key={option.effort}
                value={option.effort}
                className="relative cursor-pointer select-none py-2 pl-7 pr-3 outline-none data-[highlighted]:bg-panel data-[highlighted]:text-ink"
              >
                <SelectPrimitive.ItemIndicator className="absolute left-2 top-2.5 text-ink">
                  <Check size={12} aria-hidden />
                </SelectPrimitive.ItemIndicator>
                <SelectPrimitive.ItemText>
                  <span className="block text-ink">{option.effort}</span>
                </SelectPrimitive.ItemText>
                {option.description ? (
                  <span className="mt-0.5 block max-w-[42ch] text-[11px] leading-[1.45] text-ink-faint">
                    {option.description}
                  </span>
                ) : null}
              </SelectPrimitive.Item>
            ))}
          </SelectPrimitive.Viewport>
        </SelectPrimitive.Content>
      </SelectPrimitive.Portal>
    </SelectPrimitive.Root>
  )
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
  const effortByModel = useRef(new Map<ModelId, ThinkingLevel>())

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
    if (!safeValue) return
    if (effortSupported(selectedEfforts, thinkingLevel)) {
      effortByModel.current.set(safeValue, thinkingLevel)
      return
    }
    if (thinkingLevel !== 'default') onThinkingLevelChange('default')
    effortByModel.current.set(safeValue, 'default')
  }, [safeValue, selectedEfforts, thinkingLevel, onThinkingLevelChange])

  function handleModelChange(next: string) {
    const nextModel = optionsById.get(next)
    const nextEfforts = effortOptionsFor(nextModel)
    const remembered = effortByModel.current.get(next) ?? 'default'
    const nextEffort = effortSupported(nextEfforts, remembered)
      ? remembered
      : 'default'
    onChange(next)
    if (nextEffort !== thinkingLevel) onThinkingLevelChange(nextEffort)
  }

  function handleEffortChange(next: ThinkingLevel) {
    if (safeValue) effortByModel.current.set(safeValue, next)
    onThinkingLevelChange(next)
  }

  return (
    <span className="inline-flex min-w-0 items-center gap-1">
      <SelectPrimitive.Root
        value={safeValue ?? ''}
        open={open}
        onOpenChange={setOpen}
        onValueChange={handleModelChange}
        disabled={pickerDisabled}
      >
        <SelectPrimitive.Trigger
          aria-label={loading ? 'model (loading catalog)' : 'model'}
          aria-busy={loading || undefined}
          className={cn(
            'inline-flex h-9 min-w-0 max-w-full items-center justify-between gap-x-2 border border-rule bg-bg px-3 font-mono text-[13px] lowercase text-ink transition-colors focus:border-ink focus:outline-none data-[placeholder]:text-ink-faint data-[state=open]:border-ink',
            pickerDisabled && 'pointer-events-none opacity-40',
            className,
          )}
        >
          <span className="min-w-0 flex-1 truncate text-left">
            <SelectPrimitive.Value
              placeholder={loading ? 'loading…' : 'no models'}
            >
              {selected?.label}
            </SelectPrimitive.Value>
          </span>
          <SelectPrimitive.Icon asChild>
            {open ? (
              <ChevronUp size={12} aria-hidden />
            ) : (
              <ChevronDown size={12} aria-hidden />
            )}
          </SelectPrimitive.Icon>
        </SelectPrimitive.Trigger>

        <SelectPrimitive.Portal>
          <SelectPrimitive.Content
            position="popper"
            sideOffset={4}
            className="z-50 min-w-[var(--radix-select-trigger-width)] overflow-hidden border border-rule bg-bg font-mono text-[13px] lowercase text-ink"
          >
            <SelectPrimitive.Viewport className="max-h-[60vh] p-1">
              {groups.map((group) => {
                const unavailable = unavailableSet.has(group.label)
                const unconfigured = !unavailable && group.options.length === 0
                return (
                  <SelectPrimitive.Group key={group.label}>
                    <div className="flex items-center justify-between gap-2 pb-1 pr-2 pt-2">
                      <span className="flex min-w-0 items-baseline gap-1.5">
                        <SelectPrimitive.Label className="px-3 text-[11px] uppercase tracking-[0.12em] text-ink-faint">
                          {group.label}
                        </SelectPrimitive.Label>
                        {unavailable ? (
                          <span className="text-[10px] lowercase tracking-normal text-ink-ghost">
                            not loaded
                          </span>
                        ) : unconfigured ? (
                          <span className="text-[10px] lowercase tracking-normal text-ink-ghost">
                            not configured
                          </span>
                        ) : null}
                      </span>
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
                    {group.options.map((option) => (
                      <SelectPrimitive.Item
                        key={option.id}
                        value={option.id}
                        disabled={unavailable}
                        className={cn(
                          'relative flex min-w-0 cursor-pointer select-none items-center py-1.5 pl-7 pr-3 outline-none',
                          'data-[highlighted]:bg-panel data-[highlighted]:text-ink',
                          'data-[state=checked]:text-ink',
                          'data-[disabled]:cursor-default data-[disabled]:opacity-40',
                        )}
                      >
                        <SelectPrimitive.ItemIndicator className="absolute left-2 top-1/2 -translate-y-1/2 text-ink">
                          <Check size={12} aria-hidden />
                        </SelectPrimitive.ItemIndicator>
                        <SelectPrimitive.ItemText className="truncate">
                          {option.label}
                        </SelectPrimitive.ItemText>
                      </SelectPrimitive.Item>
                    ))}
                  </SelectPrimitive.Group>
                )
              })}
            </SelectPrimitive.Viewport>
          </SelectPrimitive.Content>
        </SelectPrimitive.Portal>
      </SelectPrimitive.Root>

      {selectedEfforts.length > 1 ? (
        <EffortPicker
          value={thinkingLevel}
          options={selectedEfforts}
          onChange={handleEffortChange}
          disabled={disabled || loading}
        />
      ) : null}

      {ctx ? (
        <button
          type="button"
          aria-label="refresh model list"
          title="refresh model list from providers"
          disabled={ctx.refreshingModels || disabled}
          onClick={() => {
            void ctx.refreshModels()
          }}
          className="p-1 text-ink-ghost transition-colors hover:text-ink disabled:opacity-50"
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
