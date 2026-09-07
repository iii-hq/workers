/**
 * Typed `session::*` calls against the session-manager worker, via the
 * shared iii-browser-sdk client. Reads paginate (`session::messages`
 * defaults to 50 rows, hard cap 500/page) so callers always see the full
 * active path.
 */

import type {
  AttachmentMeta,
  GetAttachmentResponse,
  PutAttachmentResponse,
} from '@/lib/attachments/store'
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

/** `session::set-draft`'s answer: what is parked after the write. */
export interface SetDraftResponse {
  draft: string | null
  attachments: AttachmentMeta[]
}

/**
 * Park (or clear, with `null`/empty text) the session's unsent composer
 * input. Event-silent and `updated_at`-neutral server-side, so it is safe
 * at keystroke cadence; reads back as `SessionMeta.draft`.
 *
 * `attachmentIds` is the parked attachment list (`SessionMeta.draft_attachments`),
 * in chip order. It is sent only when given: a text-only keystroke save
 * leaves the server's list alone, `[]` clears it (the post-send clear), and
 * an id the store does not know is refused with `session/invalid_request`.
 */
export async function setSessionDraft(
  sessionId: string,
  draft: string | null,
  attachmentIds?: string[],
): Promise<SetDraftResponse> {
  const client = await getIiiClient()
  return client.trigger<SetDraftResponse>('session::set-draft', {
    session_id: sessionId,
    ...(draft ? { draft } : {}),
    ...(attachmentIds !== undefined ? { attachment_ids: attachmentIds } : {}),
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
 * Store an attachment's original bytes against a session. `data` is standard
 * (padded) base64 of the raw file. The answer carries the `file` content
 * block to put on the outgoing message, so a caller never assembles one by
 * hand. Rejects with `session/not_found`, `session/attachment_too_large` or
 * `session/invalid_request` ahead of the colon in the error message.
 */
export async function putAttachment(input: {
  session_id: string
  name: string
  mime: string
  data: string
}): Promise<PutAttachmentResponse> {
  const client = await getIiiClient()
  return client.trigger('session::put-attachment', input)
}

/**
 * Read a stored attachment back. `null` when the session or the attachment
 * is unknown. `include_data: false` answers with the metadata alone, for a
 * caller that only needs to know the bytes are still there.
 */
export async function getAttachment(input: {
  session_id: string
  attachment_id: string
  include_data?: boolean
}): Promise<GetAttachmentResponse | null> {
  const client = await getIiiClient()
  const resp = await client.trigger<GetAttachmentResponse | null>(
    'session::get-attachment',
    input,
  )
  return resp ?? null
}

/**
 * Drop a stored attachment nothing sent references any more — a chip the
 * user removed from the composer after it was parked with the draft. The
 * store refuses (`session/attachment_in_use`) when a sent message still
 * points at it, which a draft chip never does.
 */
export async function deleteAttachment(input: {
  session_id: string
  attachment_id: string
}): Promise<{ deleted: boolean }> {
  const client = await getIiiClient()
  return client.trigger('session::delete-attachment', input)
}

/**
 * Full active path (oldest first), custom entries interleaved at their path
 * position, looping `cursor` until exhausted.
 *
 * Image bytes are left out unless `includeImageData` says otherwise. Every
 * picture ever sent in a session rides inline on its user message, and
 * opening a long conversation used to pull all of them at once, most for
 * messages far above the fold. Blocks that name a stored original come back
 * with `data: ""` and the chip fetches the picture when it scrolls into view;
 * blocks without one (sent before the store existed) keep their bytes, since
 * there is nowhere else to get them. A session-manager that predates the
 * option ignores it and answers with everything, which renders as before.
 * Readers that hand the transcript to a model or write it to disk need the
 * bytes and ask for them.
 */
export async function fetchTranscript(
  sessionId: string,
  opts?: { timeoutMs?: number; includeImageData?: boolean },
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
        // Omitted, not `true`, when the bytes are wanted: that is the
        // server's default, and the payload stays what older workers expect.
        ...(opts?.includeImageData ? {} : { include_image_data: false }),
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
