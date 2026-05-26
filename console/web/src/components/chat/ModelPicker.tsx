import * as SelectPrimitive from '@radix-ui/react-select'
import { Settings } from 'lucide-react'
import { useState } from 'react'
import { ProviderSettingsDialog } from '@/components/providers/ProviderSettingsDialog'
import {
  ACTIVE_PROVIDERS,
  type ActiveProvider,
  ENV_VAR_MAP,
} from '@/components/providers/provider-registry'
import { Select, type SelectGroup } from '@/components/ui/Select'
import {
  CATALOG_MODEL_KEY_SEP,
  type ModelId,
  type ModelOption,
} from '@/types/chat'

const ENV_VAR_BY_ID = new Map(ENV_VAR_MAP)
const ACTIVE_PROVIDER_SET: ReadonlySet<string> = new Set(ACTIVE_PROVIDERS)

function isActiveProvider(id: string): id is ActiveProvider {
  return ACTIVE_PROVIDER_SET.has(id)
}

interface ModelPickerProps {
  value: ModelId
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
  const [settingsProvider, setSettingsProvider] =
    useState<ActiveProvider | null>(null)

  const pickerOptions =
    options.length > 0 ? options : [{ id: value, label: value }]
  const safeValue = pickerOptions.some((o) => o.id === value)
    ? value
    : pickerOptions[0].id

  return (
    <>
      <Select<ModelId>
        value={safeValue}
        groups={groupByProvider(pickerOptions)}
        onChange={onChange}
        disabled={disabled || loading}
        aria-label={loading ? 'model (loading catalog)' : 'model'}
        aria-busy={loading || undefined}
        className={className}
        renderGroupHeader={(g) => (
          <div className="flex items-center justify-between gap-2 pr-2 pt-2 pb-1">
            <SelectPrimitive.Label className="px-3 text-[11px] uppercase tracking-[0.12em] text-ink-faint">
              {g.label}
            </SelectPrimitive.Label>
            {isActiveProvider(g.label) ? (
              <button
                type="button"
                aria-label={`configure ${g.label}`}
                title={`configure ${g.label}`}
                // Stop Radix Select from interpreting the click as an
                // option-pick / outside-click; the gear opens the provider
                // settings dialog, which steals focus and closes the
                // dropdown naturally.
                onPointerDown={(e) => e.stopPropagation()}
                onClick={(e) => {
                  e.preventDefault()
                  e.stopPropagation()
                  setSettingsProvider(g.label as ActiveProvider)
                }}
                className="text-ink-faint hover:text-ink transition-colors p-0.5 -mr-0.5"
              >
                <Settings size={12} />
              </button>
            ) : null}
          </div>
        )}
      />

      {settingsProvider ? (
        <ProviderSettingsDialog
          provider={settingsProvider}
          envVar={ENV_VAR_BY_ID.get(settingsProvider) ?? ''}
          open
          onOpenChange={(open) => {
            if (!open) setSettingsProvider(null)
          }}
        />
      ) : null}
    </>
  )
}
