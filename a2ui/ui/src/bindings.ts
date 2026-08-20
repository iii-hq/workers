import type { JsonValue } from './types'

export function getPath(model: JsonValue, path: string): JsonValue | undefined {
  if (path === '' || path === '/') return model
  const segments = pointerSegments(path)
  if (!segments) return undefined
  let cursor: JsonValue | undefined = model
  for (const segment of segments) {
    if (cursor == null || typeof cursor !== 'object') return undefined
    if (Array.isArray(cursor)) {
      const index = Number(segment)
      if (!Number.isInteger(index) || index < 0 || index >= cursor.length) return undefined
      cursor = cursor[index]
    } else {
      if (!Object.hasOwn(cursor, segment)) return undefined
      cursor = cursor[segment]
    }
  }
  return cursor
}

export function setPath(model: JsonValue, path: string, value: JsonValue): JsonValue {
  if (path === '' || path === '/') return value
  const segments = pointerSegments(path)
  if (!segments) return model
  return setSegments(model, segments, value)
}

const unsafeSegments = new Set(['__proto__', 'prototype', 'constructor'])

function pointerSegments(path: string): string[] | null {
  if (!path.startsWith('/')) return null
  const segments = path
    .slice(1)
    .split('/')
    .map((segment) => segment.replaceAll('~1', '/').replaceAll('~0', '~'))
  return segments.some((segment) => unsafeSegments.has(segment)) ? null : segments
}

function setSegments(current: JsonValue | undefined, segments: string[], value: JsonValue): JsonValue {
  if (segments.length === 0) return value
  const [segment, ...rest] = segments
  if (Array.isArray(current)) {
    const index = Number(segment)
    if (!Number.isInteger(index) || index < 0) return current
    const next = [...current]
    next[index] = setSegments(next[index], rest, value)
    return next
  }
  const source = current != null && typeof current === 'object' ? current : {}
  const next: Record<string, JsonValue> = { ...source }
  const child = Object.hasOwn(source, segment) ? source[segment] : undefined
  next[segment] = setSegments(child, rest, value)
  return next
}
