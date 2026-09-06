/**
 * The worker's configuration as the page reads and writes it: one
 * `configuration::get` / `configuration::set` round trip on the `voice`
 * entry, with typed accessors over the nested JSON so page controls and the
 * Settings form edit the same fields.
 */

import type { ExtensionIii, JsonValue } from '@iii-dev/console-ui'

export const CONFIG_ID = 'voice'

export const NONE = '__none__'

export type JsonObject = { [key: string]: JsonValue }

export function asObject(value: JsonValue | undefined | null): JsonObject {
  return value && typeof value === 'object' && !Array.isArray(value) ? { ...value } : {}
}

export function getPath(value: JsonValue | undefined, path: readonly string[]): JsonValue | undefined {
  let cursor: JsonValue | undefined = value
  for (const key of path) {
    if (!cursor || typeof cursor !== 'object' || Array.isArray(cursor)) return undefined
    cursor = (cursor as JsonObject)[key]
  }
  return cursor
}

/** A copy of `value` with `path` set to `next` (`undefined` removes the key). */
export function setPath(
  value: JsonValue | undefined,
  path: readonly string[],
  next: JsonValue | undefined,
): JsonObject {
  const root = asObject(value)
  if (path.length === 0) return root
  let cursor = root
  for (const key of path.slice(0, -1)) {
    const child = asObject(cursor[key])
    cursor[key] = child
    cursor = child
  }
  const last = path[path.length - 1]
  if (next === undefined) delete cursor[last]
  else cursor[last] = next
  return root
}

export function stringAt(value: JsonValue | undefined, path: readonly string[], fallback = ''): string {
  const found = getPath(value, path)
  return typeof found === 'string' ? found : fallback
}

export function numberAt(value: JsonValue | undefined, path: readonly string[], fallback: number): number {
  const found = getPath(value, path)
  return typeof found === 'number' && Number.isFinite(found) ? found : fallback
}

export async function readConfig(iii: ExtensionIii): Promise<JsonObject> {
  const res = await iii.trigger<{ value?: JsonValue }>('configuration::get', { id: CONFIG_ID })
  return asObject(res?.value)
}

/** Read, apply `patch`, write. The worker hot-reloads on the change. */
export async function patchConfig(iii: ExtensionIii, patch: (current: JsonObject) => JsonObject): Promise<JsonObject> {
  const current = await readConfig(iii)
  const next = patch(current)
  await iii.trigger('configuration::set', { id: CONFIG_ID, value: next })
  return next
}

export const DEFAULTS = {
  sttBackend: 'local',
  model: 'zipformer-en-20m',
  finalModel: 'parakeet-tdt-0.6b-v2',
  numThreads: 2,
  silenceAfterSpeech: 0.8,
  silenceWithoutSpeech: 2.4,
  maxUtterance: 20,
  openaiBaseUrl: 'https://api.openai.com/v1',
  openaiSttModel: 'whisper-1',
  ttsBackend: 'host',
  maxSpeakChars: 4000,
  openaiTtsModel: 'tts-1',
  openaiTtsVoice: 'alloy',
  modelsDir: 'data/voice/models',
  maxAudioBytes: 10 * 1024 * 1024,
  maxSessions: 8,
  sessionIdleSecs: 120,
} as const
