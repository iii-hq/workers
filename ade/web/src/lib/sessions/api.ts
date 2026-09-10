/**
 * Typed `session::*` calls against the session-manager worker, via the
 * shared iii-browser-sdk client.
 *
 * Three transcript readers, for three needs. `fetchTranscript` loops
 * `session::messages` until the cursor runs out and is the FULL active path:
 * an export or a compaction hands the whole conversation on and needs every
 * byte. `fetchTranscriptTail` is one page of `session::messages-tail`, the
 * newest blocks first and then backwards from an anchor: what a chat opens
 * on and what scrolling up asks for, with the inside of long tool-call runs
 * left out (`elided`). `fetchTranscriptRange` is `session::messages-range`,
 * whole entries by id or span, and brings those elided parts back when a
 * reader expands a group.
 */

import type {
  AttachmentMeta,
  GetAttachmentResponse,
  PutAttachmentResponse,
} from '@/lib/attachments/store'
import { errText } from '@/lib/errors'
import { getIiiClient } from '@/lib/iii-client'
import type { SessionMeta, SessionStatus, TranscriptItem } from './types'

/** `session::messages` hard cap per page. */
const MESSAGES_PAGE_LIMIT = 500
/**
 * Blocks on the page a chat opens with. A block is a plain entry or one whole
 * activity run, so this is roughly the last fifteen conversational steps —
 * enough to fill a tall viewport, small enough that a 300-call session opens
 * as fast as a short one.
 */
export const TRANSCRIPT_TAIL_PAGE_LIMIT = 15
/** Blocks per page when scrolling up into earlier history. */
export const TRANSCRIPT_OLDER_PAGE_LIMIT = 25
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

/** One page of `session::messages-tail`, oldest first. */
export interface TranscriptTailPage {
  items: TranscriptItem[]
  /** Older blocks exist above `items[0]`. */
  hasMore: boolean
  /**
   * The first entry on the page — the `beforeEntryId` for the next page up.
   * Absent when the page is empty.
   */
  oldestEntryId?: string
}

/**
 * One page of the active path counted in BLOCKS (a plain entry, or one whole
 * activity run with its results and wake entries), newest first and then
 * backwards from `beforeEntryId` (exclusive — pass the oldest entry held).
 * A page never splits a run, so a call and its result always land together.
 *
 * `untilEntryId` widens the page backwards until that entry's block is on
 * it, for a deep link into history: the server decides how far back that is,
 * and the caller does not cap it. An anchor that is not on the active path
 * rejects with `session/invalid_cursor` — the leaf moved, reload from the
 * top.
 *
 * Inside a run, only the last function-calling assistant entry, the results
 * answering its calls, and wake entries arrive whole; every other entry is
 * `elided` (see `TranscriptItem`). Image bytes are always left out here, as
 * `fetchTranscript` does for the chat: the chips fetch them on view.
 */
export async function fetchTranscriptTail(
  sessionId: string,
  opts: {
    limit: number
    beforeEntryId?: string
    untilEntryId?: string
    timeoutMs?: number
  },
): Promise<TranscriptTailPage> {
  const client = await getIiiClient()
  const resp = await client.trigger<{
    messages?: TranscriptItem[]
    has_more?: boolean
    oldest_entry_id?: string | null
  }>(
    'session::messages-tail',
    {
      session_id: sessionId,
      limit: opts.limit,
      include_custom: true,
      include_image_data: false,
      ...(opts.beforeEntryId ? { before_entry_id: opts.beforeEntryId } : {}),
      ...(opts.untilEntryId ? { until_entry_id: opts.untilEntryId } : {}),
    },
    opts.timeoutMs ? { timeoutMs: opts.timeoutMs } : undefined,
  )
  const items: TranscriptItem[] = []
  for (const item of resp?.messages ?? []) {
    if (item && typeof item.entry_id === 'string') items.push(item)
  }
  return {
    items,
    hasMore: resp?.has_more === true,
    ...(resp?.oldest_entry_id ? { oldestEntryId: resp.oldest_entry_id } : {}),
  }
}

/** Exactly one of: a span of the active path, or a list of specific ids. */
export type TranscriptRangeSelector =
  | { fromEntryId: string; toEntryId: string }
  | { entryIds: string[] }

/**
 * Whole entries (never elided) for a span of the active path or a list of
 * ids, in path order, looping `next_cursor` like `fetchTranscript`. This is
 * how the placeholders a tail page left behind get their arguments and
 * results back: "show all" on a group names the entries it holds. Rejects
 * with `session/entry_not_found` for an id nobody wrote,
 * `session/invalid_cursor` for one on another branch, and
 * `session/invalid_request` for a reversed span.
 */
export async function fetchTranscriptRange(
  sessionId: string,
  selector: TranscriptRangeSelector,
  opts?: { timeoutMs?: number },
): Promise<TranscriptItem[]> {
  const client = await getIiiClient()
  const items: TranscriptItem[] = []
  const selection =
    'entryIds' in selector
      ? { entry_ids: selector.entryIds }
      : { from_entry_id: selector.fromEntryId, to_entry_id: selector.toEntryId }
  let cursor: string | undefined
  for (;;) {
    const resp = await client.trigger<{
      messages?: TranscriptItem[]
      next_cursor?: string | null
    }>(
      'session::messages-range',
      {
        session_id: sessionId,
        ...selection,
        include_image_data: false,
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

/**
 * The worker predates the function. The engine answers a call to an
 * unregistered id with `function_not_found`; older engines phrased it as
 * "not registered". Paging readers fall back to the full read on this, so a
 * console shipped ahead of its session-manager keeps working.
 */
export function isMissingFunctionError(err: unknown): boolean {
  return /function_not_found|function .*not (?:found|registered)|not registered/i.test(
    errText(err),
  )
}

/**
 * The paging anchor is no longer on the active path (`session/invalid_cursor`):
 * the leaf moved under us. The only recovery is to reload from the top.
 */
export function isInvalidCursorError(err: unknown): boolean {
  return /session\/invalid_cursor|session\/entry_not_found/.test(errText(err))
}
