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
 * Whether `state::on-changed` is emitted (default `true`). The worker migrates
 * an entry stored by an earlier release to `events_enabled` when it boots,
 * before it serves this form.
 */
export function eventsEnabledFor(value: JsonObject): boolean {
  return value.events_enabled !== false
}

/** Set `events_enabled`. */
export function withEventsEnabled(value: JsonObject, enabled: boolean): JsonObject {
  return { ...value, events_enabled: enabled }
}
