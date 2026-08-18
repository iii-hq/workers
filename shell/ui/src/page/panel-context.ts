import type { JsonValue } from '@iii-dev/console-ui'

export type ShellPanelContext =
  | {
      type: 'change-diff'
      changeId: string
      path: string
      canViewFile: boolean
    }
  | { type: 'file'; path: string }
  /** Open a terminal on the directory an agent worked in, ready for its CLI. */
  | { type: 'agent-terminal'; cwd: string; command: string }

function asRecord(value: JsonValue): Record<string, JsonValue> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? value : null
}

export function parseShellPanelContext(value: JsonValue): ShellPanelContext | null {
  const record = asRecord(value)
  if (!record) return null
  // An agent terminal is a directory, not a file: the pane it opens is a PTY
  // rooted there, and the CLI is what the user drives inside it.
  if (record.type === 'agent-terminal' && typeof record.cwd === 'string' && record.cwd !== '') {
    return {
      type: 'agent-terminal',
      cwd: record.cwd,
      command: typeof record.command === 'string' ? record.command : '',
    }
  }
  if (typeof record.path !== 'string' || record.path === '') {
    return null
  }
  if (record.type === 'file') return { type: 'file', path: record.path }
  if (record.type === 'change-diff' && typeof record.changeId === 'string' && record.changeId !== '') {
    return {
      type: 'change-diff',
      changeId: record.changeId,
      path: record.path,
      canViewFile: record.canViewFile === true,
    }
  }
  return null
}
