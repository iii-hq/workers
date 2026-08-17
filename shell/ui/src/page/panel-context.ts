import type { JsonValue } from '@iii-dev/console-ui'

export type ShellPanelContext =
  | {
      type: 'change-diff'
      changeId: string
      path: string
      canViewFile: boolean
    }
  | { type: 'file'; path: string }

function asRecord(value: JsonValue): Record<string, JsonValue> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value
    : null
}

export function parseShellPanelContext(
  value: JsonValue,
): ShellPanelContext | null {
  const record = asRecord(value)
  if (!record || typeof record.path !== 'string' || record.path === '') {
    return null
  }
  if (record.type === 'file') return { type: 'file', path: record.path }
  if (
    record.type === 'change-diff' &&
    typeof record.changeId === 'string' &&
    record.changeId !== ''
  ) {
    return {
      type: 'change-diff',
      changeId: record.changeId,
      path: record.path,
      canViewFile: record.canViewFile === true,
    }
  }
  return null
}
