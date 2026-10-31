import type { Host } from '@iii-dev/console-ui'

/** What a step's condition looked like when it fired. */
export interface Fired {
  /** The observed function whose call closed the step. */
  function_id: string
  payload: unknown
  at: number
}

/** The function a step waits on, as an observe source, plus what the page says about it. */
export interface Condition {
  function_id: string
  args?: Record<string, unknown>
  filters?: Record<string, unknown>
  label: string
  hint?: string
  prompt?: string
}

/**
 * Observe one step's condition with a browser-local handler, the same way the
 * console's own live pages do: a per-tab function id, then an observer of the
 * condition's function with that id as its target. Returns the disposer.
 *
 * The observer is real — the tour is not simulating one — so what the page
 * shows the operator is the event the engine delivered.
 */
export function bindCondition(
  host: Host,
  key: string,
  condition: Condition,
  onFire: (fired: Fired) => void,
): () => void {
  const localId = `learn-iii::condition::${key}`
  // The handler is registered before the observer, so a throw from `observe`
  // must take it back down. A live handler with no observer behind it would
  // fire alongside the next successful bind.
  let offHandler: () => void = () => {}
  try {
    offHandler = host.iii.on(localId, (payload: unknown) => {
      onFire({ function_id: condition.function_id, payload, at: Date.now() })
    })
    const offObserver = host.iii.observe(
      {
        function_id: condition.function_id,
        ...(condition.args ? { args: condition.args } : {}),
        ...(condition.filters ? { filters: condition.filters } : {}),
      },
      `${localId}::${host.iii.browserId}`,
    )
    return () => {
      offObserver()
      offHandler()
    }
  } catch {
    // The console may be restarting. The step stays waiting and binds again
    // on the next mount.
    offHandler()
    return () => {}
  }
}

/** The `input.<field>` a filter selects with `$eq`, when it does. */
function equalsFilter(filters: Record<string, unknown> | undefined, field: string): unknown {
  const filter = filters?.[`input.${field}`]
  return filter && typeof filter === 'object' && '$eq' in filter ? (filter as { $eq: unknown }).$eq : undefined
}

/**
 * What a condition waits on, read once: the fired event when it has already
 * happened, else `null`. Only a `state::on-changed` condition can be read back
 * — its key stays written — so a step the agent reported done while nothing
 * was observing still closes. Event-only conditions (a harness turn) leave no
 * trace to read; the console reports those itself when it has one
 * (`learn-iii::steps::observe`).
 */
export async function reconcileCondition(host: Host, condition: Condition): Promise<Fired | null> {
  if (condition.function_id !== 'state::on-changed') return null
  const scope = equalsFilter(condition.filters, 'scope')
  const key = equalsFilter(condition.filters, 'key')
  if (typeof scope !== 'string' || typeof key !== 'string') return null
  try {
    const value = await host.iii.call<unknown>('state::get', { scope, key })
    if (value === null || value === undefined) return null
    return {
      function_id: condition.function_id,
      payload: { scope, key, value, read_back: true },
      at: Date.now(),
    }
  } catch {
    return null
  }
}
