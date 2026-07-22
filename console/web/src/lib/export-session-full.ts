/**
 * "With sub-agents" export: bundles the open session plus every descendant
 * sub-agent session (linked via `metadata.parent_session_id`) into one
 * markdown file. Best-effort throughout — discovery or per-transcript
 * failures degrade to `_(unavailable)_` markers, never a failed download.
 */

import { getIiiClient } from '@/lib/iii-client'
import { fetchTranscript } from '@/lib/sessions/api'
import { transcriptToMessages } from '@/lib/sessions/entry-mapper'
import type { SessionMeta } from '@/lib/sessions/types'
import type { Conversation, Message } from '@/types/chat'
import {
  buildExportFilename,
  conversationToMarkdown,
  fetchWorkerVersions,
  formatTimestamp,
  messagesToMarkdown,
  triggerMarkdownDownload,
} from './export-session'

const SESSIONS_LIST_RPC = 'session::list'
const SESSIONS_PAGE_LIMIT = 200
const SESSIONS_MAX_PAGES = 10

export interface DescendantSession {
  meta: SessionMeta
  depth: number
}

/**
 * All sessions, paged `updated_desc` up to `SESSIONS_MAX_PAGES`; `null` on
 * any failure. Descendants older than the page window are silently missed
 * (accepted in the spec's Known limits).
 */
export async function listAllSessions(): Promise<SessionMeta[] | null> {
  try {
    const client = await getIiiClient()
    const sessions: SessionMeta[] = []
    let cursor: string | undefined
    for (let page = 0; page < SESSIONS_MAX_PAGES; page++) {
      const resp = await client.trigger<{
        sessions?: SessionMeta[]
        next_cursor?: string | null
      }>(SESSIONS_LIST_RPC, {
        limit: SESSIONS_PAGE_LIMIT,
        order: 'updated_desc',
        ...(cursor ? { cursor } : {}),
      })
      const batch = resp?.sessions
      if (!Array.isArray(batch)) return null
      sessions.push(...batch)
      const next = resp?.next_cursor
      if (!next || batch.length === 0) break
      cursor = next
    }
    return sessions
  } catch {
    return null
  }
}

/**
 * Descendants of `rootId`, depth-first, siblings by `created_at` ascending;
 * root's direct children have depth 1. Cycle-safe via a visited set.
 */
export function collectDescendants(
  sessions: SessionMeta[],
  rootId: string,
): DescendantSession[] {
  const children = new Map<string, SessionMeta[]>()
  for (const s of sessions) {
    const parent = s.metadata?.parent_session_id
    if (typeof parent !== 'string' || parent === '') continue
    const list = children.get(parent)
    if (list) list.push(s)
    else children.set(parent, [s])
  }
  for (const list of children.values()) {
    list.sort((a, b) => a.created_at - b.created_at)
  }
  const out: DescendantSession[] = []
  const visited = new Set<string>([rootId])
  const walk = (id: string, depth: number) => {
    for (const child of children.get(id) ?? []) {
      if (visited.has(child.session_id)) continue
      visited.add(child.session_id)
      out.push({ meta: child, depth })
      walk(child.session_id, depth + 1)
    }
  }
  walk(rootId, 1)
  return out
}

/** One `# Sub-agent:` section; `null` messages = transcript fetch failed. */
export function subagentSectionToMarkdown(
  sub: DescendantSession,
  messages: Message[] | null,
): string {
  const { meta } = sub
  const parent = meta.metadata?.parent_session_id
  const header: string[] = [
    `# Sub-agent: ${meta.title || meta.session_id}`,
    '',
    `- ID: \`${meta.session_id}\``,
    `- Parent: \`${typeof parent === 'string' ? parent : 'unknown'}\``,
    `- Depth: ${sub.depth}`,
    `- Created: ${formatTimestamp(meta.created_at)}`,
    `- Updated: ${formatTimestamp(meta.updated_at)}`,
  ]
  if (messages === null) {
    return [...header, '', '---', '', '_(transcript unavailable)_'].join('\n')
  }
  header.push(`- Message count: ${messages.length}`)
  const body =
    messages.length === 0 ? '_(no messages)_' : messagesToMarkdown(messages)
  return [...header, '', '---', '', body].join('\n')
}

/**
 * Everything but the DOM: fetch versions + descendants, render one markdown
 * document. Separated from the download trigger so tests can cover the
 * assembly without a browser click.
 */
export async function assembleFullExport(
  conversation: Conversation,
): Promise<{ markdown: string; filename: string }> {
  const workers = await fetchWorkerVersions()
  const filename = buildExportFilename(conversation, 'full')
  const sessions = await listAllSessions()
  if (sessions === null) {
    return {
      markdown: conversationToMarkdown(conversation, workers, null),
      filename,
    }
  }
  const descendants = collectDescendants(sessions, conversation.id)
  const sections: string[] = []
  for (const sub of descendants) {
    let messages: Message[] | null = null
    try {
      const items = await fetchTranscript(sub.meta.session_id)
      messages = transcriptToMessages(items, sub.meta.session_id)
    } catch {
      messages = null
    }
    sections.push(subagentSectionToMarkdown(sub, messages))
  }
  const main = conversationToMarkdown(conversation, workers, descendants.length)
  const markdown = `${[main.trimEnd(), ...sections.map((s) => s.trimEnd())].join('\n\n')}\n`
  return { markdown, filename }
}

/**
 * Dropdown's "with sub-agents" action. Returns the filename so callers can
 * announce it, mirroring `downloadConversationAsMarkdown`.
 */
export async function downloadConversationWithSubagents(
  conversation: Conversation,
): Promise<string> {
  const { markdown, filename } = await assembleFullExport(conversation)
  triggerMarkdownDownload(markdown, filename)
  return filename
}
