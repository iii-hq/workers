import {
  Chip,
  type ConfigFormProps,
  type ExtensionIii,
  Input,
  Select,
  SettingsField,
  SettingsList,
  SettingsSection,
  StatusPanel,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'

const MODELS = [{ value: 'decider-4b-v2', label: 'decider-4b-v2', description: 'decider-4b v2 Q4_K_M · 2.7 GB · Qwen3.5-4B-Base fine-tuned for typed decisions' }]
const numberFields = [
  { field: 'threads', section: 'model', label: 'CPU threads', placeholder: '8', description: 'Threads for prompt processing, applied at the next start. Hybrid CPUs are fastest around their performance-core count; the built-in default is min(8, logical cores).' },
  { field: 'gpu_layers', section: 'model', label: 'GPU layers', placeholder: 'all', allowZero: true, description: 'Layers offloaded to the GPU in Vulkan or Metal builds, applied at the next start. Clear to offload every layer when a GPU is present; 0 keeps the model on the CPU.' },
  { field: 'context_tokens', section: 'model', label: 'Context window (tokens)', placeholder: '16384', description: 'Longest prompt (state, question and options) accepted, applied at the next start. Longer prompts answer payload_too_large: the state is never truncated.' },
  { field: 'parallel_questions', section: 'model', label: 'Parallel questions', placeholder: '8', description: 'Questions about one state decoded together in a batch, applied at the next start. Pays off on a GPU; 1 decodes them one at a time.' },
  { field: 'max_request_bytes', section: 'limits', label: 'Maximum request bytes', placeholder: '8388608', description: 'Maximum encoded JSON bytes per evaluation. Clear to use 8388608 (8 MiB).' },
  { field: 'max_timeout_ms', section: 'limits', label: 'Maximum timeout (ms)', placeholder: '300000', description: 'Maximum caller timeout. Clear to use 300000 (5 minutes).' },
]
const knownFields = ['model', ...numberFields.map(({ field }) => field)]

interface ModelCard {
  name: string
  description: string
  release_date: string
}
type ModelsReply = { status: 'ok'; models: ModelCard[] } | { status: 'error'; code: string }
type Engine = Pick<ExtensionIii, 'trigger'>

/** What the running worker loaded; throws the typed code on refusal. */
export async function loadedModel(iii: Engine): Promise<ModelCard> {
  const reply = await iii.trigger<ModelsReply>('judge-decider::models::list', { timeout_ms: 15_000 }, { timeoutMs: 20_000 })
  if (reply?.status === 'ok' && reply.models?.[0]) return reply.models[0]
  throw new Error(reply?.status === 'error' && reply.code ? reply.code : 'invalid_response')
}

/** The hardware the worker reports in its model card ("… running in-process on X with …"). */
export function deviceOf(card: ModelCard): string {
  return /running in-process on (.+?) with a/.exec(card.description)?.[1] ?? 'unknown device'
}

export function createDeciderConfigForm(iii: Engine) {
  return function BoundDeciderConfigForm(props: ConfigFormProps) {
    return <DeciderConfigForm {...props} iii={iii} />
  }
}

export function DeciderConfigForm({ iii, ...props }: ConfigFormProps & { iii: Engine }) {
  const rootRef = useRef<HTMLDivElement>(null)
  // null = the first probe has not answered yet.
  const [loaded, setLoaded] = useState<ModelCard | null>(null)
  const [probeError, setProbeError] = useState<string | null>(null)
  const refresh = useCallback(() => {
    setProbeError(null)
    loadedModel(iii)
      .then(setLoaded)
      .catch((error: unknown) => setProbeError(error instanceof Error ? error.message : String(error)))
  }, [iii])
  useEffect(() => {
    refresh()
  }, [refresh])

  const focusField = props.focusField?.[0]
  useEffect(() => {
    if (!focusField || !knownFields.includes(focusField)) return
    const control = rootRef.current?.querySelector<HTMLElement>(`#decider-cfg-${focusField}`)
    control?.scrollIntoView?.({ block: 'center' })
    control?.focus()
  }, [focusField])

  if (props.value !== null && (typeof props.value !== 'object' || Array.isArray(props.value))) {
    return (
      <StatusPanel
        variant="info"
        headline="Judge decider configuration is supplied as a single value"
        detail="Edit this value in the configuration source. It is preserved until you replace it with an object."
      />
    )
  }
  const value = props.value ?? {}
  const setModel = (raw: string | undefined) => {
    const next = { ...value }
    if (raw === undefined || raw === '') delete next.model
    else next.model = raw
    props.onChange(next)
  }
  const setNumber = (field: string, raw: string, allowZero = false) => {
    const next = { ...value }
    if (raw === '') delete next[field]
    else {
      const number = Number(raw)
      if (!Number.isSafeInteger(number) || number < (allowZero ? 0 : 1)) return
      next[field] = number
    }
    props.onChange(next)
  }
  const unassociatedErrors = [...(props.errors?.entries() ?? [])].filter(
    ([pointer]) => !knownFields.some((field) => pointer === `/${field}`),
  )
  const selectedModel = typeof value.model === 'string' ? value.model : 'decider-4b-v2'
  const status =
    loaded === null && !probeError ? (
      <Chip tone="neutral">Checking the worker…</Chip>
    ) : probeError ? (
      <Chip tone="warning">Worker not answering · {probeError}</Chip>
    ) : loaded && loaded.name !== selectedModel ? (
      <Chip tone="warning">Running {loaded.name} · restart to load {selectedModel}</Chip>
    ) : (
      <Chip tone="success">
        Running {loaded?.name} on {loaded ? deviceOf(loaded) : ''}
      </Chip>
    )
  const numberField = ({ field, label, placeholder, description, allowZero }: (typeof numberFields)[number]) => (
    <SettingsField
      key={field}
      id={`decider-cfg-${field}`}
      field={field}
      label={label}
      description={description}
      error={props.errors?.get(`/${field}`)}
      renderControl={(controlProps) => (
        <Input
          {...controlProps}
          type="number"
          min={allowZero ? 0 : 1}
          step={1}
          aria-label={label}
          placeholder={placeholder}
          value={typeof value[field] === 'number' ? String(value[field]) : ''}
          onChange={(next) => setNumber(field, next, allowZero)}
        />
      )}
    />
  )

  return (
    <div className="decider-ui-form" ref={rootRef}>
      <SettingsSection
        title="Model"
        description="decider-4b runs inside this worker through llama.cpp and answers from the next-token logits of the option labels, at its fitted temperature: no API key, no external service. The GGUF is downloaded from the Hugging Face Hub on first load and cached."
      >
        <SettingsList>
          <SettingsField
            id="decider-cfg-model"
            field="model"
            label="Model"
            description="Applied at the next worker start."
            error={props.errors?.get('/model')}
            meta={status}
            renderControl={(controlProps) => (
              <Select
                {...controlProps}
                value={typeof value.model === 'string' ? value.model : undefined}
                options={MODELS}
                placeholder="decider-4b-v2"
                allowEmpty
                emptyLabel="Built-in default (decider-4b-v2)"
                onClear={() => setModel(undefined)}
                onChange={(next) => setModel(next)}
                aria-label="Model"
              />
            )}
          />
          {numberFields.filter(({ section }) => section === 'model').map(numberField)}
        </SettingsList>
      </SettingsSection>
      <SettingsSection title="Execution limits" description="Positive integers only. These hot-reload for new calls.">
        <SettingsList>{numberFields.filter(({ section }) => section === 'limits').map(numberField)}</SettingsList>
      </SettingsSection>
      {unassociatedErrors.length > 0 ? (
        <StatusPanel
          variant="alert"
          headline="Review the configuration errors"
          detail={unassociatedErrors.map(([pointer, message]) => (
            <div key={pointer}>
              {pointer ? `${pointer}: ` : ''}
              {message}
            </div>
          ))}
        />
      ) : null}
    </div>
  )
}
