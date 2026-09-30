import type { Host, ModelOption } from '@iii-dev/console-ui'
import { createContext, useEffect, useState } from 'react'

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

/**
 * The router's model catalog, re-read when the router announces a change. An
 * unreachable router (or no `host`) leaves the list empty; callers keep
 * showing their stored value.
 */
export function useModelCatalog(host: Host | null): { options: ModelOption[]; loading: boolean } {
  const [state, setState] = useState<{ options: ModelOption[]; loading: boolean }>({
    options: [],
    loading: host !== null,
  })

  useEffect(() => {
    if (!host) {
      setState({ options: [], loading: false })
      return
    }
    let live = true
    let latest = 0
    const load = () => {
      const request = ++latest
      // `resolve().then` turns a synchronous throw (no connection yet) into the same empty list.
      Promise.resolve()
        .then(() => host.iii.trigger('router::models::list', {}, { timeoutMs: RPC_TIMEOUT_MS }))
        .then(modelOptionsFromCatalog, () => [])
        .then((options) => {
          // A slower, older response must not overwrite a newer one.
          if (live && request === latest) setState({ options, loading: false })
        })
    }
    load()

    const handlerId = `${MODELS_CHANGED_FN}::${++subscriptions}`
    let offHandler: (() => void) | undefined
    let offTrigger: (() => void) | undefined
    try {
      offHandler = host.iii.on(handlerId, load)
      offTrigger = host.iii.registerTrigger({
        type: 'router::models::changed',
        function_id: `${handlerId}::${host.iii.browserId}`,
        config: {},
      })
    } catch {
      // A console without the fan-out keeps the list it read once.
      offTrigger?.()
      offHandler?.()
      offTrigger = undefined
      offHandler = undefined
    }
    return () => {
      live = false
      offTrigger?.()
      offHandler?.()
    }
  }, [host])

  return state
}
