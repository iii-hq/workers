import {
  Input,
  ModelPicker,
  SettingsField,
  SettingsList,
  type ConfigFormProps,
  type Host,
  type JsonValue,
  type ModelOption,
  type ReasoningEffortOption,
} from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'

function asObject(value: JsonValue | undefined): Record<string, JsonValue> {
  return value && typeof value === 'object' && !Array.isArray(value) ? value : {}
}

interface CatalogRow {
  id?: unknown
  provider?: unknown
  display_name?: unknown
  context_window?: unknown
  supports_thinking?: unknown
  supports_vision?: unknown
  reasoning_efforts?: unknown
}

function efforts(value: unknown): ReasoningEffortOption[] | undefined {
  if (!Array.isArray(value)) return undefined
  const rows = value.flatMap((raw: { effort?: unknown; description?: unknown }) =>
    typeof raw?.effort === 'string' && raw.effort
      ? [{ effort: raw.effort, ...(typeof raw.description === 'string' ? { description: raw.description } : {}) }]
      : [],
  )
  return rows.length ? rows : undefined
}

/** The chat's catalog: `router::models::list`, keyed `provider::id` as the writer reads it. */
export function modelOptions(rows: unknown): ModelOption[] {
  if (!Array.isArray(rows)) return []
  return rows.flatMap((raw: CatalogRow) => {
    if (typeof raw?.id !== 'string' || typeof raw.provider !== 'string' || !raw.id || !raw.provider) return []
    return [
      {
        id: `${raw.provider}::${raw.id}`,
        label: typeof raw.display_name === 'string' && raw.display_name ? raw.display_name : raw.id,
        contextWindow: typeof raw.context_window === 'number' ? raw.context_window : undefined,
        supportsThinking: typeof raw.supports_thinking === 'boolean' ? raw.supports_thinking : undefined,
        supportsVision: typeof raw.supports_vision === 'boolean' ? raw.supports_vision : undefined,
        reasoningEfforts: efforts(raw.reasoning_efforts),
      },
    ]
  })
}

function useModelCatalog(host: Host) {
  const [options, setOptions] = useState<ModelOption[]>([])
  const [loading, setLoading] = useState(true)
  useEffect(() => {
    let live = true
    host.iii
      .call<{ models?: unknown }>('router::models::list', {})
      .then((result) => live && setOptions(modelOptions(result?.models)))
      .catch(() => live && setOptions([]))
      .finally(() => live && setLoading(false))
    return () => {
      live = false
    }
  }, [host])
  return { options, loading }
}

export function ControlPlaneConfigForm({ host, value, onChange, errors }: ConfigFormProps & { host: Host }) {
  const config = asObject(value)
  const summary = asObject(config.summary)
  const catalog = useModelCatalog(host)
  const setModel = (model: string | null) => {
    const next = { ...summary }
    if (model) next.model = model
    else delete next.model
    // An effort belongs to its model; a new model starts at its default.
    if (model !== summary.model) delete next.reasoning
    onChange({ ...config, summary: next })
  }
  const setReasoning = (level: string) => {
    const next = { ...summary }
    if (level && level !== 'default') next.reasoning = level
    else delete next.reasoning
    onChange({ ...config, summary: next })
  }
  return (
    <SettingsList>
      <SettingsField
        label="Summary model"
        field="summary.model"
        description="The model that writes the summary and the diary, and its reasoning effort. The same catalog as the chat."
        error={errors?.get('/summary/model')}
        renderControl={() => (
          <ModelPicker
            value={typeof summary.model === 'string' ? summary.model : null}
            options={catalog.options}
            loading={catalog.loading}
            thinkingLevel={typeof summary.reasoning === 'string' ? summary.reasoning : 'default'}
            showReasoningEffort
            onChange={setModel}
            onThinkingLevelChange={setReasoning}
            automatic={{ label: 'No summary', onSelect: () => setModel(null) }}
          />
        )}
      />
      <SettingsField
        label="Stall timeout"
        field="stall_after_min"
        description="Minutes without progress before a waiting run is flagged as stalled."
        error={errors?.get('/stall_after_min')}
        renderControl={(props) => (
          <Input
            {...props}
            type="number"
            min={0}
            step="any"
            placeholder="30"
            value={String(config.stall_after_min ?? '')}
            onChange={(minutes) => {
              const next = { ...config }
              if (minutes) next.stall_after_min = Number(minutes)
              else delete next.stall_after_min
              onChange(next)
            }}
          />
        )}
      />
    </SettingsList>
  )
}
