import type { JsonValue } from '@iii-dev/console-ui'
import { isObject, type JsonObject } from './value'

export type StructuredValueKind = 'object' | 'list' | 'string' | 'number' | 'boolean' | 'null'

export function structuredValueKind(value: JsonValue): StructuredValueKind {
  if (value === null) return 'null'
  if (Array.isArray(value)) return 'list'
  if (isObject(value)) return 'object'
  return typeof value as 'string' | 'number' | 'boolean'
}

export function emptyStructuredValue(kind: StructuredValueKind): JsonValue {
  if (kind === 'object') return {}
  if (kind === 'list') return []
  if (kind === 'string') return ''
  if (kind === 'number') return 0
  if (kind === 'boolean') return false
  return null
}

export function renameStructuredKey(value: JsonObject, from: string, to: string): JsonObject {
  if (to !== from && Object.hasOwn(value, to)) return value
  const next: JsonObject = {}
  for (const [key, entry] of Object.entries(value)) {
    next[key === from ? to : key] = entry
  }
  return next
}
