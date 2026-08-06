/**
 * Custom configuration form for the `llm-router` configuration entry —
 * registered through `host.configForms`, replacing the console's generic
 * schema-driven form for this worker only.
 *
 * One card per provider (api_key / api_url / max_tokens / system_prompt),
 * a default-provider picker fed by the configured provider ids, the stream
 * budget knobs, and the routing-heuristics table. The form edits the
 * working draft via `onChange`; dirty tracking, save/reset, validation and
 * the SaveBar stay host-owned. Mirrors compose_entry_schema
 * (llm-router/src/config/schema.rs).
 *
 * The one opinion this form adds over the generic one: an api_key that
 * holds a PLAIN-TEXT secret gets a warning steering the operator to
 * `${ENV_VAR}` syntax — the configuration worker expands env references on
 * read, so the entry (and every export of it) never needs to carry the
 * secret itself.
 */

import { useEffect, useRef } from 'react'
import {
  type ConfigFormProps,
  type JsonValue,
  Select,
  type SelectOption,
} from '@iii-dev/console-ui'

type JsonObject = { [key: string]: JsonValue }

function asObject(v: JsonValue | undefined): JsonObject {
  return v && typeof v === 'object' && !Array.isArray(v) ? { ...v } : {}
}

function asString(v: JsonValue | undefined): string {
  return typeof v === 'string' ? v : ''
}

/** `${VAR}` (exactly one reference, nothing else) — the recommended shape. */
export function isEnvReference(v: string): boolean {
  return /^\$\{[A-Za-z_][A-Za-z0-9_]*\}$/.test(v.trim())
}

/** Mentions an env reference somewhere (partial templating still counts). */
function hasEnvReference(v: string): boolean {
  return /\$\{[A-Za-z_][A-Za-z0-9_]*\}/.test(v)
}

/** Suggest an env-var name for a provider: `ANTHROPIC_API_KEY`. */
function suggestedEnvVar(providerId: string): string {
  const slug = providerId.toUpperCase().replace(/[^A-Z0-9]+/g, '_')
  return `${slug || 'PROVIDER'}_API_KEY`
}

/**
 * The provider-declared identity prompt, from the entry schema — the
 * registration rides it in as the `system_prompt` field's `default`
 * (llm-router/src/config/schema.rs, system_prompt_schema).
 */
function providerPromptDefault(
  schema: Record<string, unknown> | null,
  id: string,
): string | null {
  const get = (o: unknown, k: string): unknown =>
    o && typeof o === 'object' && !Array.isArray(o)
      ? (o as Record<string, unknown>)[k]
      : undefined
  const field = get(
    get(get(get(get(get(schema, 'properties'), 'providers'), 'properties'), id), 'properties'),
    'system_prompt',
  )
  const dflt = get(field, 'default')
  return typeof dflt === 'string' && dflt.length > 0 ? dflt : null
}

/**
 * Options for a provider picker: the configured ids, plus the current
 * value when it names a provider that is not connected (so an operator
 * can SEE a stale selection instead of the placeholder lying about it).
 */
function providerOptions(
  providerIds: string[],
  current: string,
): SelectOption[] {
  const options = providerIds.map((id) => ({ value: id, label: id }))
  if (current && !providerIds.includes(current)) {
    options.push({
      value: current,
      label: `${current} (not connected)`,
    })
  }
  return options
}

/** `300000` → `5m`, `90500` → `1m 30s`, `800` → `800ms`. */
function formatMs(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  const s = Math.round(ms / 1000)
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  const rest = s % 60
  return rest ? `${m}m ${rest}s` : `${m}m`
}

/**
 * The stream-budget knobs (mirrors RouterSettings defaults,
 * llm-router/src/settings.rs). `echo` renders the wire value in the
 * operator's unit so nobody has to parse 300000 as five minutes.
 */
const SETTINGS_FIELDS = [
  {
    key: 'stream_timeout_ms',
    label: 'stream timeout (ms)',
    defaultValue: 300_000,
    echo: formatMs,
  },
  {
    key: 'idle_timeout_ms',
    label: 'idle timeout (ms)',
    defaultValue: 120_000,
    echo: formatMs,
  },
  {
    key: 'retry_max',
    label: 'retry max',
    defaultValue: 2,
    echo: (n: number) => (n === 1 ? '1 retry' : `${n} retries`),
  },
  {
    key: 'output_token_max',
    label: 'output token max',
    defaultValue: 32_000,
    echo: (n: number) => `${n.toLocaleString()} tokens`,
  },
] as const

export function LlmRouterConfigForm(props: ConfigFormProps) {
  const value = asObject(props.value)
  const providers = asObject(value.providers)
  const providerIds = Object.keys(providers)
  const settings = asObject(value.settings)
  const heuristics = Array.isArray(value.routing_heuristics)
    ? (value.routing_heuristics as JsonValue[])
    : []

  const commit = (patch: JsonObject) => props.onChange({ ...value, ...patch })

  // Deep-link focus (#/workers/configuration/llm-router/<field>): the host
  // only scroll-focuses the generic form's DOM ids, so honoring the request
  // is this override's job.
  const rootRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const field = props.focusField?.[0]
    if (!field || !rootRef.current) return
    const el = rootRef.current.querySelector<HTMLElement>(
      `[data-field="${CSS.escape(field)}"]`,
    )
    el?.scrollIntoView({ block: 'center' })
    el?.focus()
  }, [props.focusField])

  return (
    <div className="llmr-cfg" ref={rootRef}>
      <span className="llmr-cfg-caption">
        custom form · shipped by the llm-router worker
      </span>

      <span className="llmr-cfg-label">default provider</span>
      <div className="llmr-cfg-row" data-field="default_provider">
        <Select
          aria-label="default provider"
          value={asString(value.default_provider) || undefined}
          options={providerOptions(providerIds, asString(value.default_provider))}
          placeholder="no default — heuristics only"
          allowEmpty
          emptyLabel="no default"
          onClear={() => {
            const next = { ...value }
            delete next.default_provider
            props.onChange(next)
          }}
          onChange={(id) => commit({ default_provider: id })}
        />
      </div>

      <h3 className="llmr-cfg-heading">providers</h3>
      {providerIds.length === 0 ? (
        <div className="llmr-cfg-empty">
          No providers connected yet — provider workers register themselves
          here when they come up.
        </div>
      ) : (
        providerIds.map((id) => (
          <ProviderCard
            key={id}
            id={id}
            slice={asObject(providers[id])}
            promptDefault={providerPromptDefault(props.schema, id)}
            onChange={(next) =>
              commit({ providers: { ...providers, [id]: next } })
            }
          />
        ))
      )}

      <h3 className="llmr-cfg-heading">stream budgets</h3>
      <div className="llmr-cfg-grid">
        {SETTINGS_FIELDS.map((f) => {
          const set = typeof settings[f.key] === 'number'
          const effective = set ? (settings[f.key] as number) : f.defaultValue
          return (
            <div key={f.key}>
              <label className="llmr-cfg-label" htmlFor={`llmr-set-${f.key}`}>
                {f.label}
              </label>
              <input
                id={`llmr-set-${f.key}`}
                className="llmr-cfg-input"
                inputMode="numeric"
                value={set ? String(settings[f.key]) : ''}
                placeholder={String(f.defaultValue)}
                onChange={(e) => {
                  const next = { ...settings }
                  const n = Number(e.target.value)
                  if (e.target.value === '' || Number.isNaN(n)) {
                    delete next[f.key]
                  } else {
                    next[f.key] = n
                  }
                  commit({ settings: next })
                }}
              />
              <div className="llmr-cfg-echo">
                {set ? `= ${f.echo(effective)}` : `default · ${f.echo(effective)}`}
              </div>
            </div>
          )
        })}
      </div>

      <h3 className="llmr-cfg-heading">routing heuristics</h3>
      <p className="llmr-cfg-hint">
        First pattern (substring or regex) matching the requested model wins;
        no match falls through to the default provider.
      </p>
      {heuristics.map((h, i) => {
        const row = asObject(h)
        const update = (patch: JsonObject) => {
          const next = [...heuristics]
          next[i] = { ...row, ...patch }
          commit({ routing_heuristics: next })
        }
        return (
          // Rows have no id; order IS the routing priority, so index keys
          // are the honest choice here.
          // biome-ignore lint/suspicious/noArrayIndexKey: positional rows
          <div key={i} className="llmr-cfg-row llmr-cfg-heuristic">
            <input
              className="llmr-cfg-input"
              value={asString(row.pattern)}
              placeholder="pattern, e.g. gpt-"
              aria-label={`heuristic ${i + 1} pattern`}
              onChange={(e) => update({ pattern: e.target.value })}
            />
            <span className="llmr-cfg-arrow" aria-hidden>
              →
            </span>
            <Select
              aria-label={`heuristic ${i + 1} provider`}
              value={asString(row.provider) || undefined}
              options={providerOptions(providerIds, asString(row.provider))}
              placeholder="provider"
              onChange={(id) => update({ provider: id })}
            />
            <button
              type="button"
              className="llmr-cfg-remove"
              aria-label={`remove heuristic ${i + 1}`}
              onClick={() =>
                commit({
                  routing_heuristics: heuristics.filter((_, j) => j !== i),
                })
              }
            >
              ×
            </button>
          </div>
        )
      })}
      <button
        type="button"
        className="llmr-cfg-add"
        onClick={() =>
          commit({
            routing_heuristics: [...heuristics, { pattern: '', provider: '' }],
          })
        }
      >
        + add heuristic
      </button>

      {props.errors && props.errors.size > 0 ? (
        <div className="llmr-cfg-errors">
          {[...props.errors].map(([path, message]) => (
            <div key={path}>
              <code>{path}</code> {message}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}

function ProviderCard({
  id,
  slice,
  promptDefault,
  onChange,
}: {
  id: string
  slice: JsonObject
  /** The provider-declared identity prompt (schema default), if any. */
  promptDefault: string | null
  onChange(next: JsonObject): void
}) {
  const apiKey = asString(slice.api_key)
  // Plain text = something typed that is not an env reference. A partial
  // reference (`sk-${SUFFIX}`) still embeds secret material, so only the
  // pure `${VAR}` shape counts as safe.
  const plainTextKey = apiKey.length > 0 && !isEnvReference(apiKey)
  const systemPrompt = slice.system_prompt

  const set = (key: string, v: JsonValue | undefined) => {
    const next = { ...slice }
    if (v === undefined) delete next[key]
    else next[key] = v
    onChange(next)
  }

  return (
    <section className="llmr-cfg-card" data-field={`providers-${id}`}>
      <header className="llmr-cfg-card-head">{id}</header>

      <label className="llmr-cfg-label" htmlFor={`llmr-${id}-api-key`}>
        api key
      </label>
      <input
        id={`llmr-${id}-api-key`}
        className="llmr-cfg-input"
        // Env references are not secrets — keep them readable; anything
        // else gets masked like the generic password field.
        type={apiKey === '' || isEnvReference(apiKey) ? 'text' : 'password'}
        autoComplete="off"
        spellCheck={false}
        value={apiKey}
        placeholder={`\${${suggestedEnvVar(id)}}`}
        onChange={(e) => set('api_key', e.target.value || undefined)}
      />
      {plainTextKey ? (
        <div className="llmr-cfg-warning" role="alert">
          <strong>plain-text secret</strong> — this key is stored verbatim in
          the configuration entry (and every export of it). Set it as an
          environment variable on the engine host and reference it instead:{' '}
          <button
            type="button"
            className="llmr-cfg-envfix"
            title="replace with the env reference"
            onClick={() => set('api_key', `\${${suggestedEnvVar(id)}}`)}
          >
            {'${'}
            {suggestedEnvVar(id)}
            {'}'}
          </button>{' '}
          — the value expands when the entry is read, so the secret never
          lands in the store.
        </div>
      ) : isEnvReference(apiKey) ? (
        <div className="llmr-cfg-envok">
          env reference — expands on read, secret stays out of the store
        </div>
      ) : hasEnvReference(apiKey) ? (
        <div className="llmr-cfg-warning" role="alert">
          partial env reference — the literal part is still stored as plain
          text. Move the whole key into one variable.
        </div>
      ) : null}

      <label className="llmr-cfg-label" htmlFor={`llmr-${id}-api-url`}>
        api url
      </label>
      <input
        id={`llmr-${id}-api-url`}
        className="llmr-cfg-input"
        value={asString(slice.api_url)}
        placeholder="provider default"
        spellCheck={false}
        onChange={(e) => set('api_url', e.target.value || undefined)}
      />

      <label className="llmr-cfg-label" htmlFor={`llmr-${id}-max-tokens`}>
        max tokens
      </label>
      <input
        id={`llmr-${id}-max-tokens`}
        className="llmr-cfg-input"
        inputMode="numeric"
        value={typeof slice.max_tokens === 'number' ? String(slice.max_tokens) : ''}
        placeholder="provider default"
        onChange={(e) => {
          const n = Number(e.target.value)
          set(
            'max_tokens',
            e.target.value === '' || Number.isNaN(n) ? undefined : n,
          )
        }}
      />

      <div className="llmr-cfg-prompt-head">
        <span className="llmr-cfg-label">system prompt</span>
        {typeof systemPrompt === 'string' ? (
          <button
            type="button"
            className="llmr-cfg-toggle"
            onClick={() => set('system_prompt', undefined)}
          >
            use provider default
          </button>
        ) : (
          <button
            type="button"
            className="llmr-cfg-toggle"
            // Prefill with the provider's own prompt: an override usually
            // starts as an edit of it, not a blank page.
            onClick={() => set('system_prompt', promptDefault ?? '')}
          >
            override
          </button>
        )}
      </div>
      {typeof systemPrompt === 'string' ? (
        <textarea
          className="llmr-cfg-input llmr-cfg-textarea"
          value={systemPrompt}
          rows={4}
          placeholder="override the provider-declared identity prompt"
          onChange={(e) => set('system_prompt', e.target.value)}
        />
      ) : promptDefault ? (
        <div className="llmr-cfg-prompt-preview" title={promptDefault}>
          {promptDefault}
        </div>
      ) : (
        <div className="llmr-cfg-echo">no provider-declared prompt</div>
      )}
    </section>
  )
}
