import type { JsonValue } from '@iii-dev/console-ui'
import type { ConfigPath } from './types'

export type JsonObject = { [key: string]: JsonValue }

export function isObject(value: JsonValue | undefined): value is JsonObject {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

export function asObject(value: JsonValue | undefined): JsonObject {
  return isObject(value) ? value : {}
}

export function atPath(root: JsonValue | undefined, path: ConfigPath): JsonValue | undefined {
  let current = root
  for (const part of path) {
    if (Array.isArray(current)) {
      const index = Number(part)
      if (!Number.isInteger(index)) return undefined
      current = current[index]
    } else if (isObject(current)) {
      current = current[part]
    } else {
      return undefined
    }
  }
  return current
}

export function setAtPath(root: JsonValue, path: ConfigPath, value: JsonValue): JsonValue {
  if (path.length === 0) return value
  const [head, ...tail] = path

  if (Array.isArray(root)) {
    const index = Number(head)
    const next = [...root]
    next[index] = setAtPath(next[index] ?? {}, tail, value)
    return next
  }

  const object = { ...asObject(root) }
  object[head] = setAtPath(object[head] ?? {}, tail, value)
  return object
}

export function deleteAtPath(root: JsonValue, path: ConfigPath): JsonValue {
  if (path.length === 0) return root
  const [head, ...tail] = path

  if (Array.isArray(root)) {
    const index = Number(head)
    const next = [...root]
    if (tail.length === 0) next.splice(index, 1)
    else next[index] = deleteAtPath(next[index] ?? {}, tail)
    return next
  }

  const object = { ...asObject(root) }
  if (tail.length === 0) delete object[head]
  else if (object[head] !== undefined) {
    object[head] = deleteAtPath(object[head], tail)
  }
  return object
}

/**
 * Select a declared variant without silently discarding configuration written
 * by a newer console or by an adapter whose contract this console does not
 * know yet. Declared fields from a known object variant are replaced by the
 * next defaults; opaque values and unknown variant objects are preserved.
 */
export function selectVariantValue(
  current: JsonObject,
  discriminator: string,
  contentKey: string,
  nextDiscriminator: string,
  nextDefaults: JsonObject,
  declaredPaths: readonly ConfigPath[],
  currentVariantKnown: boolean,
): JsonObject {
  const hasContent = Object.hasOwn(current, contentKey)
  const content = current[contentKey]

  if (hasContent && (!currentVariantKnown || !isObject(content))) {
    return {
      ...current,
      ...nextDefaults,
      [discriminator]: nextDiscriminator,
      [contentKey]: isObject(content)
        ? { ...asObject(nextDefaults[contentKey]), ...content }
        : content,
    }
  }

  let retained: JsonValue = current
  for (const declaredPath of declaredPaths) {
    retained = deleteAtPath(retained, declaredPath)
  }
  const retainedObject = asObject(retained)
  return {
    ...retainedObject,
    ...nextDefaults,
    [discriminator]: nextDiscriminator,
    [contentKey]: {
      ...asObject(retainedObject[contentKey]),
      ...asObject(nextDefaults[contentKey]),
    },
  }
}

export function joinPath(base: ConfigPath, path: ConfigPath): string[] {
  return [...base, ...path]
}

export function pointerFor(path: ConfigPath): string {
  return `/${path.map((part) => part.replaceAll('~', '~0').replaceAll('/', '~1')).join('/')}`
}

export function cloneJson<T extends JsonValue>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

export function displayValue(value: JsonValue | undefined): string {
  if (typeof value === 'string') return value
  if (typeof value === 'number' || typeof value === 'boolean') {
    return String(value)
  }
  return ''
}

const ENVIRONMENT_VALUE_PATTERN = /^\$\{([^}:]+)(?::([^}]*))?\}$/
const NUMBER_PATTERN = /^[-+]?(\d+\.?\d*|\.\d+)([eE][-+]?\d+)?$/

/** A complete `${VAR}` / `${VAR:default}` value understood by the engine. */
export function isEnvironmentValue(value: string): boolean {
  return ENVIRONMENT_VALUE_PATTERN.test(value)
}

export function environmentValueDefault(value: string): string | undefined {
  const match = ENVIRONMENT_VALUE_PATTERN.exec(value)
  return match?.[2]
}

/**
 * Typed configuration fields may legitimately carry a raw string while the
 * configuration worker is waiting to expand an environment value. Keep every
 * such string visible; validation remains responsible for malformed values.
 */
export function isRawTypedValue(value: JsonValue | undefined): value is string {
  return typeof value === 'string'
}

export function numberLiteralForRawValue(value: string, fallback: number): number {
  const candidate = environmentValueDefault(value)
  if (candidate !== undefined && NUMBER_PATTERN.test(candidate)) {
    const parsed = Number(candidate)
    if (Number.isFinite(parsed)) return parsed
  }
  return fallback
}

export function booleanLiteralForRawValue(value: string, fallback: boolean): boolean {
  const candidate = environmentValueDefault(value)
  if (candidate === 'true' || candidate === 'True' || candidate === 'TRUE') {
    return true
  }
  if (candidate === 'false' || candidate === 'False' || candidate === 'FALSE') {
    return false
  }
  return fallback
}

export function selectLiteralForRawValue(value: string, options: readonly string[], fallback: string): string {
  const candidate = environmentValueDefault(value)
  return candidate !== undefined && options.includes(candidate) ? candidate : fallback
}

/**
 * Return the object a purpose-built form should display. Legacy worker
 * parsers prioritize their named envelope over flat siblings, so the UI must
 * read from that same source until it has migrated the draft.
 */
export function legacyConfigurationValue(root: JsonValue, wrapper: string): JsonObject {
  const object = asObject(root)
  if (!Object.hasOwn(object, wrapper)) return object
  return asObject(object[wrapper])
}

/**
 * Atomically migrate an edited legacy envelope to the flat configuration
 * shape. Known flat fields are cleared before the inner object is applied so
 * stale siblings from older, envelope-unaware editors cannot win after the
 * wrapper is removed. Unknown root and inner fields are preserved.
 */
export function migrateLegacyConfiguration(
  root: JsonValue,
  wrapper: string,
  nextValue: JsonValue,
  ownedTopLevelFields: readonly string[],
): JsonValue {
  const object = asObject(root)
  if (!Object.hasOwn(object, wrapper)) return nextValue

  const migrated: JsonObject = { ...object }
  delete migrated[wrapper]
  for (const field of ownedTopLevelFields) delete migrated[field]
  return { ...migrated, ...asObject(nextValue) }
}
