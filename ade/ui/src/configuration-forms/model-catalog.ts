import type { Host, ModelOption } from '@iii-dev/console-ui'
import { createContext, useCallback, useEffect, useRef, useState } from 'react'

/**
 * The console `host` of the script that registered the form. Provided by
 * `configurationForm(id, host)`, so a hot reload (new script, new `host`)
 * replaces it with the form instead of leaving a stale module-level handle.
 * `null` outside a registered form; model fields then render without a catalog.
 */
export const ConfigurationHostContext = createContext<Host | null>(null)

const RPC_TIMEOUT_MS = 10_000
/** Handler id prefix; `host.iii.on` makes it tab-scoped (`<id>::<browserId>`). */
const MODELS_CHANGED_FN = 'console-ui::config-models-changed'

let subscriptions = 0

function trimmed(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function reasoningEfforts(value: unknown): ModelOption['reasoningEfforts'] {
  if (!Array.isArray(value)) return undefined
  const efforts = value.flatMap((raw) => {
    if (!raw || typeof raw !== 'object') return []
    const row = raw as Record<string, unknown>
    const effort = trimmed(row.effort)
    return effort ? [{ effort, description: trimmed(row.description) || undefined }] : []
  })
  return efforts.length > 0 ? efforts : undefined
}

/**
 * `router::models::list` rows as picker options: the id is
 * `${provider}::${id}` unless the router already qualified it.
 */
export function modelOptionsFromCatalog(response: unknown): ModelOption[] {
  const models = (response as { models?: unknown } | null | undefined)?.models
  if (!Array.isArray(models)) return []
  return models.flatMap((raw) => {
    if (!raw || typeof raw !== 'object') return []
    const row = raw as Record<string, unknown>
    const id = trimmed(row.id)
    if (!id) return []
    const provider = trimmed(row.provider)
    return [
      {
        id: provider && !id.includes('::') ? `${provider}::${id}` : id,
        label: trimmed(row.display_name) || id,
        contextWindow: typeof row.context_window === 'number' ? row.context_window : undefined,
        supportsThinking: typeof row.supports_thinking === 'boolean' ? row.supports_thinking : undefined,
        supportsVision: typeof row.supports_vision === 'boolean' ? row.supports_vision : undefined,
        reasoningEfforts: reasoningEfforts(row.reasoning_efforts),
      },
    ]
  })
}

/**
 * Keep the stored model visible when the catalog does not list it (router
 * unreachable, provider removed): the picker only shows a value it has an
 * option for, and a form must never read as unset while a model is configured.
 * Every effort level is offered for it so the picker does not reset the stored
 * level on a model it knows nothing about.
 */
export function withStoredModel(options: ModelOption[], stored: string | null): ModelOption[] {
  if (!stored || options.some((option) => option.id === stored)) return options
  return [...options, { id: stored, label: stored, supportsThinking: true }]
}

/** A `router::provider::list` row, as far as the automatic model rule reads it. */
export interface ProviderRow {
  id: string
  available: boolean
  configured: boolean
  /** `null` when the provider owns its sign-in (openai-codex, claude-code). */
  credentialEnvVar: string | null
  /** Empty when the provider declares none or its catalog holds none of them. */
  defaultModel: string
}

export function providerRowsFromList(response: unknown): ProviderRow[] {
  const rows = (response as { providers?: unknown } | null | undefined)?.providers
  if (!Array.isArray(rows)) return []
  return rows.flatMap((raw) => {
    if (!raw || typeof raw !== 'object') return []
    const row = raw as Record<string, unknown>
    const id = trimmed(row.id)
    if (!id) return []
    return [
      {
        id,
        available: row.available === true,
        configured: row.configured === true,
        credentialEnvVar: typeof row.credential_env_var === 'string' ? row.credential_env_var : null,
        defaultModel: trimmed(row.default_model),
      },
    ]
  })
}

export type AutomaticReason = 'provider_default' | 'first_listed_model'

/** An id the router lists that no chat call can use (embedding or rerank models). */
const looksNonChat = (id: string) => /embed|rerank/.test(id.toLowerCase())

const usable = (provider: ProviderRow) =>
  provider.available && (provider.configured || provider.credentialEnvVar === null)

/** Code-unit order, never `localeCompare`: the Rust twins compare bytes. */
const byCodeUnit = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0)

/**
 * The model an unset `extraction_model` (memory) or `llm_model`
 * (memory-consolidate) resolves to. Its twins are `automatic_model` in
 * memory/src/extract.rs and memory-consolidate/src/llm.rs; the three run one
 * case table, memory/tests/fixtures/automatic-model-cases.json.
 *
 * A provider is usable when it is available and either configured or owns its
 * sign-in (no credential env var). First the default model of the first usable
 * provider by id, else the first listed chat model of a usable provider by
 * (provider, model). Ids that look like embedding or rerank models never win.
 */
export function automaticModel(
  providers: readonly ProviderRow[],
  options: readonly ModelOption[],
): { id: string; reason: AutomaticReason } | null {
  const ready = providers.filter(usable).sort((a, b) => byCodeUnit(a.id, b.id))
  const preferred = ready.find((provider) => provider.defaultModel !== '' && !looksNonChat(provider.defaultModel))
  if (preferred) return { id: `${preferred.id}::${preferred.defaultModel}`, reason: 'provider_default' }
  const owners = new Set(ready.map((provider) => provider.id))
  // Sorted as (provider, model) pairs: `a-b::c` and `a::z` order differently as strings.
  const [first] = options
    .flatMap((option) => {
      const at = option.id.indexOf('::')
      const provider = option.id.slice(0, at)
      const model = option.id.slice(at + 2)
      return at > 0 && owners.has(provider) && !looksNonChat(model) ? [[provider, model] as const] : []
    })
    .sort(([a, x], [b, y]) => byCodeUnit(a, b) || byCodeUnit(x, y))
  return first ? { id: `${first[0]}::${first[1]}`, reason: 'first_listed_model' } : null
}

export const NO_USABLE_MODEL_LABEL = 'Automatic · no usable model'

export interface ModelFieldState {
  /** What the picker selects: a bare id with exactly one owner reads as that owner's option. */
  value: string | null
  options: ModelOption[]
  /** The Automatic row and, while unset, the trigger. */
  automaticLabel: string
  notice: 'none' | 'no-usable-model' | 'unreachable'
  /** Quiet text under the label. */
  meta: string | undefined
}

/**
 * Everything a model field shows, from the stored value and the catalog. The
 * stored value is never rewritten here: a bare id shown as its owner's option
 * stays bare until the user picks another model.
 */
export function modelFieldState({
  stored,
  options,
  providers,
  loading,
  failed,
  automatic,
}: {
  stored: string | null
  options: ModelOption[]
  providers: readonly ProviderRow[]
  loading: boolean
  failed: boolean
  automatic: boolean
}): ModelFieldState {
  const listed = stored !== null && options.some((option) => option.id === stored)
  const owners =
    stored !== null && !listed && !stored.includes('::')
      ? options.filter((option) => option.id.endsWith(`::${stored}`))
      : []
  const value = owners.length === 1 ? owners[0].id : stored
  const settled = !loading && !failed
  const pick = settled ? automaticModel(providers, options) : null
  // shortcut: the router's label, not the picker's formatted one, so a raw id reads raw; send the id with the label if that shows.
  const automaticLabel = !settled
    ? 'Automatic'
    : pick
      ? `Automatic · ${options.find((option) => option.id === pick.id)?.label ?? pick.id}`
      : NO_USABLE_MODEL_LABEL
  return {
    value,
    options: withStoredModel(options, value),
    automaticLabel,
    notice: failed
      ? 'unreachable'
      : automatic && settled && stored === null && pick === null
        ? 'no-usable-model'
        : 'none',
    meta:
      settled && stored !== null && !listed
        ? owners.length > 1
          ? 'Listed by more than one provider'
          : owners.length === 0
            ? 'Not in the router catalog'
            : undefined
        : undefined,
  }
}

export interface ModelCatalog {
  options: ModelOption[]
  providers: ProviderRow[]
  loading: boolean
  /** The router did not answer, so both lists are unknown rather than empty. */
  failed: boolean
  reload(): void
}

/**
 * The router's model catalog and providers, re-read when the router announces
 * a change to either. An unreachable router sets `failed`; without a `host`
 * both lists stay empty. Callers keep showing their stored value.
 */
export function useModelCatalog(host: Host | null): ModelCatalog {
  const [state, setState] = useState<Omit<ModelCatalog, 'reload'>>({
    options: [],
    providers: [],
    loading: host !== null,
    failed: false,
  })
  const loadRef = useRef<(() => void) | null>(null)
  const reload = useCallback(() => {
    const load = loadRef.current
    if (!load) return
    // `failed` holds until the answer, so the notice and its focused Retry button stay mounted.
    setState((current) => ({ ...current, loading: true }))
    load()
  }, [])

  useEffect(() => {
    if (!host) {
      setState({ options: [], providers: [], loading: false, failed: false })
      return
    }
    let live = true
    let latest = 0
    // `resolve().then` turns a synchronous throw (no connection yet) into a failed read.
    const read = (functionId: string) =>
      Promise.resolve().then(() => host.iii.call(functionId, {}, { timeoutMs: RPC_TIMEOUT_MS }))
    const load = () => {
      const request = ++latest
      Promise.all([read('router::models::list'), read('router::provider::list')])
        .then(
          ([models, providers]) => ({
            options: modelOptionsFromCatalog(models),
            providers: providerRowsFromList(providers),
            loading: false,
            failed: false,
          }),
          () => ({ options: [], providers: [], loading: false, failed: true }),
        )
        .then((next) => {
          // A slower, older response must not overwrite a newer one.
          if (live && request === latest) setState(next)
        })
    }
    loadRef.current = load
    load()

    const handlerId = `${MODELS_CHANGED_FN}::${++subscriptions}`
    const target = `${handlerId}::${host.iii.browserId}`
    const offs: Array<() => void> = []
    try {
      offs.push(host.iii.on(handlerId, load))
      offs.push(host.iii.observe('router::models::on-changed', target))
      offs.push(host.iii.observe('router::provider::on-changed', target))
    } catch {
      // A console that cannot observe keeps the lists it read once.
      for (const off of offs.splice(0).reverse()) off()
    }
    return () => {
      live = false
      loadRef.current = null
      // Observers go before the handler they deliver to.
      for (const off of offs.reverse()) off()
    }
  }, [host])

  return { ...state, reload }
}
