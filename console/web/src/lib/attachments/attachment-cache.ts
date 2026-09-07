/**
 * One copy of each stored attachment's bytes per tab, fetched at most once.
 *
 * A transcript read leaves image bytes out (`include_image_data: false`, see
 * `fetchTranscript`), and every chip that needs a thumbnail asks for its
 * original when it scrolls into view. Left to the chips alone that is one
 * round trip per render: React remounts a chip when the message list
 * virtualises past it and back, when the user switches sessions and returns,
 * and when a snapshot replaces the message it sits on. The same picture would
 * cross the wire again every time. The download arrow on the same chip used
 * to fetch its own copy too.
 *
 * So the fetch lives here, keyed by session and attachment, and the entry
 * holds the in-flight promise until it settles: two chips asking at once
 * share one request, and a settled entry answers without a request at all.
 * A failure is forgotten, so the next ask (the chip's retry) really retries.
 *
 * The cache is unbounded on purpose. It holds only what the user has already
 * scrolled past in this tab, which is what a browser's image cache would hold
 * for a page of `<img>` tags; the elision above is what stops it from holding
 * every picture in every session.
 */

import { getAttachment } from '@/lib/sessions/api'
import type { TriggerFn } from './shared'
import {
  type AttachmentMeta,
  GET_ATTACHMENT_FUNCTION_ID,
  type GetAttachmentResponse,
} from './store'

/** A stored attachment with its bytes, as the chips consume it. */
export interface CachedAttachment {
  attachment: AttachmentMeta
  /** Standard base64 of the original, as the store answers. */
  data: string
  /** `data:<mime>;base64,<data>`, built once so re-renders do not rebuild it. */
  dataUrl: string
}

interface CacheEntry {
  promise: Promise<CachedAttachment>
  /** Set once the promise resolved, for the synchronous read. */
  value?: CachedAttachment
}

const entries = new Map<string, CacheEntry>()

export function attachmentCacheKey(
  sessionId: string,
  attachmentId: string,
): string {
  return `${sessionId}/${attachmentId}`
}

/**
 * The resolved bytes, without asking for them. A chip mounting for the second
 * time reads this in its initial state and draws the thumbnail on the first
 * paint, with no loading placeholder and no effect round trip.
 */
export function peekAttachment(
  sessionId: string,
  attachmentId: string,
): CachedAttachment | undefined {
  return entries.get(attachmentCacheKey(sessionId, attachmentId))?.value
}

/**
 * The bytes, fetched on the first ask and shared by every ask after it.
 *
 * Rejects when the store no longer has the attachment or the call failed;
 * the entry is dropped first so a later call fetches again rather than
 * replaying the failure forever. `trigger` is the test seam, as in
 * `uploadAttachments`.
 */
export function loadAttachment(
  sessionId: string,
  attachmentId: string,
  trigger?: TriggerFn,
): Promise<CachedAttachment> {
  const key = attachmentCacheKey(sessionId, attachmentId)
  const existing = entries.get(key)
  if (existing) return existing.promise

  const entry: CacheEntry = {
    promise: fetchStored(sessionId, attachmentId, trigger).then(
      (value) => {
        entry.value = value
        return value
      },
      (err: unknown) => {
        // Only this attempt's entry goes: a newer one (a retry that raced
        // this failure) must not be evicted by the failure it replaced.
        if (entries.get(key) === entry) entries.delete(key)
        throw err
      },
    ),
  }
  entries.set(key, entry)
  return entry.promise
}

async function fetchStored(
  sessionId: string,
  attachmentId: string,
  trigger?: TriggerFn,
): Promise<CachedAttachment> {
  const request = {
    session_id: sessionId,
    attachment_id: attachmentId,
    include_data: true,
  }
  const response = trigger
    ? ((await trigger(GET_ATTACHMENT_FUNCTION_ID, request)) as
        | GetAttachmentResponse
        | null
        | undefined)
    : await getAttachment(request)
  if (!response?.data) {
    throw new Error('the original is no longer in the session store')
  }
  const mime = response.attachment.mime || 'application/octet-stream'
  return {
    attachment: response.attachment,
    data: response.data,
    dataUrl: `data:${mime};base64,${response.data}`,
  }
}

/** Forget everything. For tests, which share the module between cases. */
export function clearAttachmentCache(): void {
  entries.clear()
}
