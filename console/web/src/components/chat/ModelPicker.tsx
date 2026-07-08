import * as SelectPrimitive from '@radix-ui/react-select'
import {
  Check,
  ChevronDown,
  ChevronUp,
  RefreshCw,
  Settings,
} from 'lucide-react'
import { useMemo, useState } from 'react'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { cn } from '@/lib/utils'
import {
  CATALOG_MODEL_KEY_SEP,
  type ModelId,
  type ModelOption,
  THINKING_LEVELS,
  type ThinkingLevel,
} from '@/types/chat'

// Deep link to the llm-router configuration entry in the workers/config editor,
// where api keys + per-provider settings are now edited (the bespoke
// per-provider dialog was retired in favour of the schema-driven form).
const HARNESS_CONFIG_HASH = '#/configuration/workers/llm-router'

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

function formatThinkingLevel(level: ThinkingLevel): string {
  return level === 'off' ? 'thinking off' : `thinking ${level}`
}

function triggerLabel(
  model: ModelOption | undefined,
  thinkingLevel: ThinkingLevel,
): string | undefined {
  if (!model) return undefined
  if (model.supportsThinking && thinkingLevel !== 'off') {
    return `${model.label} · ${formatThinkingLevel(thinkingLevel)}`
  }
  return model.label
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
  const [expandedModelId, setExpandedModelId] = useState<ModelId | null>(null)

  const presentIds = ctx?.presentProviders.map((p) => p.id) ?? []
  const presentSet = new Set<string>(presentIds)

  const optionsById = useMemo(
    () => new Map(options.map((o) => [o.id, o])),
    [options],
  )

  const pickerOptions = options
  const safeValue =
    value != null && pickerOptions.some((o) => o.id === value)
      ? value
      : undefined
  const selected = safeValue ? optionsById.get(safeValue) : undefined

  const modelGroups = groupByProvider(pickerOptions)
  const grouped = new Set(modelGroups.map((g) => g.label))
  const emptyGroups: ModelGroup[] = presentIds
    .filter((id) => !grouped.has(id))
    .map((id) => ({ label: id, options: [] }))
  const groups = [...modelGroups, ...emptyGroups].sort((a, b) =>
    a.label.localeCompare(b.label),
  )

  const pickerDisabled = disabled || loading || groups.length === 0

  function handleThinkingPick(modelId: ModelId, level: ThinkingLevel) {
    onThinkingLevelChange(level)
    if (safeValue !== modelId) onChange(modelId)
  }

  return (
    <span className="inline-flex items-center gap-1">
      <SelectPrimitive.Root
        value={safeValue ?? ''}
        open={open}
        onOpenChange={(next) => {
          setOpen(next)
          if (!next) setExpandedModelId(null)
        }}
        onValueChange={(next) => {
          onChange(next as ModelId)
          setExpandedModelId(null)
        }}
        disabled={pickerDisabled}
      >
        <SelectPrimitive.Trigger
          aria-label={loading ? 'model (loading catalog)' : 'model'}
          aria-busy={loading || undefined}
          className={cn(
            'inline-flex items-center justify-between gap-x-2 border border-rule bg-bg px-3 h-9 text-ink font-mono text-[13px] lowercase focus:outline-none focus:border-ink data-[state=open]:border-ink transition-colors max-w-full min-w-0 data-[placeholder]:text-ink-faint',
            pickerDisabled && 'opacity-40 pointer-events-none',
            className,
          )}
        >
          <span className="min-w-0 flex-1 truncate text-left">
            <SelectPrimitive.Value
              placeholder={loading ? 'loading…' : 'no models'}
            >
              {triggerLabel(selected, thinkingLevel)}
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
            className={cn(
              'z-50 min-w-[var(--radix-select-trigger-width)] overflow-hidden border border-rule bg-bg text-ink font-mono text-[13px] lowercase shadow-lg',
              'data-[state=open]:animate-in data-[state=closed]:animate-out',
              'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
            )}
          >
            <SelectPrimitive.Viewport className="p-1 max-h-[60vh]">
              {groups.map((g) => {
                const unconfigured = g.options.length === 0
                return (
                  <SelectPrimitive.Group key={g.label}>
                    <div className="flex items-center justify-between gap-2 pr-2 pt-2 pb-1">
                      <span className="flex min-w-0 items-baseline gap-1.5">
                        <SelectPrimitive.Label className="px-3 text-[11px] uppercase tracking-[0.12em] text-ink-faint">
                          {g.label}
                        </SelectPrimitive.Label>
                        {unconfigured ? (
                          <span className="text-[10px] lowercase tracking-normal text-ink-ghost">
                            not configured
                          </span>
                        ) : null}
                      </span>
                      {presentSet.has(g.label) ? (
                        <button
                          type="button"
                          aria-label={`configure ${g.label}`}
                          title={`configure ${g.label} in harness configuration`}
                          onPointerDown={(e) => e.stopPropagation()}
                          onClick={(e) => {
                            e.preventDefault()
                            e.stopPropagation()
                            window.location.hash = HARNESS_CONFIG_HASH
                          }}
                          className="text-ink-faint hover:text-ink transition-colors p-0.5 -mr-0.5"
                        >
                          <Settings size={12} />
                        </button>
                      ) : null}
                    </div>
                    {g.options.map((opt) => {
                      const expanded = expandedModelId === opt.id
                      const showThinking = opt.supportsThinking === true
                      return (
                        <div key={opt.id}>
                          <div className="group/model relative flex items-center">
                            <SelectPrimitive.Item
                              value={opt.id}
                              className={cn(
                                'relative flex flex-1 min-w-0 items-center pl-7 pr-12 py-1.5 cursor-pointer outline-none select-none',
                                'data-[highlighted]:bg-rule data-[highlighted]:text-ink',
                                'data-[state=checked]:text-ink',
                              )}
                            >
                              <SelectPrimitive.ItemIndicator className="absolute left-2 top-1/2 -translate-y-1/2 text-ink">
                                <Check size={12} aria-hidden />
                              </SelectPrimitive.ItemIndicator>
                              <SelectPrimitive.ItemText className="truncate">
                                {opt.label}
                              </SelectPrimitive.ItemText>
                            </SelectPrimitive.Item>
                            {showThinking ? (
                              <button
                                type="button"
                                aria-label={`edit thinking for ${opt.label}`}
                                aria-expanded={expanded}
                                onPointerDown={(e) => e.stopPropagation()}
                                onClick={(e) => {
                                  e.preventDefault()
                                  e.stopPropagation()
                                  setExpandedModelId((current) =>
                                    current === opt.id ? null : opt.id,
                                  )
                                }}
                                className={cn(
                                  'absolute right-2 top-1/2 -translate-y-1/2 px-1.5 py-0.5 text-[11px] text-ink-ghost transition-opacity',
                                  'opacity-0 group-hover/model:opacity-100 focus-visible:opacity-100',
                                  expanded && 'opacity-100 text-ink',
                                )}
                              >
                                edit
                              </button>
                            ) : null}
                          </div>
                          {expanded && showThinking ? (
                            <div className="ml-7 border-l border-rule-2 pl-2 pb-1">
                              {THINKING_LEVELS.map((level) => {
                                const active =
                                  safeValue === opt.id &&
                                  thinkingLevel === level
                                return (
                                  <button
                                    key={level}
                                    type="button"
                                    onPointerDown={(e) => e.stopPropagation()}
                                    onClick={(e) => {
                                      e.preventDefault()
                                      e.stopPropagation()
                                      handleThinkingPick(opt.id, level)
                                    }}
                                    className={cn(
                                      'flex w-full items-center gap-2 px-2 py-1 text-left text-[12px] transition-colors',
                                      active
                                        ? 'text-ink'
                                        : 'text-ink-faint hover:text-ink hover:bg-rule',
                                    )}
                                  >
                                    <span
                                      aria-hidden
                                      className={cn(
                                        'w-3 text-center',
                                        active ? 'opacity-100' : 'opacity-0',
                                      )}
                                    >
                                      <Check size={12} aria-hidden />
                                    </span>
                                    {formatThinkingLevel(level)}
                                  </button>
                                )
                              })}
                            </div>
                          ) : null}
                        </div>
                      )
                    })}
                  </SelectPrimitive.Group>
                )
              })}
            </SelectPrimitive.Viewport>
          </SelectPrimitive.Content>
        </SelectPrimitive.Portal>
      </SelectPrimitive.Root>

      {ctx ? (
        <button
          type="button"
          aria-label="refresh model list"
          title="refresh model list from providers"
          disabled={ctx.refreshingModels || disabled}
          onClick={() => {
            void ctx.refreshModels()
          }}
          className="text-ink-ghost hover:text-ink transition-colors p-1 disabled:opacity-50"
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
