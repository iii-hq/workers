/**
 * Renders a `Conversation` as a markdown transcript and triggers a
 * browser download. Designed so the resulting `.md` file can be pasted
 * into another AI for analysis — role-prefixed messages, function triggers
 * as fenced JSON, attachments listed by metadata only (no base64 payload).
 */

import { getIiiClient } from '@/lib/iii-client'
import type {
  Attachment,
  Conversation,
  FunctionCallMessage,
  Message,
} from '@/types/chat'

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return `${bytes} B`
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function formatTimestamp(ms: number): string {
  try {
    return new Date(ms).toISOString()
  } catch {
    return String(ms)
  }
}

/** Best-effort pretty JSON; falls back to `String(value)` on circular refs / BigInt. */
function formatJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

function renderAttachments(attachments: readonly Attachment[]): string {
  // dataUrl is intentionally omitted — base64 payloads would balloon the
  // markdown and aren't useful for an AI doing transcript analysis.
  const items = attachments.map(
    (a) => `\`${a.name}\` (${a.type || 'unknown'}, ${formatBytes(a.size)})`,
  )
  return `**Attachments:** ${items.join(', ')}`
}

function renderMessage(message: Message): string {
  switch (message.role) {
    case 'user': {
      const parts: string[] = ['## User', message.content || '_(empty)_']
      if (message.attachments && message.attachments.length > 0) {
        parts.push('', renderAttachments(message.attachments))
      }
      return parts.join('\n')
    }
    case 'assistant': {
      const meta: string[] = []
      if (message.model) meta.push(message.model)
      if (message.mode) meta.push(message.mode)
      const header =
        meta.length > 0 ? `## Assistant (${meta.join(', ')})` : '## Assistant'
      return `${header}\n${message.content || '_(empty)_'}`
    }
    case 'thought': {
      return `## Thought\n${message.content || '_(empty)_'}`
    }
    case 'function-call': {
      return renderFunctionCall(message)
    }
    case 'system': {
      const tone = message.tone ? ` — ${message.tone}` : ''
      const kind = message.kind === 'compaction' ? ' (compaction)' : ''
      return `## System${tone}${kind}\n${message.content || '_(empty)_'}`
    }
  }
}

function renderFunctionCall(message: FunctionCallMessage): string {
  const status = message.pendingApproval
    ? ' (pending approval)'
    : message.running
      ? ' (triggering)'
      : ''
  const parts: string[] = [
    `## Trigger — ${message.functionId}${status}`,
    '**Input:**',
    '```json',
    formatJson(message.input),
    '```',
  ]
  if (message.output !== undefined) {
    parts.push('**Output:**', '```json', formatJson(message.output), '```')
  }
  return parts.join('\n')
}

/** One connected worker as reported by `engine::workers::list`. */
export interface WorkerVersion {
  name: string
  version?: string
}

const WORKERS_LIST_RPC = 'engine::workers::list'
const WORKERS_LIST_TIMEOUT_MS = 2000

/**
 * Best-effort snapshot of connected workers for the export header.
 * Lenient by design: `null` on any failure (offline engine, timeout,
 * malformed payload) — the caller renders `_(unavailable)_` and the
 * export proceeds. Deliberately does not reuse the zod schema in
 * `components/chat/engine/parsers.ts` (lib must not import components).
 */
export async function fetchWorkerVersions(): Promise<WorkerVersion[] | null> {
  try {
    const client = await getIiiClient()
    const res = await client.trigger<unknown>(
      WORKERS_LIST_RPC,
      {},
      { timeoutMs: WORKERS_LIST_TIMEOUT_MS },
    )
    const workers = (res as { workers?: unknown } | null)?.workers
    if (!Array.isArray(workers)) return null
    const out: WorkerVersion[] = []
    for (const entry of workers) {
      if (typeof entry !== 'object' || entry === null) continue
      const rec = entry as { id?: unknown; name?: unknown; version?: unknown }
      const name =
        typeof rec.name === 'string' && rec.name !== ''
          ? rec.name
          : typeof rec.id === 'string' && rec.id !== ''
            ? rec.id
            : null
      if (name === null) continue
      out.push({
        name,
        version:
          typeof rec.version === 'string' && rec.version !== ''
            ? rec.version
            : undefined,
      })
    }
    return out
  } catch {
    return null
  }
}

/**
 * `- Workers:` metadata bullet. `null` means the fetch failed (best-effort
 * export still proceeds); `[]` means the engine answered with no workers.
 */
function renderWorkersBlock(workers: WorkerVersion[] | null): string[] {
  if (workers === null) return ['- Workers: _(unavailable)_']
  if (workers.length === 0) return ['- Workers: _(none connected)_']
  const lines = workers.map(
    (w) => `  - ${w.name}: ${w.version ?? '(no version)'}`,
  )
  return ['- Workers:', ...Array.from(new Set(lines)).sort()]
}

export function conversationToMarkdown(
  conversation: Conversation,
  workers?: WorkerVersion[] | null,
): string {
  const header: string[] = [
    `# Session: ${conversation.title}`,
    '',
    `- ID: \`${conversation.id}\``,
    `- Created: ${formatTimestamp(conversation.createdAt)}`,
    `- Updated: ${formatTimestamp(conversation.updatedAt)}`,
    `- Model: \`${conversation.model}\``,
    `- Mode: \`${conversation.mode}\``,
    `- Message count: ${conversation.messages.length}`,
    ...(workers !== undefined ? renderWorkersBlock(workers) : []),
    '',
    '---',
    '',
  ]

  if (conversation.messages.length === 0) {
    return `${header.join('\n')}_(no messages)_\n`
  }

  const body = conversation.messages.map(renderMessage).join('\n\n')
  return `${header.join('\n')}${body}\n`
}

function pad2(n: number): string {
  return n < 10 ? `0${n}` : String(n)
}

function timestampSlug(now: Date = new Date()): string {
  const y = now.getFullYear()
  const m = pad2(now.getMonth() + 1)
  const d = pad2(now.getDate())
  const hh = pad2(now.getHours())
  const mm = pad2(now.getMinutes())
  return `${y}${m}${d}-${hh}${mm}`
}

export function buildExportFilename(conversation: Conversation): string {
  const shortId = conversation.id.slice(0, 8) || 'session'
  return `iii-session-${shortId}-${timestampSlug()}.md`
}

/**
 * Triggers a browser download of the rendered markdown.
 * Returns the filename so callers can announce it (e.g. via a live region).
 */
export function downloadConversationAsMarkdown(
  conversation: Conversation,
): string {
  const markdown = conversationToMarkdown(conversation)
  const filename = buildExportFilename(conversation)
  const blob = new Blob([markdown], { type: 'text/markdown;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  a.rel = 'noopener'
  document.body.appendChild(a)
  a.click()
  document.body.removeChild(a)
  // Revoke on the next tick so Safari has time to honour the click.
  window.setTimeout(() => URL.revokeObjectURL(url), 0)
  return filename
}
