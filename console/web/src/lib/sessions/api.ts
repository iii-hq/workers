/**
 * Typed `session::*` calls against the session-manager worker, via the
 * shared iii-browser-sdk client. Reads paginate (`session::messages`
 * defaults to 50 rows, hard cap 500/page) so callers always see the full
 * active path.
 */

import { getIiiClient } from '@/lib/iii-client'
import type { SessionMeta, SessionStatus, TranscriptItem } from './types'

/** `session::messages` hard cap per page. */
const MESSAGES_PAGE_LIMIT = 500
/** Sidebar page size (one page; `updated_desc` keeps recent chats first). */
const LIST_PAGE_LIMIT = 200

export async function listSessions(): Promise<SessionMeta[]> {
  const client = await getIiiClient()
  const resp = await client.trigger<{
    sessions?: SessionMeta[]
    next_cursor?: string | null
  }>('session::list', { limit: LIST_PAGE_LIMIT, order: 'updated_desc' })
  return resp?.sessions ?? []
}

export async function getSession(
  sessionId: string,
): Promise<SessionMeta | null> {
  const client = await getIiiClient()
  const resp = await client.trigger<{ meta: SessionMeta } | null>(
    'session::get',
    { session_id: sessionId },
  )
  return resp?.meta ?? null
}

/**
 * Idempotently materialise a caller-chosen session id. Creation applies
 * title/metadata; for an existing id this is a pure read.
 */
export async function ensureSession(input: {
  session_id: string
  title?: string
  metadata?: Record<string, unknown>
}): Promise<{ meta: SessionMeta; created: boolean }> {
  const client = await getIiiClient()
  return client.trigger('session::ensure', input)
}

/**
 * Update title/description/metadata. `metadata` replaces WHOLESALE — always
 * send the full object, never a delta.
 */
export async function setSessionMeta(input: {
  session_id: string
  title?: string
  description?: string
  metadata?: Record<string, unknown>
}): Promise<{ meta: SessionMeta }> {
  const client = await getIiiClient()
  return client.trigger('session::set-meta', input)
}

/**
 * Park (or clear, with `null`/empty text) the session's unsent composer
 * input. Event-silent and `updated_at`-neutral server-side, so it is safe
 * at keystroke cadence; reads back as `SessionMeta.draft`.
 */
export async function setSessionDraft(
  sessionId: string,
  draft: string | null,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('session::set-draft', {
    session_id: sessionId,
    ...(draft ? { draft } : {}),
  })
}

export async function deleteSession(
  sessionId: string,
): Promise<{ deleted: boolean }> {
  const client = await getIiiClient()
  return client.trigger('session::delete', { session_id: sessionId })
}

export async function setSessionStatus(
  sessionId: string,
  status: SessionStatus,
  reason?: string,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('session::set-status', {
    session_id: sessionId,
    status,
    ...(reason ? { reason } : {}),
  })
}

/**
 * Append a `kind: "custom"` bookkeeping entry (e.g. the `/compact` marker).
 * Idempotent on `entry_id`. Mirrors how the harness persists its own
 * compaction record, so the entry renders identically and future turns can
 * anchor on it. The `data` surfaces back as `custom.data` on reads/events.
 */
export async function appendCustomEntry(input: {
  session_id: string
  custom_type: string
  data: unknown
  entry_id?: string
}): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('session::append', {
    session_id: input.session_id,
    custom: { custom_type: input.custom_type, data: input.data },
    ...(input.entry_id ? { entry_id: input.entry_id } : {}),
  })
}

/**
 * Full active path (oldest first), custom entries interleaved at their path
 * position, looping `cursor` until exhausted.
 */
export async function fetchTranscript(
  sessionId: string,
  opts?: { timeoutMs?: number },
): Promise<TranscriptItem[]> {
  const client = await getIiiClient()
  const items: TranscriptItem[] = []
  let cursor: string | undefined
  for (;;) {
    const resp = await client.trigger<{
      messages?: TranscriptItem[]
      next_cursor?: string | null
    }>(
      'session::messages',
      {
        session_id: sessionId,
        limit: MESSAGES_PAGE_LIMIT,
        include_custom: true,
        ...(cursor ? { cursor } : {}),
      },
      opts?.timeoutMs ? { timeoutMs: opts.timeoutMs } : undefined,
    )
    const page = resp?.messages ?? []
    for (const item of page) {
      if (item && typeof item.entry_id === 'string') items.push(item)
    }
    const next = resp?.next_cursor
    if (!next || page.length === 0) return items
    cursor = next
  }
}
