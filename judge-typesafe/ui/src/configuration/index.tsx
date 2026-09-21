import {
  Button,
  Chip,
  type ConfigFormProps,
  type ExtensionIii,
  Input,
  Select,
  type SelectOption,
  SettingsField,
  SettingsList,
  SettingsSection,
  StatusPanel,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'

/** What the worker uses when the entry stores no model. */
export const BUILT_IN_MODEL = 'jev-latest'

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
type Engine = Pick<ExtensionIii, 'trigger'>

/** The catalog as the running worker answers it with its saved credentials. */
export async function listModels(iii: Engine): Promise<ModelCard[]> {
  const reply = await iii.trigger<ModelsReply>('judge-typesafe::models::list', { timeout_ms: 15_000 }, { timeoutMs: 20_000 })
  if (reply?.status === 'ok' && Array.isArray(reply.models)) return reply.models
  // Typed provider refusals (`missing_key`, `http`, …) surface by code; bus failures by message.
  throw new Error(reply?.status === 'error' && reply.code ? reply.code : 'invalid_response')
}

/** Bind the form to the console's engine client once, at registration. */
export function createJevConfigForm(iii: Engine) {
  return function BoundJevConfigForm(props: ConfigFormProps) {
    return <JevConfigForm {...props} iii={iii} />
  }
}

export function JevConfigForm({ iii, ...props }: ConfigFormProps & { iii: Engine }) {
  const rootRef = useRef<HTMLDivElement>(null)
  // null = the first listing has not answered yet.
  const [catalog, setCatalog] = useState<ModelCard[] | null>(null)
  const [catalogError, setCatalogError] = useState<string | null>(null)
  const refresh = useCallback(() => {
    setCatalogError(null)
    listModels(iii)
      .then(setCatalog)
      .catch((error: unknown) => {
        setCatalogError(error instanceof Error ? error.message : String(error))
        setCatalog((current) => current ?? [])
      })
  }, [iii])
  useEffect(() => {
    refresh()
  }, [refresh])

  const focusField = props.focusField?.[0]
  useEffect(() => {
    if (!focusField || !knownFields.includes(focusField)) return
    const control = rootRef.current?.querySelector<HTMLElement>(`#jev-cfg-${focusField}`)
    control?.scrollIntoView?.({ block: 'center' })
    control?.focus()
  }, [focusField])

  if (props.value !== null && (typeof props.value !== 'object' || Array.isArray(props.value))) {
    return (
      <StatusPanel
        variant="info"
        headline="Judge TypeSafe configuration is supplied as a single value"
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
    ) : (
      <Chip tone="success">Key accepted · {catalog.length} models</Chip>
    )

  return (
    <div className="jev-ui-form" ref={rootRef}>
      <SettingsSection
        title="Credentials"
        description="The key judge-typesafe sends to TypeSafe for evaluations and model listing. Saved changes apply to new calls without restarting."
      >
        <SettingsList>
          <SettingsField
            id="jev-cfg-api_key"
            field="api_key"
            label="API key"
            description="Overrides TYPESAFE_API_KEY in the worker's environment. Clear it to fall back to that variable; restart judge-typesafe after changing the environment."
            error={props.errors?.get('/api_key')}
            meta={keyStatus}
            renderControl={(controlProps) => (
              <Input
                {...controlProps}
                type="password"
                autoComplete="new-password"
                spellCheck={false}
                aria-label="API key"
                placeholder="Use TYPESAFE_API_KEY"
                value={typeof value.api_key === 'string' ? value.api_key : ''}
                onChange={(next) => setString('api_key', next)}
              />
            )}
          />
        </SettingsList>
      </SettingsSection>
      <SettingsSection
        title="Model"
        description="Catalog as answered by judge-typesafe::models::list with the saved credentials. Callers may still name a model per call."
        action={
          <Button type="button" variant="ghost" size="sm" onClick={refresh} disabled={catalog === null}>
            Refresh
          </Button>
        }
      >
        <SettingsList>
          <SettingsField
            id="jev-cfg-model"
            field="model"
            label="Default model"
            description={`Used when an evaluation omits its model. Versioned ids stay accepted even when the catalog lists only aliases. Clear to use ${BUILT_IN_MODEL}.`}
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
              id={`jev-cfg-${field}`}
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
          detail="Save a key above, or export TYPESAFE_API_KEY in the judge-typesafe process and restart it. Evaluations and model listing answer missing_key until then."
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
