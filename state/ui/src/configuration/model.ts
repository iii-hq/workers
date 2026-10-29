import type { JsonValue } from '@iii-dev/console-ui'

export type JsonObject = { [key: string]: JsonValue }
export type PersistenceMode = 'file_based' | 'in_memory'

export function persistenceModeFor(value: JsonValue | undefined): PersistenceMode {
  return value === 'in_memory' ? 'in_memory' : 'file_based'
}

export function withPersistenceMode(config: JsonObject, mode: PersistenceMode): JsonObject {
  const next: JsonObject = { ...config, store_method: mode }
  if (mode === 'in_memory') delete next.file_path
  return next
}

/**
 * Entries stored by earlier releases name the `events_enabled` knob
 * `triggers_enabled`; the worker reads either.
 */
const EARLIER_EVENTS_KEY = 'triggers_enabled'

/** Whether `state::on-changed` is emitted (default `true`). */
export function eventsEnabledFor(value: JsonObject): boolean {
  const enabled = 'events_enabled' in value ? value.events_enabled : value[EARLIER_EVENTS_KEY]
  return enabled !== false
}

/** Set `events_enabled`, writing the knob under its schema name only. */
export function withEventsEnabled(value: JsonObject, enabled: boolean): JsonObject {
  const next: JsonObject = { ...value, events_enabled: enabled }
  delete next[EARLIER_EVENTS_KEY]
  return next
}
