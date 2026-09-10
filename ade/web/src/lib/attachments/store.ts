/**
 * The original bytes of every attachment, kept by session-manager.
 *
 * Everything else under `lib/attachments` consumes the picked `File` and
 * replaces it with something a model can read — extracted text, a downscaled
 * picture — and that derived content is all the transcript ever held. The
 * original was gone the moment the send finished: a reloaded conversation
 * showed a chip with a name and nothing behind it, an image chip did not even
 * keep its name, and there was no way to get the document back out.
 *
 * This module stores the bytes themselves, per session, BEFORE the send. The
 * store answers with a `file` content block — a reference, not the payload —
 * that rides on the user message next to the expansions. The harness strips
 * `file` blocks before the model sees them, so the text and image expansions
 * still go exactly as before; the reference is added, never substituted.
 *
 * Nothing here blocks a send. A store that is down or a file over its ceiling
 * becomes a warn notice, and the message goes out with the expansions alone,
 * as it did before this module existed.
 */

import { putAttachment } from '@/lib/sessions/api'
import type { Attachment } from '@/types/chat'
import {
  type AttachmentFailure,
  describeWorkerFailure,
  fileToBase64,
  type TriggerFn,
} from './shared'

export const PUT_ATTACHMENT_FUNCTION_ID = 'session::put-attachment'
export const GET_ATTACHMENT_FUNCTION_ID = 'session::get-attachment'

/** What session-manager records about one stored attachment. */
export interface AttachmentMeta {
  attachment_id: string
  session_id: string
  name: string
  mime: string
  size: number
  sha256: string
  created_at: number
}

/**
 * A reference to stored bytes, as a content block on a user message.
 *
 * Wire-identical to the harness's `ContentBlock::File`. It carries enough to
 * draw the chip without a round trip — name, type, size — and the id to fetch
 * the bytes when someone asks for them.
 */
export interface FileBlock {
  type: 'file'
  attachment_id: string
  name: string
  mime: string
  size: number
}

/** `session::put-attachment`'s answer. */
export interface PutAttachmentResponse {
  attachment: AttachmentMeta
  block: FileBlock
}

/** `session::get-attachment`'s answer; `data` is null when not requested. */
export interface GetAttachmentResponse {
  attachment: AttachmentMeta
  data: string | null
}

export interface UploadedAttachments {
  /** One `file` block per stored attachment, in input order. */
  blocks: FileBlock[]
  /** Chip id → server id, so the local message's chips can be patched. */
  uploaded: Array<{ id: string; attachmentId: string }>
  /** Everything that could not be stored, for the notices above the composer. */
  failures: AttachmentFailure[]
}

export const EMPTY_UPLOAD: UploadedAttachments = {
  blocks: [],
  uploaded: [],
  failures: [],
}

/**
 * `true` when a send has something for the store: bytes to upload, or a
 * reference to bytes already stored (a draft chip parked before the send, or
 * restored after a refresh). A conversation reloaded from history carries
 * chips with neither, and answers `false`.
 */
export function hasStorableAttachments(attachments: Attachment[]): boolean {
  return attachments.some((a) => a.file || a.attachmentId)
}

/**
 * The `file` block for a chip whose bytes the store already holds. Built from
 * the chip alone — name, type, size are what the chip was drawn from — so a
 * parked attachment rides on the message without a second round trip.
 */
export function fileBlockFromAttachment(
  attachment: Attachment & { attachmentId: string },
): FileBlock {
  return {
    type: 'file',
    attachment_id: attachment.attachmentId,
    name: attachment.name,
    mime: attachment.type || 'application/octet-stream',
    size: attachment.size,
  }
}

/**
 * The chips with the ids the store just gave them, for the expansion that
 * follows an upload. `expandAttachments` stamps a picture's inline image
 * block with `attachment.attachmentId`, and a chip fresh from the composer
 * has none until `uploadAttachments` answers — so the two run in that order
 * and this joins them. A chip the store refused keeps no id, and its image
 * block goes out as it always has: inline bytes, nothing to fetch later.
 */
export function linkStoredAttachments(
  attachments: Attachment[],
  uploaded: UploadedAttachments['uploaded'],
): Attachment[] {
  if (uploaded.length === 0) return attachments
  const byId = new Map(uploaded.map((u) => [u.id, u.attachmentId]))
  return attachments.map((a) => {
    const attachmentId = byId.get(a.id)
    return attachmentId && a.attachmentId !== attachmentId
      ? { ...a, attachmentId }
      : a
  })
}

/**
 * Store the bytes of every attachment on a message.
 *
 * Every kind goes, not just the ones the expanders read: the point is the
 * original, and a `.heic` no model decodes or a format the console cannot
 * read into a message is exactly the file worth keeping intact. Attachments
 * without their `File` are skipped — a conversation reloaded from history
 * carries chips, not bytes, and those were stored when they were sent.
 *
 * A chip that already carries an `attachmentId` was stored while the draft
 * was being composed (or restored from a parked draft): its block is
 * synthesized from the chip and the bytes are NOT sent again — uploading
 * twice would leave an orphan copy behind and double the transfer for
 * nothing.
 *
 * Uploads run one at a time. They all land on the same worker, and encoding
 * happens in this tab; two large documents in flight together would double
 * the memory held while nothing finished sooner. Never throws: a failed
 * upload is a failure entry, and the send goes on with the expansions.
 */
export async function uploadAttachments(
  sessionId: string,
  attachments: Attachment[],
  trigger?: TriggerFn,
): Promise<UploadedAttachments> {
  const storable = attachments.filter((a) => a.file || a.attachmentId)
  if (storable.length === 0) return EMPTY_UPLOAD

  const blocks: FileBlock[] = []
  const uploaded: Array<{ id: string; attachmentId: string }> = []
  const failures: AttachmentFailure[] = []

  for (const attachment of storable) {
    if (attachment.attachmentId) {
      blocks.push(
        fileBlockFromAttachment(
          attachment as Attachment & { attachmentId: string },
        ),
      )
      uploaded.push({
        id: attachment.id,
        attachmentId: attachment.attachmentId,
      })
      continue
    }
    try {
      const data = await fileToBase64(attachment.file as File)
      const request = {
        session_id: sessionId,
        name: attachment.name,
        // A file dragged out of an archive often arrives with no type at all;
        // the store requires one, and the generic binary type is the honest
        // answer rather than a guess from the extension.
        mime: attachment.type || 'application/octet-stream',
        data,
      }
      const response = trigger
        ? ((await trigger(PUT_ATTACHMENT_FUNCTION_ID, request)) as
            | PutAttachmentResponse
            | null
            | undefined)
        : await putAttachment(request)
      if (!response?.block?.attachment_id) {
        throw new Error('the store returned no attachment reference')
      }
      blocks.push(response.block)
      uploaded.push({
        id: attachment.id,
        attachmentId: response.block.attachment_id,
      })
    } catch (err) {
      failures.push({
        name: attachment.name,
        reason: describeStoreFailure(err),
      })
    }
  }

  return { blocks, uploaded, failures }
}

/**
 * The store's own error codes travel as `<code>: <detail>` in the message;
 * the two a person can act on get a sentence, the rest surface as they are.
 */
function describeStoreFailure(err: unknown): string {
  const message = describeWorkerFailure(err, 'session-manager')
  if (message.startsWith('session/attachment_too_large')) {
    return 'larger than the attachment store accepts'
  }
  if (message.startsWith('session/not_found')) {
    return 'the session does not exist yet'
  }
  return message
}

/**
 * The original bytes as a `Blob`, from the store's base64 payload. `atob`
 * yields a binary string; copying it byte by byte is what keeps the result
 * identical to what was uploaded, not a UTF-8 re-encoding of it.
 */
export function base64ToBlob(data: string, mime: string): Blob {
  const binary = atob(data)
  const bytes = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i)
  return new Blob([bytes], { type: mime || 'application/octet-stream' })
}
