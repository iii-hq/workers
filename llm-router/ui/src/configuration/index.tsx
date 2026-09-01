/** Purpose-built configuration form for the LLM router. */

import {
  Button,
  type ConfigFormProps,
  Input,
  type JsonValue,
  Select,
  type SelectOption,
  SettingsList,
  SettingsRow,
  SettingsSection,
  Switch,
} from '@iii-dev/console-ui'
import { type ReactNode, useEffect, useRef } from 'react'
import { type ProviderFieldDefinition, providerCardIds, providerFieldDefinitions } from './provider-cards'

type JsonObject = { [key: string]: JsonValue }

function asObject(value: JsonValue | undefined): JsonObject {
  return value && typeof value === 'object' && !Array.isArray(value) ? { ...value } : {}
}

function asString(value: JsonValue | undefined): string {
  return typeof value === 'string' ? value : ''
}

/** `${VAR}` (exactly one reference, nothing else) — the recommended shape. */
export function isEnvReference(value: string): boolean {
  return /^\$\{[A-Za-z_][A-Za-z0-9_]*\}$/.test(value.trim())
}

function hasEnvReference(value: string): boolean {
  return /\$\{[A-Za-z_][A-Za-z0-9_]*\}/.test(value)
}

function suggestedEnvVar(providerId: string, fieldKey: string): string {
  const slug = `${providerId}_${fieldKey}`.toUpperCase().replace(/[^A-Z0-9]+/g, '_')
  return slug || 'PROVIDER_SECRET'
}

function providerOptions(providerIds: string[], current: string): SelectOption[] {
  const options = providerIds.map((id) => ({ value: id, label: id }))
  if (current && !providerIds.includes(current)) {
    options.push({ value: current, label: `${current} (not connected)` })
  }
  return options
}

function formatMs(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  const seconds = Math.round(ms / 1000)
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  const rest = seconds % 60
  return rest ? `${minutes}m ${rest}s` : `${minutes}m`
}

const SETTINGS_FIELDS = [
  {
    key: 'stream_timeout_ms',
    label: 'Stream timeout',
    description: 'Maximum total time for a streaming response, in milliseconds.',
    defaultValue: 300_000,
    echo: formatMs,
  },
  {
    key: 'idle_timeout_ms',
    label: 'Idle timeout',
    description: 'Maximum pause between stream events, in milliseconds.',
    defaultValue: 120_000,
    echo: formatMs,
  },
  {
    key: 'retry_max',
    label: 'Maximum retries',
    description: 'Number of retry attempts after a retryable provider failure.',
    defaultValue: 2,
    echo: (value: number) => (value === 1 ? '1 retry' : `${value} retries`),
  },
  {
    key: 'output_token_max',
    label: 'Output token limit',
    description: 'Router-wide ceiling for generated output tokens.',
    defaultValue: 32_000,
    echo: (value: number) => `${value.toLocaleString()} tokens`,
  },
] as const

export function LlmRouterConfigForm(props: ConfigFormProps) {
  const value = asObject(props.value)
  const providers = asObject(value.providers)
  const providerIds = providerCardIds(props.schema, props.value)
  const settings = asObject(value.settings)
  const heuristics = Array.isArray(value.routing_heuristics) ? (value.routing_heuristics as JsonValue[]) : []
  const commit = (patch: JsonObject) => props.onChange({ ...value, ...patch })

  // Canonical deep links use #/configuration/workers/llm-router/<field>.
  // The injected form owns focus because the host cannot know its DOM shape.
  const rootRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!props.focusField?.length || !rootRef.current) return
    const exact = props.focusField.join('-')
    const target =
      rootRef.current.querySelector<HTMLElement>(`[data-field="${CSS.escape(exact)}"]`) ??
      rootRef.current.querySelector<HTMLElement>(`[data-field="${CSS.escape(props.focusField[0])}"]`)
    target?.scrollIntoView({ block: 'center' })
    const focusable = target?.matches('input, button, [tabindex]')
      ? target
      : target?.querySelector<HTMLElement>('input, button, [tabindex]')
    focusable?.focus()
  }, [props.focusField])

  return (
    <div className="llmr-cfg" ref={rootRef}>
      <SettingsSection title="Routing" description="Choose the fallback provider used when no routing rule matches.">
        <SettingsList>
          <SettingsRow
            data-field="default_provider"
            label="Default provider"
            description="Requests fall back to this provider after all routing rules are evaluated."
            control={
              <div className="llmr-cfg-select">
                <Select
                  aria-label="Default provider"
                  value={asString(value.default_provider) || undefined}
                  options={providerOptions(providerIds, asString(value.default_provider))}
                  placeholder="No default provider"
                  allowEmpty
                  emptyLabel="No default provider"
                  onClear={() => {
                    const next = { ...value }
                    delete next.default_provider
                    props.onChange(next)
                  }}
                  onChange={(id) => commit({ default_provider: id })}
                />
              </div>
            }
          />
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        data-field="providers"
        title="Providers"
        description="Credentials, endpoints, limits, and provider-specific settings."
      >
        {providerIds.length === 0 ? (
          <SettingsList>
            <SettingsRow
              label="No providers connected"
              description="Provider workers appear here after they register with the router."
            />
          </SettingsList>
        ) : (
          <div className="llmr-cfg-provider-stack">
            {providerIds.map((id) => (
              <ProviderCard
                key={id}
                id={id}
                schema={props.schema}
                rootValue={props.value}
                slice={asObject(providers[id])}
                onChange={(next) => commit({ providers: { ...providers, [id]: next } })}
              />
            ))}
          </div>
        )}
      </SettingsSection>

      <SettingsSection
        data-field="settings"
        title="Stream budgets"
        description="Operational limits shared by every provider. Empty values use router defaults."
      >
        <SettingsList>
          {SETTINGS_FIELDS.map((field) => {
            const configured = typeof settings[field.key] === 'number'
            const effective = configured ? (settings[field.key] as number) : field.defaultValue
            return (
              <SettingsRow
                key={field.key}
                data-field={`settings-${field.key}`}
                label={field.label}
                description={field.description}
                meta={configured ? field.echo(effective) : `Default: ${field.echo(effective)}`}
                control={
                  <Input
                    className="llmr-cfg-number"
                    inputMode="numeric"
                    type="number"
                    step={field.key === 'retry_max' ? 1 : 'any'}
                    min={field.key === 'retry_max' ? 0 : undefined}
                    value={configured ? String(settings[field.key]) : ''}
                    placeholder={String(field.defaultValue)}
                    aria-label={field.label}
                    onChange={(nextValue) => {
                      const next = { ...settings }
                      const number = Number(nextValue)
                      if (nextValue === '' || Number.isNaN(number)) {
                        delete next[field.key]
                      } else {
                        next[field.key] = number
                      }
                      commit({ settings: next })
                    }}
                  />
                }
              />
            )
          })}
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        data-field="routing_heuristics"
        title="Routing rules"
        description="The first substring or regular-expression match wins."
        action={
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() =>
              commit({
                routing_heuristics: [...heuristics, { pattern: '', provider: '' }],
              })
            }
          >
            Add rule
          </Button>
        }
      >
        <SettingsList>
          {heuristics.length === 0 ? (
            <SettingsRow
              label="No routing rules"
              description="Requests use the default provider when no rules are configured."
            />
          ) : (
            heuristics.map((heuristic, index) => {
              const row = asObject(heuristic)
              const update = (patch: JsonObject) => {
                const next = [...heuristics]
                next[index] = { ...row, ...patch }
                commit({ routing_heuristics: next })
              }
              return (
                <SettingsRow
                  key={index}
                  label={`Rule ${index + 1}`}
                  control={
                    <div className="llmr-cfg-heuristic">
                      <Input
                        value={asString(row.pattern)}
                        placeholder="Pattern, for example gpt-"
                        aria-label={`Rule ${index + 1} pattern`}
                        onChange={(pattern) => update({ pattern })}
                      />
                      <span aria-hidden="true">→</span>
                      <div className="llmr-cfg-select">
                        <Select
                          aria-label={`Rule ${index + 1} provider`}
                          value={asString(row.provider) || undefined}
                          options={providerOptions(providerIds, asString(row.provider))}
                          placeholder="Provider"
                          onChange={(provider) => update({ provider })}
                        />
                      </div>
                    </div>
                  }
                  action={
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      aria-label={`Remove rule ${index + 1}`}
                      onClick={() =>
                        commit({
                          routing_heuristics: heuristics.filter((_, candidate) => candidate !== index),
                        })
                      }
                    >
                      Remove
                    </Button>
                  }
                />
              )
            })
          )}
        </SettingsList>
      </SettingsSection>

      {props.errors && props.errors.size > 0 ? (
        <div className="llmr-cfg-errors" role="alert">
          {[...props.errors].map(([path, message]) => (
            <div key={path}>
              {path ? `${path}: ` : ''}
              {message}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}

function ProviderCard({
  id,
  schema,
  rootValue,
  slice,
  onChange,
}: {
  id: string
  schema: Record<string, unknown> | null
  rootValue: JsonValue
  slice: JsonObject
  onChange(next: JsonObject): void
}) {
  const fields = providerFieldDefinitions(schema, rootValue, id)
  const set = (key: string, nextValue: JsonValue | undefined) => {
    const next = { ...slice }
    if (nextValue === undefined) delete next[key]
    else next[key] = nextValue
    onChange(next)
  }

  return (
    <SettingsSection className="llmr-cfg-provider" data-field={`providers-${id}`} title={id}>
      <SettingsList>
        {fields.length === 0 ? (
          <SettingsRow label="No settings required" description="This provider registers no configurable fields." />
        ) : (
          fields.map((field) => (
            <ProviderFieldRow
              key={field.key}
              providerId={id}
              field={field}
              value={slice[field.key]}
              configured={field.key in slice}
              onChange={(nextValue) => set(field.key, nextValue)}
            />
          ))
        )}
      </SettingsList>
    </SettingsSection>
  )
}

function ProviderFieldRow({
  providerId,
  field,
  value,
  configured,
  onChange,
}: {
  providerId: string
  field: ProviderFieldDefinition
  value: JsonValue | undefined
  configured: boolean
  onChange(value: JsonValue | undefined): void
}) {
  const fieldId = `llmr-${providerId}-${field.key}`
  const description =
    field.description ??
    (field.required
      ? 'Required by this provider.'
      : field.defaultValue !== undefined
        ? `Provider default: ${String(field.defaultValue)}`
        : undefined)

  if (field.kind === 'structured') {
    return (
      <SettingsRow
        data-field={`providers-${providerId}-${field.key}`}
        label={field.label}
        description={description}
        meta="Structured value preserved unchanged"
      />
    )
  }

  let control: ReactNode
  if (field.kind === 'boolean') {
    control = (
      <Switch
        checked={value === true || (!configured && field.defaultValue === true)}
        onChange={(event) => onChange(event.currentTarget.checked)}
        aria-label={field.label}
      />
    )
  } else if (field.enumValues && field.enumValues.length > 0) {
    const current = asString(value)
    const options = field.enumValues.map((entry) => ({
      value: entry,
      label: entry,
    }))
    if (current && !field.enumValues.includes(current)) {
      options.push({ value: current, label: `${current} (current)` })
    }
    control = (
      <div className="llmr-cfg-select">
        <Select
          aria-label={field.label}
          value={current || undefined}
          options={options}
          placeholder="Use provider default"
          allowEmpty={!field.required}
          emptyLabel="Use provider default"
          onClear={() => onChange(undefined)}
          onChange={onChange}
        />
      </div>
    )
  } else if (field.kind === 'number' || field.kind === 'integer') {
    control = (
      <Input
        id={fieldId}
        className="llmr-cfg-number"
        type="number"
        step={field.kind === 'integer' ? 1 : 'any'}
        inputMode="decimal"
        value={typeof value === 'number' ? String(value) : ''}
        placeholder={field.defaultValue !== undefined ? String(field.defaultValue) : 'Provider default'}
        aria-label={field.label}
        onChange={(nextValue) => {
          if (nextValue === '') {
            onChange(field.required ? '' : undefined)
            return
          }
          const number = Number(nextValue)
          onChange(Number.isNaN(number) ? nextValue : number)
        }}
      />
    )
  } else {
    const text = asString(value)
    const secret = field.writeOnly
    control = (
      <div className="llmr-cfg-secret-wrap">
        <Input
          id={fieldId}
          type={secret && text && !isEnvReference(text) ? 'password' : 'text'}
          autoComplete="off"
          spellCheck={false}
          value={text}
          placeholder={
            secret
              ? `\${${suggestedEnvVar(providerId, field.key)}}`
              : field.defaultValue !== undefined
                ? String(field.defaultValue)
                : 'Provider default'
          }
          aria-label={field.label}
          onChange={(nextValue) => onChange(nextValue === '' && !field.required ? undefined : nextValue)}
        />
        {secret ? (
          <SecretGuidance providerId={providerId} fieldKey={field.key} value={text} onChange={onChange} />
        ) : null}
      </div>
    )
  }

  return (
    <SettingsRow
      data-field={`providers-${providerId}-${field.key}`}
      label={field.label}
      description={description}
      control={control}
      action={
        configured && !field.required ? (
          <Button type="button" variant="ghost" size="sm" onClick={() => onChange(undefined)}>
            Reset
          </Button>
        ) : undefined
      }
    />
  )
}

function SecretGuidance({
  providerId,
  fieldKey,
  value,
  onChange,
}: {
  providerId: string
  fieldKey: string
  value: string
  onChange(value: JsonValue | undefined): void
}) {
  if (!value) return null
  const environmentVariable = suggestedEnvVar(providerId, fieldKey)
  if (isEnvReference(value)) {
    return <div className="llmr-cfg-envok">Environment reference. The secret stays out of configuration storage.</div>
  }
  const partial = hasEnvReference(value)
  return (
    <div className="llmr-cfg-warning" role="alert">
      {partial
        ? 'This partial environment reference still stores literal secret text.'
        : 'This plain-text secret will be stored in configuration.'}{' '}
      <button type="button" className="llmr-cfg-envfix" onClick={() => onChange(`\${${environmentVariable}}`)}>
        Use {'${'}
        {environmentVariable}
        {'}'}
      </button>
    </div>
  )
}
