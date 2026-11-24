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
  ShadcnTextarea,
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
  const savedLabels = asObject(config.needs_you).card_labels
  const [labelsText, setLabelsText] = useState(() =>
    Array.isArray(savedLabels) ? savedLabels.join(', ') : '',
  )
  const setModel = (model: string | null) => {
    const next = { ...summary }
    if (model) next.model = model
    else delete next.model
    // An effort belongs to its model; a new model starts at its default.
    if (model !== summary.model) delete next.reasoning
    onChange({ ...config, summary: next })
  }
  const controller = asObject(config.controller)
  const setController = (patch: Record<string, JsonValue | null>) => {
    const next: Record<string, JsonValue> = { ...controller }
    for (const [key, value] of Object.entries(patch)) {
      if (value === null) delete next[key]
      else next[key] = value
    }
    onChange({ ...config, controller: next })
  }
  const setSummaryText = (key: 'guidance' | 'language', text: string) => {
    const next = { ...summary }
    if (text.trim()) next[key] = text
    else delete next[key]
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
        label="Controller model"
        field="controller.model"
        description="The model of the controller you chat with from the control plane, and the effort a new chat starts on (minimal to xhigh; another effort applies to its first reply only). Unset, it uses the summary's."
        error={errors?.get('/controller/model')}
        renderControl={() => (
          <ModelPicker
            value={typeof controller.model === 'string' ? controller.model : null}
            options={catalog.options}
            loading={catalog.loading}
            thinkingLevel={typeof controller.reasoning === 'string' ? controller.reasoning : 'default'}
            showReasoningEffort
            // An effort belongs to its model; a new model starts at its default.
            onChange={(model) => setController({ model: model ?? null, reasoning: null })}
            onThinkingLevelChange={(level) => setController({ reasoning: level && level !== 'default' ? level : null })}
            automatic={{ label: 'Same as the summary', onSelect: () => setController({ model: null, reasoning: null }) }}
          />
        )}
      />
      <SettingsField
        label="Writer guidance"
        field="summary.guidance"
        description="What the summary and diary should focus on, highlight or skip, and their tone and length. Sent with every refresh; the writer's rules (references, no invented facts) still apply."
        error={errors?.get('/summary/guidance')}
        renderControl={(props) => (
          <ShadcnTextarea
            data-field={props['data-field' as keyof typeof props] as string | undefined}
            rows={4}
            maxLength={4000}
            placeholder="Highlight blocked cards and failing PRs. Keep the brief to two sentences."
            value={typeof summary.guidance === 'string' ? summary.guidance : ''}
            onChange={(event) => setSummaryText('guidance', event.target.value)}
          />
        )}
      />
      <SettingsField
        label="Summary language"
        field="summary.language"
        description="The language of the summary and diary. Card keys and timestamps stay as they are."
        error={errors?.get('/summary/language')}
        renderControl={(props) => (
          <Input
            {...props}
            placeholder="English"
            value={typeof summary.language === 'string' ? summary.language : ''}
            onChange={(language) => setSummaryText('language', language)}
          />
        )}
      />
      <SettingsField
        label="Card labels that need you"
        field="needs_you.card_labels"
        description="Open cards with one of these labels show up in Needs you, comma separated. The “needs you” pill always counts."
        error={errors?.get('/needs_you/card_labels')}
        renderControl={(props) => (
          <Input
            {...props}
            placeholder="human-only, só humano, decisão"
            value={labelsText}
            onChange={(text) => {
              // Keep what is typed (a trailing comma included); save the parsed labels.
              setLabelsText(text)
              const labels = text.split(',').map((label) => label.trim()).filter(Boolean)
              const next = { ...config }
              if (labels.length) next.needs_you = { ...asObject(config.needs_you), card_labels: labels }
              else delete next.needs_you
              onChange(next)
            }}
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
