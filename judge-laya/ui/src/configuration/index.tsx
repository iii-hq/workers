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

const MODELS = [
  { value: 'laya', label: 'laya', description: 'English · ModernBERT-large · 421M · 512 tokens' },
  { value: 'laya-multilingual', label: 'laya-multilingual', description: '100+ languages · mmBERT-base · 322M · 1024 tokens' },
]
const limitFields = [
  { field: 'batch_questions', label: 'Questions per batch', fallback: 16, description: 'Questions scored in one forward pass; cancellation and deadlines are checked between batches. Clear to use 16.' },
  { field: 'max_request_bytes', label: 'Maximum request bytes', fallback: 8388608, description: 'Maximum encoded request size. Clear to use 8388608 (8 MiB).' },
  { field: 'max_timeout_ms', label: 'Maximum timeout (ms)', fallback: 300000, description: 'Maximum caller timeout. Clear to use 300000 (5 minutes).' },
]
const knownFields = ['model', 'revision', 'threads', ...limitFields.map(({ field }) => field)]

interface ModelCard {
  name: string
  description: string
  release_date: string
}
type ModelsReply = { status: 'ok'; models: ModelCard[] } | { status: 'error'; code: string }
type Engine = Pick<ExtensionIii, 'trigger'>

/** What the running worker loaded; throws the typed code on refusal. */
export async function loadedModel(iii: Engine): Promise<ModelCard> {
  const reply = await iii.trigger<ModelsReply>('judge-laya::models::list', { timeout_ms: 15_000 }, { timeoutMs: 20_000 })
  if (reply?.status === 'ok' && reply.models?.[0]) return reply.models[0]
  throw new Error(reply?.status === 'error' && reply.code ? reply.code : 'invalid_response')
}

export function createLayaConfigForm(iii: Engine) {
  return function BoundLayaConfigForm(props: ConfigFormProps) {
    return <LayaConfigForm {...props} iii={iii} />
  }
}

export function LayaConfigForm({ iii, ...props }: ConfigFormProps & { iii: Engine }) {
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
    const control = rootRef.current?.querySelector<HTMLElement>(`#laya-cfg-${focusField}`)
    control?.scrollIntoView?.({ block: 'center' })
    control?.focus()
  }, [focusField])

  if (props.value !== null && (typeof props.value !== 'object' || Array.isArray(props.value))) {
    return (
      <StatusPanel
        variant="info"
        headline="Judge laya configuration is supplied as a single value"
        detail="Edit this value in the configuration source. It is preserved until you replace it with an object."
      />
    )
  }
  const value = props.value ?? {}
  const setString = (field: string, raw: string | undefined) => {
    const next = { ...value }
    if (raw === undefined || raw === '') delete next[field]
    else next[field] = raw
    props.onChange(next)
  }
  const setNumber = (field: string, raw: string) => {
    const next = { ...value }
    if (raw === '') delete next[field]
    else {
      const number = Number(raw)
      if (!Number.isSafeInteger(number) || number <= 0) return
      next[field] = number
    }
    props.onChange(next)
  }
  const unassociatedErrors = [...(props.errors?.entries() ?? [])].filter(
    ([pointer]) => !knownFields.some((field) => pointer === `/${field}`),
  )
  const selectedModel = typeof value.model === 'string' ? value.model : 'laya'
  const status =
    loaded === null && !probeError ? (
      <Chip tone="neutral">Checking the worker…</Chip>
    ) : probeError ? (
      <Chip tone="warning">Worker not answering · {probeError}</Chip>
    ) : loaded && loaded.name !== selectedModel ? (
      <Chip tone="warning">Running {loaded.name} · restart to load {selectedModel}</Chip>
    ) : (
      <Chip tone="success">Running {loaded?.name} · {loaded?.release_date}</Chip>
    )

  return (
    <div className="laya-ui-form" ref={rootRef}>
      <SettingsSection
        title="Checkpoint"
        description="The model runs inside this worker on the CPU: no API key, no external service. The checkpoint is downloaded from the Hugging Face Hub at start and cached."
      >
        <SettingsList>
          <SettingsField
            id="laya-cfg-model"
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
                placeholder="laya"
                allowEmpty
                emptyLabel="Built-in default (laya)"
                onClear={() => setString('model', undefined)}
                onChange={(next) => setString('model', next)}
                aria-label="Model"
              />
            )}
          />
          <SettingsField
            id="laya-cfg-revision"
            field="revision"
            label="Revision"
            description="Hugging Face branch, tag or commit of convaiinnovations/laya. Clear to follow main."
            error={props.errors?.get('/revision')}
            renderControl={(controlProps) => (
              <Input
                {...controlProps}
                type="text"
                autoComplete="off"
                spellCheck={false}
                aria-label="Revision"
                placeholder="main"
                value={typeof value.revision === 'string' ? value.revision : ''}
                onChange={(next) => setString('revision', next)}
              />
            )}
          />
          <SettingsField
            id="laya-cfg-threads"
            field="threads"
            label="CPU threads"
            description="Threads for the forward pass, applied at the next worker start. Hybrid CPUs are fastest around their performance-core count; the built-in default is min(8, logical cores)."
            error={props.errors?.get('/threads')}
            renderControl={(controlProps) => (
              <Input
                {...controlProps}
                type="number"
                min={1}
                step={1}
                aria-label="CPU threads"
                placeholder="8"
                value={typeof value.threads === 'number' ? String(value.threads) : ''}
                onChange={(next) => setNumber('threads', next)}
              />
            )}
          />
        </SettingsList>
      </SettingsSection>
      <SettingsSection
        title="Execution limits"
        description="Positive integers only. These hot-reload for new calls."
      >
        <SettingsList>
          {limitFields.map(({ field, label, fallback, description }) => (
            <SettingsField
              key={field}
              id={`laya-cfg-${field}`}
              field={field}
              label={label}
              description={description}
              error={props.errors?.get(`/${field}`)}
              renderControl={(controlProps) => (
                <Input
                  {...controlProps}
                  type="number"
                  min={1}
                  step={1}
                  aria-label={label}
                  placeholder={String(fallback)}
                  value={typeof value[field] === 'number' ? String(value[field]) : ''}
                  onChange={(next) => setNumber(field, next)}
                />
              )}
            />
          ))}
        </SettingsList>
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
