import type { JsonValue } from '@iii-dev/console-ui'

/** Opens the page at a bucket/prefix/object — the palette source, or any
    other worker's "inspect this object" affordance. */
export interface StoragePanelContext {
  bucket: string
  prefix?: string
  objectKey?: string
}

function asRecord(value: JsonValue): Record<string, JsonValue> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value
    : null
}

export function parseStoragePanelContext(
  value: JsonValue,
): StoragePanelContext | null {
  const record = asRecord(value)
  if (!record || typeof record.bucket !== 'string' || record.bucket === '')
    return null
  return {
    bucket: record.bucket,
    prefix: typeof record.prefix === 'string' ? record.prefix : undefined,
    objectKey:
      typeof record.objectKey === 'string' ? record.objectKey : undefined,
  }
}
