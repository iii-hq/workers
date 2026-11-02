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
 * What `host.iii.observe` returns: its disposer, and — on consoles whose host
 * exposes the observer handle — the handle's `ready()` and `onStateChange`.
 * Older hosts return the bare disposer, so both are optional.
 */
type ObserveResult = (() => void) & {
  ready?: () => Promise<void>
  onStateChange?: (listener: (state: string, reason?: { code?: string; message?: string }) => void) => () => void
}

/** Why a condition's observer will never fire, in words for the step. */
export type ObserveFailure = string

/**
 * Observe one step's condition with a browser-local handler, the same way the
 * console's own live pages do: a per-tab function id, then an observer of the
 * condition's function with that id as its target. Returns the disposer.
 *
 * The observer is real — the tour is not simulating one — so what the page
 * shows the operator is the event the engine delivered.
 *
 * `onFail` is told when the observer will never fire: it could not be
 * created, the engine refused it (`ready()` rejects, or the state turns
 * `error`), or the engine dropped it (`disposed`). Disposing it ourselves
 * says nothing. A host that exposes no handle state can only report the
 * first case.
 */
export function bindCondition(
  host: Host,
  key: string,
  condition: Condition,
  onFire: (fired: Fired) => void,
  onFail?: (failure: ObserveFailure) => void,
): () => void {
  const localId = `learn-iii::condition::${key}`
  let closed = false
  const fail = (failure: ObserveFailure) => {
    if (closed) return
    closed = true
    onFail?.(failure)
  }
  // The handler is registered before the observer, so a throw from `observe`
  // must take it back down. A live handler with no observer behind it would
  // fire alongside the next successful bind.
  let offHandler: () => void = () => {}
  try {
    offHandler = host.iii.on(localId, (payload: unknown) => {
      onFire({ function_id: condition.function_id, payload, at: Date.now() })
    })
    const observed = host.iii.observe(
      {
        function_id: condition.function_id,
        ...(condition.args ? { args: condition.args } : {}),
        ...(condition.filters ? { filters: condition.filters } : {}),
      },
      `${localId}::${host.iii.browserId}`,
    ) as ObserveResult
    const offState = observed.onStateChange?.((state, reason) => {
      if (state === 'error' || state === 'disposed') {
        fail(`${condition.function_id} observer ${state}${reason?.code ? `: ${reason.code}` : ''}`)
      }
    })
    observed.ready?.().catch((cause: unknown) => {
      fail(`${condition.function_id} observer refused: ${cause instanceof Error ? cause.message : String(cause)}`)
    })
    return () => {
      closed = true
      offState?.()
      observed()
      offHandler()
    }
  } catch (cause) {
    // `observe` throws only when it cannot even send the observer: a console
    // without `host.iii.observe`, or a malformed source. Nothing will fire, so
    // the step says so and offers to close by hand.
    offHandler()
    fail(`${condition.function_id} cannot be observed: ${cause instanceof Error ? cause.message : String(cause)}`)
    return () => {}
  }
}

/**
 * Whether a fired condition closes its step. A `harness::on-turn-completed`
 * counts only in the tour's own chat: an example built from setup, or any
 * other chat, finishing a turn says nothing about it. Every other condition
 * closes its step when it fires.
 */
export async function firesStep(
  condition: Condition,
  fired: Fired,
  tourChat: () => Promise<string | null>,
): Promise<boolean> {
  if (condition.function_id !== 'harness::on-turn-completed') return true
  const session = (fired.payload as { session_id?: unknown } | null)?.session_id
  const id = await tourChat()
  return Boolean(id) && session === id
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
 * trace to read: a turn that ended while nothing was observing is gone, and
 * the step offers to close by hand when its observer fails.
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
