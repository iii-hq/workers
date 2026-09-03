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
