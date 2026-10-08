import {
  Button,
  Chip,
  type ConfigFormProps,
  type ExtensionIii,
  Input,
  type SecretKeyFieldProps,
  Select,
  type SelectOption,
  SettingsField,
  SettingsList,
  SettingsSection,
  StatusPanel,
} from '@iii-dev/console-ui'
import { type ComponentType, useCallback, useEffect, useRef, useState } from 'react'

/** What the worker uses when the entry stores no model. */
export const BUILT_IN_MODEL = 'gpt-6-luna'
const NOT_VISIBLE = `${BUILT_IN_MODEL} is not visible to this key`

const limitFields = [
  {
    field: 'max_request_bytes',
    label: 'Maximum request bytes',
    fallback: 8388608,
    description: 'Maximum encoded JSON bytes per upstream evaluation. Clear to use 8388608 (8 MiB).',
  },
  {
    field: 'max_response_bytes',
    label: 'Maximum response bytes',
    fallback: 8388608,
    description: 'Maximum response bytes per evaluation or model listing. Clear to use 8388608 (8 MiB).',
  },
  {
    field: 'max_timeout_ms',
    label: 'Maximum timeout (ms)',
    fallback: 300000,
    description: 'Maximum caller timeout, including queue waits and response reading. Clear to use 300000 (5 minutes).',
  },
]
const knownFields = ['api_key', 'model', ...limitFields.map(({ field }) => field)]

export interface ModelCard {
  name: string
  description: string
  release_date: string
}
type ModelsReply = { status: 'ok'; models: ModelCard[] } | { status: 'error'; code: string }
type Engine = Pick<ExtensionIii, 'trigger'> & Partial<Pick<ExtensionIii, 'on' | 'registerTrigger' | 'browserId'>>

/** Lets the worker reload a saved entry before the status asks it again. */
const SAVE_SETTLE_MS = 300
let liveSeq = 0

/** The supported models the running worker sees with its saved credentials. */
export async function listModels(iii: Engine): Promise<ModelCard[]> {
  const reply = await iii.trigger<ModelsReply>(
    'judge-openai::models::list',
    { timeout_ms: 15_000 },
    { timeoutMs: 20_000 },
  )
  if (reply?.status === 'ok' && Array.isArray(reply.models)) return reply.models
  // Typed provider refusals (`missing_key`, `http`, …) surface by code; bus failures by message.
  throw new Error(reply?.status === 'error' && reply.code ? reply.code : 'invalid_response')
}

/**
 * Bind the form to the console's engine client once, at registration.
 * `secretField` is the Console's shared `SecretKeyField` when it has one:
 * the key then lives in the secrets worker and `api_key` holds
 * `secret://OPENAI_API_KEY`, the way the setup wizard stores it. Older
 * Consoles keep the masked input below.
 */
export function createOpenAiConfigForm(iii: Engine, secretField?: ComponentType<SecretKeyFieldProps>) {
  return function BoundOpenAiConfigForm(props: ConfigFormProps) {
    return <OpenAiConfigForm {...props} iii={iii} secretField={secretField} />
  }
}

export function OpenAiConfigForm({
  iii,
  secretField: SecretField,
  ...props
}: ConfigFormProps & { iii: Engine; secretField?: ComponentType<SecretKeyFieldProps> }) {
  const rootRef = useRef<HTMLDivElement>(null)
  // null = a listing is in flight, so an empty catalog always comes from a reply.
  const [catalog, setCatalog] = useState<ModelCard[] | null>(null)
  const [catalogError, setCatalogError] = useState<string | null>(null)
  const refresh = useCallback(() => {
    setCatalog(null)
    setCatalogError(null)
    listModels(iii)
      .then(setCatalog)
      .catch((error: unknown) => {
        setCatalogError(error instanceof Error ? error.message : String(error))
        setCatalog([])
      })
  }, [iii])
  useEffect(() => {
    refresh()
  }, [refresh])

  // A save (a new key reference included) reloads the worker, so the status
  // asks again; without it the form would report the key it had at mount. The
  // binding names no id: an id-scoped binding holds the entry.
  const configurationId = props.id
  useEffect(() => {
    if (!iii.on || !iii.registerTrigger || !iii.browserId) return
    const handler = `iii::judge-openai-ui::configuration-${++liveSeq}`
    let timer: ReturnType<typeof setTimeout> | undefined
    const offHandler = iii.on<{ id?: unknown }>(handler, (event) => {
      if (event?.id !== configurationId) return
      clearTimeout(timer)
      timer = setTimeout(refresh, SAVE_SETTLE_MS)
    })
    let offTrigger = () => {}
    try {
      offTrigger = iii.registerTrigger({ type: 'configuration', function_id: `${handler}::${iii.browserId}`, config: {} })
    } catch {
      // No configuration trigger type on this engine: Refresh still works.
    }
    return () => {
      clearTimeout(timer)
      for (const off of [offTrigger, offHandler]) {
        try {
          off()
        } catch {
          // already gone
        }
      }
    }
  }, [iii, configurationId, refresh])

  const focusField = props.focusField?.[0]
  useEffect(() => {
    if (!focusField || !knownFields.includes(focusField)) return
    const control = rootRef.current?.querySelector<HTMLElement>(`#judge-openai-cfg-${focusField}`)
    control?.scrollIntoView?.({ block: 'center' })
    control?.focus()
  }, [focusField])

  // Hooks stay above the scalar bail-out: the host keeps this form mounted
  // while the value flips between shapes, so the hook order must not change.
  const raw = props.value
  const isObject = raw !== null && typeof raw === 'object' && !Array.isArray(raw)
  const value = isObject ? raw : {}
  // The stored key never reaches the DOM: the field is a blank replacement, and
  // `original` is the last key the host supplied that this form did not type.
  const storedKey = typeof value.api_key === 'string' && value.api_key !== '' ? value.api_key : undefined
  const [keyDraft, setKeyDraft] = useState('')
  const original = useRef(storedKey)
  if (storedKey !== (keyDraft === '' ? original.current : keyDraft)) original.current = storedKey

  if (!isObject) {
    return (
      <StatusPanel
        variant="info"
        headline="Judge OpenAI configuration is supplied as a single value"
        detail="Edit this value in the configuration source. It is preserved until you replace it with an object."
      />
    )
  }
  const setString = (field: string, raw: string | undefined) => {
    const next = { ...value }
    if (raw === undefined || raw === '') delete next[field]
    else next[field] = raw
    props.onChange(next)
  }
  const replaceKey = (draft: string) => {
    setKeyDraft(draft)
    setString('api_key', draft === '' ? original.current : draft)
  }
  const clearKey = () => {
    original.current = undefined
    setKeyDraft('')
    setString('api_key', undefined)
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

  // The key works, but OpenAI lists no supported model for it.
  const notVisible = catalog?.length === 0 && !catalogError
  const storedModel = typeof value.model === 'string' ? value.model : undefined
  const modelOptions: SelectOption[] = (catalog ?? []).map((card) => ({
    value: card.name,
    label: card.name,
    description: [card.description, card.release_date?.slice(0, 10)].filter(Boolean).join(' · '),
  }))
  if (storedModel && !modelOptions.some((option) => option.value === storedModel)) {
    modelOptions.push({ value: storedModel, label: storedModel, description: 'Not in the current catalog' })
  }
  const keyStatus =
    catalog === null ? (
      <Chip tone="neutral">Checking the worker…</Chip>
    ) : catalogError === 'missing_key' ? (
      <Chip tone="warning">No key reaches the worker</Chip>
    ) : catalogError ? (
      <Chip tone="warning">Listing failed · {catalogError}</Chip>
    ) : notVisible ? (
      <Chip tone="warning">{NOT_VISIBLE}</Chip>
    ) : (
      <Chip tone="success">Key accepted · {catalog.length} models</Chip>
    )

  return (
    <div className="openai-ui-form" ref={rootRef}>
      {SecretField ? (
        <SettingsSection title="Credentials" description="The key judge-openai sends to OpenAI. Save to apply it.">
          <div id="judge-openai-cfg-api_key" data-field="api_key">
            <SecretField
              name="OPENAI_API_KEY"
              label="API key"
              value={storedKey}
              onChange={(next) => setString('api_key', next)}
              consumers={['judge-openai']}
              keysUrl="https://platform.openai.com/api-keys"
              status={{
                checking: catalog === null,
                connected: catalog !== null && !catalogError && !notVisible,
                error:
                  catalogError === 'missing_key'
                    ? 'No key reaches the worker.'
                    : catalogError
                      ? `Listing failed · ${catalogError}`
                      : notVisible
                        ? NOT_VISIBLE
                        : undefined,
                detail: catalog && !catalogError && !notVisible ? `${catalog.length} models` : undefined,
              }}
            />
          </div>
        </SettingsSection>
      ) : (
        <SettingsSection
          title="Credentials"
          description="The key judge-openai sends to OpenAI for decisions and model listing. Saved changes apply to new calls without restarting."
        >
          <SettingsList>
            <SettingsField
              id="judge-openai-cfg-api_key"
              field="api_key"
              label="API key"
              description="Overrides OPENAI_API_KEY in the worker's environment. The stored key is never shown; type a new one to replace it, or clear it to fall back to the variable (restart judge-openai after changing the environment)."
              error={props.errors?.get('/api_key')}
              meta={
                <>
                  {keyStatus}
                  {storedKey ? (
                    <Button type="button" variant="ghost" size="sm" onClick={clearKey}>
                      Clear key
                    </Button>
                  ) : null}
                </>
              }
              renderControl={(controlProps) => (
                <Input
                  {...controlProps}
                  type="password"
                  autoComplete="new-password"
                  spellCheck={false}
                  aria-label="API key"
                  placeholder={original.current ? 'Configured · type a new key to replace it' : 'Use OPENAI_API_KEY'}
                  value={keyDraft}
                  onChange={replaceKey}
                />
              )}
            />
          </SettingsList>
        </SettingsSection>
      )}
      <SettingsSection
        title="Model"
        description="Supported models as answered by judge-openai::models::list with the saved credentials."
        action={
          <Button type="button" variant="ghost" size="sm" onClick={refresh} disabled={catalog === null}>
            Refresh
          </Button>
        }
      >
        <SettingsList>
          <SettingsField
            id="judge-openai-cfg-model"
            field="model"
            label="Default model"
            description={`Used when an evaluation omits its model. Clear to use ${BUILT_IN_MODEL}.`}
            error={props.errors?.get('/model')}
            renderControl={(controlProps) => (
              <Select
                {...controlProps}
                value={storedModel}
                options={modelOptions}
                placeholder={`Built-in default (${BUILT_IN_MODEL})`}
                allowEmpty
                emptyLabel={`Built-in default (${BUILT_IN_MODEL})`}
                onClear={() => setString('model', undefined)}
                onChange={(next) => setString('model', next)}
                aria-label="Default model"
                aria-busy={catalog === null}
              />
            )}
          />
        </SettingsList>
      </SettingsSection>
      <SettingsSection
        title="Execution limits"
        description="Positive integers only. Byte limits are local transport safeguards; provider token limits still apply. Callers can supply a shorter timeout."
      >
        <SettingsList>
          {limitFields.map(({ field, label, fallback, description }) => (
            <SettingsField
              key={field}
              id={`judge-openai-cfg-${field}`}
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
      {catalogError === 'missing_key' ? (
        <StatusPanel
          variant="warn"
          headline="No API key reaches the worker"
          detail="Save a key above, or export OPENAI_API_KEY in the judge-openai process and restart it. Evaluations and model listing answer missing_key until then."
        />
      ) : catalogError ? (
        <StatusPanel
          variant="warn"
          headline="Could not list models"
          detail={`${catalogError}. The stored model stays selectable; retry once the provider answers.`}
          action={
            <Button type="button" variant="ghost" size="sm" onClick={refresh}>
              Retry
            </Button>
          }
        />
      ) : null}
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
