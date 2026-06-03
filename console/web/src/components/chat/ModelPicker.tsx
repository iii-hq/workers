import * as SelectPrimitive from '@radix-ui/react-select'
import { RefreshCw, Settings } from 'lucide-react'
import { Select, type SelectGroup } from '@/components/ui/Select'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { cn } from '@/lib/utils'
import {
  CATALOG_MODEL_KEY_SEP,
  type ModelId,
  type ModelOption,
} from '@/types/chat'

// Deep link to the harness configuration entry in the workers/config editor,
// where api keys + per-provider settings are now edited (the bespoke
// per-provider dialog was retired in favour of the schema-driven form).
const HARNESS_CONFIG_HASH = '#/configuration/workers/harness'

interface ModelPickerProps {
  value: ModelId | null
  options: ModelOption[]
  onChange: (next: ModelId) => void
  disabled?: boolean
  loading?: boolean
  className?: string
}

function groupByProvider(options: ModelOption[]): SelectGroup<ModelId>[] {
  const byProvider = new Map<string, { value: ModelId; label: string }[]>()
  for (const opt of options) {
    const provider = opt.id.split(CATALOG_MODEL_KEY_SEP)[0] || '—'
    const bucket = byProvider.get(provider) ?? []
    bucket.push({ value: opt.id, label: opt.label })
    byProvider.set(provider, bucket)
  }
  return [...byProvider.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([label, opts]) => ({ label, options: opts }))
}

export function ModelPicker({
  value,
  options,
  onChange,
  disabled,
  loading,
  className,
}: ModelPickerProps) {
  // Optional: present in the app, absent in isolated Storybook renders.
  const ctx = useConversationsCtxOptional()

  // Providers present as harness workers (from harness::provider::list).
  // Absent in Storybook or before the list resolves, in which case no empty
  // provider groups or gears appear until the dynamic list arrives.
  const presentIds = ctx?.presentProviders.map((p) => p.id) ?? []
  const presentSet = new Set<string>(presentIds)

  const pickerOptions = options
  const safeValue =
    value != null && pickerOptions.some((o) => o.id === value) ? value : undefined

  // Groups from the registered models, plus an empty group for each present
  // provider that has no models yet (present-but-unconfigured) so it still
  // shows up with a gear to open its configuration.
  const modelGroups = groupByProvider(pickerOptions)
  const grouped = new Set(modelGroups.map((g) => g.label))
  const emptyGroups: SelectGroup<ModelId>[] = presentIds
    .filter((id) => !grouped.has(id))
    .map((id) => ({ label: id, options: [] }))
  const groups = [...modelGroups, ...emptyGroups].sort((a, b) =>
    a.label.localeCompare(b.label),
  )

  return (
    <span className="inline-flex items-center gap-1">
      <Select<ModelId>
        value={safeValue}
        groups={groups}
        onChange={onChange}
        disabled={disabled || loading || groups.length === 0}
        placeholder={loading ? 'loading…' : 'no models'}
        aria-label={loading ? 'model (loading catalog)' : 'model'}
        aria-busy={loading || undefined}
        className={className}
        renderGroupHeader={(g) => {
          const unconfigured = g.options.length === 0
          return (
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
                  // Stop Radix Select from interpreting the click as an
                  // option-pick / outside-click; navigating to the config
                  // editor closes the dropdown naturally.
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
          )
        }}
      />

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
