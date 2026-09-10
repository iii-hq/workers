import type { Host } from '@iii-dev/console-ui'

/** What a step's condition looked like when it fired. */
export interface Fired {
  trigger_type: string
  payload: unknown
  at: number
}

export interface Condition {
  type: string
  config: Record<string, unknown>
  label: string
  hint?: string
  prompt?: string
}

/**
 * Bind one step's condition to a browser-local handler, the same way the
 * console's own live pages do: a per-tab function id, then an engine trigger
 * pointed at it. Returns the unbind.
 *
 * The trigger is real — the tour is not simulating one — so what the page
 * shows the operator is the payload the engine delivered.
 */
export function bindCondition(
  host: Host,
  key: string,
  condition: Condition,
  onFire: (fired: Fired) => void,
): () => void {
  const localId = `onboarding::condition::${key}`
  try {
    const offHandler = host.iii.on(localId, (payload: unknown) => {
      onFire({ trigger_type: condition.type, payload, at: Date.now() })
    })
    const offTrigger = host.iii.registerTrigger({
      type: condition.type,
      function_id: `${localId}::${host.iii.browserId}`,
      config: condition.config,
    })
    return () => {
      offTrigger()
      offHandler()
    }
  } catch {
    // The trigger type's worker may be down or restarting. The step stays
    // waiting and rebinds on the next mount.
    return () => {}
  }
}
