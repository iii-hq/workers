import type { FunctionTriggerMessage, Message } from '@/types/chat'

/**
 * What a reply's footer says about the turn behind it: how long it took,
 * when it landed, and which files the agent's calls wrote on the way.
 */

export interface ChangedFile {
  path: string
  name: string
  additions: number
  deletions: number
  kind: 'write' | 'delete'
}

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
}

/** A call's arguments, with the iii envelope peeled when one wraps them. */
function callInput(
  call: FunctionTriggerMessage,
): Record<string, unknown> | null {
  const raw = record(call.input)
  if (!raw) return null
  const details = record(raw.details)
  if (details && Array.isArray(raw.content)) return details
  return raw
}

function lineCount(text: unknown): number {
  if (typeof text !== 'string' || text.length === 0) return 0
  return text.split('\n').length
}

export function fileName(path: string): string {
  return path.split('/').filter(Boolean).pop() ?? path
}

interface Tally {
  additions: number
  deletions: number
  kind: ChangedFile['kind']
}

/**
 * Files touched by a reply's calls, merged by path, newest-first in order
 * of first appearance. Line counts come from the arguments the agent sent
 * (content inserted, ranges removed), so they are exact for the coder and
 * shell writers and conservative (1/1) for pattern replacements.
 */
export function changedFilesFromCalls(
  calls: readonly FunctionTriggerMessage[],
): ChangedFile[] {
  const byPath = new Map<string, Tally>()
  const bump = (
    path: unknown,
    additions: number,
    deletions: number,
    kind: ChangedFile['kind'] = 'write',
  ) => {
    if (typeof path !== 'string' || !path.trim()) return
    const current = byPath.get(path) ?? { additions: 0, deletions: 0, kind }
    current.additions += additions
    current.deletions += deletions
    if (kind === 'delete') current.kind = 'delete'
    byPath.set(path, current)
  }

  for (const call of calls) {
    if (call.pendingApproval) continue
    const input = callInput(call)
    if (!input) continue
    switch (call.functionId) {
      case 'coder::create-file': {
        const files = Array.isArray(input.files) ? input.files : []
        for (const file of files) {
          const spec = record(file)
          if (spec) bump(spec.path, lineCount(spec.content), 0)
        }
        break
      }
      case 'coder::update-file': {
        const files = Array.isArray(input.files) ? input.files : []
        for (const file of files) {
          const spec = record(file)
          if (!spec) continue
          let additions = 0
          let deletions = 0
          const ops = Array.isArray(spec.ops) ? spec.ops : []
          for (const raw of ops) {
            const op = record(raw)
            if (!op) continue
            const span =
              typeof op.from_line === 'number' && typeof op.to_line === 'number'
                ? Math.max(0, op.to_line - op.from_line + 1)
                : 0
            switch (op.op) {
              case 'insert':
                additions += lineCount(op.content)
                break
              case 'remove':
                deletions += span
                break
              case 'update_lines':
                additions += lineCount(op.content)
                deletions += span
                break
              default:
                additions += 1
                deletions += 1
            }
          }
          bump(spec.path, additions, deletions)
        }
        break
      }
      case 'coder::delete-file': {
        const paths = Array.isArray(input.paths) ? input.paths : []
        for (const path of paths) bump(path, 0, 0, 'delete')
        break
      }
      case 'shell::fs::write':
        bump(input.path, lineCount(input.content), 0)
        break
      case 'shell::fs::sed':
        bump(input.path, 1, 1)
        break
      case 'shell::fs::rm': {
        const paths = Array.isArray(input.paths)
          ? input.paths
          : typeof input.path === 'string'
            ? [input.path]
            : []
        for (const path of paths) bump(path, 0, 0, 'delete')
        break
      }
      default:
        break
    }
  }

  return [...byPath.entries()].map(([path, tally]) => ({
    path,
    name: fileName(path),
    ...tally,
  }))
}

/**
 * How long the turn took up to this reply: from the user message that
 * started it to the reply's own timestamp. Null when no user message
 * precedes it (a seeded or imported transcript) or the clock ran backwards.
 */
export function turnDurationMs(
  messages: readonly Message[],
  assistantId: string,
): number | null {
  const index = messages.findIndex((m) => m.id === assistantId)
  if (index < 0) return null
  const reply = messages[index]
  for (let i = index - 1; i >= 0; i--) {
    const m = messages[i]
    if (m.role === 'user') {
      const delta = reply.createdAt - m.createdAt
      return delta >= 0 ? delta : null
    }
  }
  return null
}

/** `1.2s`, `48s`, `23m 37s`, `1h 05m`. */
export function formatTurnDuration(ms: number): string {
  if (ms < 1000) return `${Math.max(1, Math.round(ms / 100)) / 10}s`
  const seconds = Math.round(ms / 1000)
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60)
    return `${minutes}m ${String(seconds % 60).padStart(2, '0')}s`
  const hours = Math.floor(minutes / 60)
  return `${hours}h ${String(minutes % 60).padStart(2, '0')}m`
}

/** `6:45 PM` today; `Oct 27, 6:45 PM` on another day. */
export function formatReplyClock(ts: number, now = Date.now()): string {
  const date = new Date(ts)
  const time = date.toLocaleTimeString(undefined, {
    hour: 'numeric',
    minute: '2-digit',
  })
  const today = new Date(now)
  const sameDay =
    date.getFullYear() === today.getFullYear() &&
    date.getMonth() === today.getMonth() &&
    date.getDate() === today.getDate()
  if (sameDay) return time
  const day = date.toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
  })
  return `${day}, ${time}`
}

/**
 * The assistant replies that open with the agent / model header: the first
 * reply after each user message, and any later reply in the same turn whose
 * agent or model differs from the one before it. A turn that streams prose,
 * calls tools, then writes more names its author once instead of above
 * every segment.
 */
export function assistantHeaderIds(
  messages: readonly Message[],
  agentName?: string,
): Set<string> {
  const ids = new Set<string>()
  let author: string | null = null
  for (const message of messages) {
    if (message.role === 'user') {
      author = null
      continue
    }
    if (message.role !== 'assistant') continue
    const next = `${message.agent || agentName || ''}\u0000${message.model ?? ''}`
    if (next !== author) ids.add(message.id)
    author = next
  }
  return ids
}
