/**
 * Fan-out for the `aspire-dashboard::changed` trigger type: the bindings that
 * Console pages register, plus the deduplication that keeps overlapping
 * lifecycle transitions from waking every page twice.
 */

export type ChangedReason = 'dashboard' | 'observability'

export type ChangedBinding = { function_id: string; namespace?: string }

export type ChangedEvent = { reason: ChangedReason; dashboard: unknown }

export type ChangedFeed = {
  bind(id: string, binding: ChangedBinding): void
  unbind(id: string): void
  emit(reason: ChangedReason): void
}

/**
 * `snapshot` supplies the dashboard state carried on every event. `send`
 * delivers one event to one binding.
 *
 * `dashboard` events are deduplicated against the last one sent, because the
 * lifecycle transitions overlap: a failed start marks `failed` from both the
 * child's exit handler and the readiness check, and a stop that follows an
 * exit changes nothing. `observability` events describe state this snapshot
 * does not cover, so they always send.
 */
export function createChangedFeed(
  snapshot: () => unknown,
  send: (binding: ChangedBinding, event: ChangedEvent) => void,
): ChangedFeed {
  const bindings = new Map<string, ChangedBinding>()
  let lastDashboard: string | null = null

  return {
    bind(id, binding) {
      bindings.set(id, binding)
    },
    unbind(id) {
      bindings.delete(id)
    },
    emit(reason) {
      const dashboard = snapshot()
      if (reason === 'dashboard') {
        const key = JSON.stringify(dashboard)
        if (key === lastDashboard) return
        lastDashboard = key
      }
      for (const binding of bindings.values()) send(binding, { reason, dashboard })
    },
  }
}
