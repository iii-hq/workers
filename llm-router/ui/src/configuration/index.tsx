/**
 * Custom configuration form for the `llm-router` configuration entry —
 * registered through `host.configForms`, replacing the console's generic
 * schema-driven form for this worker only.
 *
 * One card per provider (api key first; url / max tokens behind advanced;
 * system prompt edited in a dialog), a default-provider picker, stream budgets, and the
 * routing-heuristics table. The form edits the working draft via
 * `onChange`; dirty tracking, save/reset, validation and the SaveBar stay
 * host-owned. Mirrors compose_entry_schema (llm-router/src/config/schema.rs).
 *
 * The one opinion this form adds over the generic one: an api_key that
 * holds a PLAIN-TEXT secret gets a warning steering the operator to
 * `${ENV_VAR}` syntax — the configuration worker expands env references on
 * read, so the entry (and every export of it) never needs to carry the
 * secret itself.
 */

import { type ConfigFormProps, type Host, type JsonValue, Select, type SelectOption } from '@iii-dev/console-ui'
import { useEffect, useId, useRef, useState } from 'react'
import { minutesToMs, msToMinutes } from './duration'
import { FieldError } from './field-error'
import { moveItem, winningHeuristicIndex } from './heuristics'
import { errorAt, pointer } from './pointers'
import { isEnvReference, ProviderCard } from './provider-card'
import {
  type LiveProvider,
  parseProviderList,
  providerCardIds,
  providerDisplayName,
  providerRuntimeStatus,
  schemaProviderIds,
  sliceHasKey,
  visibleProviderIds,
} from './provider-cards'

type JsonObject = { [key: string]: JsonValue }

function asObject(v: JsonValue | undefined): JsonObject {
  return v && typeof v === 'object' && !Array.isArray(v) ? { ...v } : {}
}

function asString(v: JsonValue | undefined): string {
  return typeof v === 'string' ? v : ''
}

/**
 * The provider-declared identity prompt, from the entry schema — the
 * registration rides it in as the `system_prompt` field's `default`
 * (llm-router/src/config/schema.rs, system_prompt_schema).
 */
export function providerPromptDefault(schema: Record<string, unknown> | null, id: string): string | null {
  const get = (o: unknown, k: string): unknown =>
    o && typeof o === 'object' && !Array.isArray(o) ? (o as Record<string, unknown>)[k] : undefined
  const field = get(
    get(get(get(get(get(schema, 'properties'), 'providers'), 'properties'), id), 'properties'),
    'system_prompt',
  )
  const dflt = get(field, 'default')
  return typeof dflt === 'string' && dflt.length > 0 ? dflt : null
}

function providerOptions(providerIds: string[], current: string, live: LiveProvider[] | null): SelectOption[] {
  const names = new Map((live ?? []).map((p) => [p.id, p.display_name]))
  const options = providerIds.map((id) => ({
    value: id,
    label: providerDisplayName(id, names.get(id)),
  }))
  if (current && !providerIds.includes(current)) {
    options.push({
      value: current,
      label: `${providerDisplayName(current)} (not connected)`,
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
 * llm-router/src/settings.rs). Timeouts are edited in minutes and stored
 * as milliseconds; `echo` still renders the wire value.
 */
const SETTINGS_FIELDS = [
  {
    key: 'stream_timeout_ms',
    label: 'stream timeout (min)',
    defaultValue: 300_000,
    echo: formatMs,
    scale: 'minutes' as const,
  },
  {
    key: 'idle_timeout_ms',
    label: 'idle timeout (min)',
    defaultValue: 120_000,
    echo: formatMs,
    scale: 'minutes' as const,
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

export function LlmRouterConfigForm(props: ConfigFormProps & { host?: Host }) {
  const value = asObject(props.value)
  const providers = asObject(value.providers)
  const allIds = providerCardIds(props.schema, props.value)
  const schemaIds = schemaProviderIds(props.schema)
  const settings = asObject(value.settings)
  const heuristics = Array.isArray(value.routing_heuristics) ? (value.routing_heuristics as JsonValue[]) : []
  const defaultProvider = asString(value.default_provider)
  const live = useProviderRuntime(props.host)
  const [budgetsOpen, setBudgetsOpen] = useState(true)
  const [heuristicsOpen, setHeuristicsOpen] = useState(true)
  const [probe, setProbe] = useState('')

  const commit = (patch: JsonObject) => props.onChange({ ...value, ...patch })

  const hasKey = (id: string) => sliceHasKey(providers[id])
  const visibleIds = visibleProviderIds({
    ids: allIds,
    schemaIds,
    live,
    hasKey,
    filter: 'all',
  })

  const heuristicRows = heuristics.map((h) => {
    const row = asObject(h)
    return { pattern: asString(row.pattern), provider: asString(row.provider) }
  })
  const winner = winningHeuristicIndex(probe, heuristicRows)

  const liveById = new Map((live ?? []).map((p) => [p.id, p]))
  const shownErrors = new Set<string>([
    pointer('default_provider'),
    ...allIds.flatMap((id) => [
      pointer('providers', id, 'api_key'),
      pointer('providers', id, 'api_url'),
      pointer('providers', id, 'max_tokens'),
      pointer('providers', id, 'system_prompt'),
    ]),
    ...SETTINGS_FIELDS.map((f) => pointer('settings', f.key)),
    ...heuristics.flatMap((_, i) => [
      pointer('routing_heuristics', i, 'pattern'),
      pointer('routing_heuristics', i, 'provider'),
    ]),
  ])
  const leftoverErrors = [...(props.errors ?? [])].filter(([path]) => path && !shownErrors.has(path))
  const budgetsHaveErrors = SETTINGS_FIELDS.some((f) => errorAt(props.errors, 'settings', f.key) !== null)
  const heuristicsHaveErrors = heuristics.some(
    (_, i) =>
      errorAt(props.errors, 'routing_heuristics', i, 'pattern') !== null ||
      errorAt(props.errors, 'routing_heuristics', i, 'provider') !== null,
  )

  useEffect(() => {
    if (budgetsHaveErrors) setBudgetsOpen(true)
  }, [budgetsHaveErrors])
  useEffect(() => {
    if (heuristicsHaveErrors) setHeuristicsOpen(true)
  }, [heuristicsHaveErrors])

  // Deep-link focus (#/workers/configuration/llm-router/<field>): the host
  // only scroll-focuses the generic form's DOM ids, so honoring the request
  // is this override's job.
  const rootRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const field = props.focusField?.[0]
    if (!field || !rootRef.current) return
    const el = rootRef.current.querySelector<HTMLElement>(`[data-field="${CSS.escape(field)}"]`)
    el?.scrollIntoView({ block: 'center' })
    el?.focus()
  }, [props.focusField])

  return (
    <div className="llmr-cfg" ref={rootRef}>
      <span className="llmr-cfg-label">default provider</span>
      <div className="llmr-cfg-row" data-field="default_provider">
        <Select
          aria-label="default provider"
          value={defaultProvider || undefined}
          options={providerOptions(allIds, defaultProvider, live)}
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
      <FieldError message={errorAt(props.errors, 'default_provider')} />

      <h3 className="llmr-cfg-heading">providers</h3>
      {allIds.length === 0 ? (
        <div className="llmr-cfg-empty">
          Waiting for provider workers to register. If this stays empty, check that provider-* workers are installed and
          that no leftover iii process is holding the engine on :49134.
        </div>
      ) : (
        visibleIds.map((id) => {
          const status = providerRuntimeStatus(id, live, schemaIds)
          return (
            <ProviderCard
              key={id}
              id={id}
              label={providerDisplayName(id, liveById.get(id)?.display_name)}
              status={status}
              isDefault={defaultProvider === id}
              slice={asObject(providers[id])}
              promptDefault={providerPromptDefault(props.schema, id)}
              errors={props.errors}
              onChange={(next) => commit({ providers: { ...providers, [id]: next } })}
              onSetDefault={() => commit({ default_provider: id })}
            />
          )
        })
      )}

      <button
        type="button"
        className="llmr-cfg-disclosure"
        aria-expanded={budgetsOpen}
        onClick={() => setBudgetsOpen((open) => !open)}
      >
        <span className="llmr-cfg-disclosure-chevron" aria-hidden>
          {budgetsOpen ? '▾' : '▸'}
        </span>
        stream budgets
        <span className="llmr-cfg-disclosure-hint">timeout · retries · token cap</span>
      </button>
      {budgetsOpen ? (
        <div className="llmr-cfg-grid">
          {SETTINGS_FIELDS.map((f) => {
            const set = typeof settings[f.key] === 'number'
            const effective = set ? (settings[f.key] as number) : f.defaultValue
            const scale = 'scale' in f ? f.scale : undefined
            const display = scale === 'minutes' ? msToMinutes(effective) : effective
            return (
              <div key={f.key}>
                <label className="llmr-cfg-label" htmlFor={`llmr-set-${f.key}`}>
                  {f.label}
                </label>
                <input
                  id={`llmr-set-${f.key}`}
                  className="llmr-cfg-input"
                  inputMode="decimal"
                  value={set ? String(display) : ''}
                  placeholder={String(scale === 'minutes' ? msToMinutes(f.defaultValue) : f.defaultValue)}
                  onChange={(e) => {
                    const next = { ...settings }
                    const n = Number(e.target.value)
                    if (e.target.value === '' || Number.isNaN(n)) {
                      delete next[f.key]
                    } else {
                      next[f.key] = scale === 'minutes' ? minutesToMs(n) : n
                    }
                    commit({ settings: next })
                  }}
                />
                <div className="llmr-cfg-echo">{set ? `= ${f.echo(effective)}` : `default · ${f.echo(effective)}`}</div>
                <FieldError message={errorAt(props.errors, 'settings', f.key)} />
              </div>
            )
          })}
        </div>
      ) : null}

      <button
        type="button"
        className="llmr-cfg-disclosure"
        aria-expanded={heuristicsOpen}
        onClick={() => setHeuristicsOpen((open) => !open)}
      >
        <span className="llmr-cfg-disclosure-chevron" aria-hidden>
          {heuristicsOpen ? '▾' : '▸'}
        </span>
        routing heuristics
        <span className="llmr-cfg-disclosure-hint">
          {heuristics.length === 0 ? 'none' : `${heuristics.length} ${heuristics.length === 1 ? 'rule' : 'rules'}`}
        </span>
      </button>
      {heuristicsOpen ? (
        <>
          <p className="llmr-cfg-hint">
            First pattern (substring or regex) matching the requested model wins; no match falls through to the default
            provider. Catalog ownership still wins first after save — this preview is the draft table only.
          </p>
          <div className="llmr-cfg-row">
            <input
              className="llmr-cfg-input"
              value={probe}
              placeholder="try a model id, e.g. gpt-4.1"
              aria-label="try a model id"
              onChange={(e) => setProbe(e.target.value)}
            />
          </div>
          {probe.trim() ? (
            <div className="llmr-cfg-echo">
              {winner === null
                ? 'no heuristic match — falls through to the default provider'
                : `row ${winner + 1} → ${heuristicRows[winner].provider}`}
            </div>
          ) : null}
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
              <div key={i}>
                <div
                  className={
                    winner === i ? 'llmr-cfg-row llmr-cfg-heuristic is-win' : 'llmr-cfg-row llmr-cfg-heuristic'
                  }
                >
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
                    options={providerOptions(allIds, asString(row.provider), live)}
                    placeholder="provider"
                    onChange={(id) => update({ provider: id })}
                  />
                  <button
                    type="button"
                    className="llmr-cfg-remove"
                    aria-label={`move heuristic ${i + 1} up`}
                    disabled={i === 0}
                    onClick={() => commit({ routing_heuristics: moveItem(heuristics, i, i - 1) })}
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    className="llmr-cfg-remove"
                    aria-label={`move heuristic ${i + 1} down`}
                    disabled={i === heuristics.length - 1}
                    onClick={() => commit({ routing_heuristics: moveItem(heuristics, i, i + 1) })}
                  >
                    ↓
                  </button>
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
                <FieldError message={errorAt(props.errors, 'routing_heuristics', i, 'pattern')} />
                <FieldError message={errorAt(props.errors, 'routing_heuristics', i, 'provider')} />
              </div>
            )
          })}
          <button
            type="button"
            className="llmr-cfg-add"
            onClick={() => {
              setHeuristicsOpen(true)
              commit({
                routing_heuristics: [...heuristics, { pattern: '', provider: '' }],
              })
            }}
          >
            + add heuristic
          </button>
        </>
      ) : null}

      {leftoverErrors.length > 0 ? (
        <div className="llmr-cfg-errors">
          {leftoverErrors.map(([path, message]) => (
            <div key={path}>
              <code>{path}</code> {message}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}

/**
 * Live `router::provider::list`, kept fresh via `router::provider::changed`.
 * `on()` namespaces as `<fn>::<browserId>`; the trigger must name that same
 * id — do not prefix browserId twice.
 */
function useProviderRuntime(host: Host | undefined): LiveProvider[] | null {
  const [live, setLive] = useState<LiveProvider[] | null>(null)
  const instance = useId().replace(/[^a-zA-Z0-9]/g, '')

  useEffect(() => {
    if (!host) {
      setLive([])
      return
    }
    let cancelled = false
    const refresh = async () => {
      try {
        const raw = await host.iii.trigger<unknown>('router::provider::list', {})
        if (!cancelled) setLive(parseProviderList(raw))
      } catch {
        if (!cancelled) setLive([])
      }
    }
    void refresh()

    const localFn = `iii::llm-router-ui::providers-changed::${instance}`
    const boundFn = `${localFn}::${host.iii.browserId}`
    const off = host.iii.on(localFn, () => {
      void refresh()
    })
    let unreg: (() => void) | undefined
    try {
      unreg = host.iii.registerTrigger({
        type: 'router::provider::changed',
        function_id: boundFn,
        config: {},
      })
    } catch {
      // Older router without this trigger type — list snapshot still stands.
    }
    return () => {
      cancelled = true
      off()
      try {
        unreg?.()
      } catch {
        // SDK already disposed.
      }
    }
  }, [host, instance])

  return live
}

export { isEnvReference }
