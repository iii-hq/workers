/**
 * Put the bytes back behind a composer chip restored from a parked draft.
 *
 * `SessionMeta.draft_attachments` gives a chip its name, type and size, which
 * is enough to draw it the moment the conversation opens — but the send path
 * expands attachments from their `File` (a PDF goes through its worker, a
 * picture becomes an image block), and an image chip shows its thumbnail from
 * `dataUrl`. Both come from the bytes, so they are fetched from the store in
 * the background and folded into the chip; until then it behaves like a chip
 * reloaded from history, and a send still ships its `file` reference.
 */

import { getAttachment } from '@/lib/sessions/api'
import type { Attachment } from '@/types/chat'
import { readPreview } from './from-files'
import { base64ToBlob, type GetAttachmentResponse } from './store'

/** Chips that need bytes: restored (stored) ones nothing has hydrated yet. */
export function needsHydration(attachment: Attachment): boolean {
  return Boolean(attachment.attachmentId) && !attachment.file
}

export type FetchAttachmentFn = (input: {
  session_id: string
  attachment_id: string
  include_data?: boolean
}) => Promise<GetAttachmentResponse | null>

/**
 * Fetch the bytes of every chip that lacks them and rebuild the fields a
 * fresh pick would have had: the `File` (same name and type, so the expanders
 * classify it identically) and the preview `dataUrl` (images and small text,
 * exactly as `attachmentsFromFiles` decides). One at a time — they share a
 * worker and a tab's memory. Never throws: a chip whose bytes cannot be
 * fetched is simply left as it was (its `file` reference still sends) and the
 * reason is logged in DEV. Resolves with the patches by chip id.
 */
export async function hydrateDraftAttachments(
  sessionId: string,
  attachments: Attachment[],
  fetchAttachment: FetchAttachmentFn = getAttachment,
): Promise<Map<string, Pick<Attachment, 'file' | 'dataUrl'>>> {
  const patches = new Map<string, Pick<Attachment, 'file' | 'dataUrl'>>()
  for (const attachment of attachments) {
    if (!needsHydration(attachment)) continue
    try {
      const response = await fetchAttachment({
        session_id: sessionId,
        attachment_id: attachment.attachmentId as string,
        include_data: true,
      })
      if (!response?.data) {
        throw new Error('the store returned no bytes')
      }
      const mime = response.attachment?.mime || attachment.type
      const blob = base64ToBlob(response.data, mime)
      const file = new File([blob], attachment.name, { type: mime })
      const dataUrl = await readPreview(file)
      patches.set(attachment.id, dataUrl ? { file, dataUrl } : { file })
    } catch (err) {
      if (import.meta.env.DEV) {
        console.warn(
          `[attachments] could not hydrate draft attachment ${attachment.name}`,
          err,
        )
      }
    }
  }
  return patches
}
