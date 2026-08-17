/**
 * One router for everything attached to a message.
 *
 * The composer accepts anything a person can pick, drag, or paste, and each
 * kind reaches a model a different way: a PDF through the `pdf` worker, an
 * office document through the `document` worker, an image as a native image
 * content block, a text or source file inlined straight from the browser.
 * Deciding that in one place is what keeps the send path from growing a branch
 * per format.
 *
 * Nothing here blocks a send. A file that cannot be read becomes a block that
 * says so, in the message, where the model can see it — the failure mode this
 * whole path exists to prevent is an agent answering as though it had been
 * handed nothing.
 */

import type { Attachment } from '@/types/chat'
import { expandDocumentAttachments, isDocumentAttachment } from './documents'
import { expandImageAttachments, isImageAttachment } from './images'
import { expandPdfAttachments, isPdfAttachment, summaryLabel } from './pdf'
import { extensionOf, failureBlock } from './shared'
import { expandTextAttachments, isTextAttachment } from './text'

export { isDocumentAttachment } from './documents'
export { isImageAttachment } from './images'
export { isPdfAttachment } from './pdf'
export type {
  AttachmentFailure,
  AttachmentImageBlock,
  AttachmentReadSummary,
} from './shared'
export { isTextAttachment } from './text'

import type {
  AttachmentFailure,
  AttachmentImageBlock,
  AttachmentReadSummary,
} from './shared'

export interface ExpandedAttachments {
  /** `<attached-file …>` text blocks, appended to the outgoing message. */
  blocks: string[]
  /** Native image content blocks, appended after the text. */
  images: AttachmentImageBlock[]
  /** New chip labels, keyed by attachment id. */
  read: AttachmentReadSummary[]
  /** Everything that could not be read, for the notices above the composer. */
  failures: AttachmentFailure[]
}

export const EMPTY_EXPANSION: ExpandedAttachments = {
  blocks: [],
  images: [],
  read: [],
  failures: [],
}

/**
 * `true` when at least one attachment carries bytes this path can do something
 * with. The send path checks this before paying for the expansion, and a
 * conversation reloaded from history (chips, no bytes) answers `false`.
 */
export function hasExpandableAttachments(attachments: Attachment[]): boolean {
  return attachments.some((a) => a.file)
}

/**
 * Expand every attachment on a message.
 *
 * The four passes run in sequence rather than concurrently. Each one is bounded
 * by its own per-send ceiling, and a composer holding a deck, a spreadsheet and
 * four screenshots would otherwise open a dozen simultaneous conversions on a
 * machine that is also running the model.
 */
export async function expandAttachments(
  attachments: Attachment[],
): Promise<ExpandedAttachments> {
  const withBytes = attachments.filter((a) => a.file)
  if (withBytes.length === 0) return EMPTY_EXPANSION

  const blocks: string[] = []
  const images: AttachmentImageBlock[] = []
  const read: AttachmentReadSummary[] = []
  const failures: AttachmentFailure[] = []

  if (withBytes.some(isPdfAttachment)) {
    const pdfs = await expandPdfAttachments(withBytes)
    blocks.push(...pdfs.blocks)
    failures.push(...pdfs.failures)
    read.push(
      ...pdfs.read.map((summary) => ({
        id: summary.id,
        label: summaryLabel(
          nameOf(withBytes, summary.id) ?? 'document',
          summary,
        ),
      })),
    )
  }

  if (withBytes.some(isDocumentAttachment)) {
    const documents = await expandDocumentAttachments(withBytes)
    blocks.push(...documents.blocks)
    read.push(...documents.read)
    failures.push(...documents.failures)
  }

  if (withBytes.some(isImageAttachment)) {
    const pictures = await expandImageAttachments(withBytes)
    blocks.push(...pictures.blocks)
    images.push(...pictures.images)
    read.push(...pictures.read)
    failures.push(...pictures.failures)
  }

  if (withBytes.some(isTextAttachment)) {
    const texts = await expandTextAttachments(withBytes)
    blocks.push(...texts.blocks)
    read.push(...texts.read)
    failures.push(...texts.failures)
  }

  // Anything left over reached the agent as nothing at all before this router
  // existed. Naming it in the message is the whole point: an unreadable
  // attachment the model knows about beats a silent one it does not.
  for (const attachment of withBytes.filter(isUnhandled)) {
    const kind = attachment.type || extensionOf(attachment.name) || 'unknown'
    const reason = `${kind} is not a file the console can read into a message`
    blocks.push(failureBlock(attachment.name, reason))
    failures.push({ name: attachment.name, reason })
  }

  return { blocks, images, read, failures }
}

function isUnhandled(attachment: Attachment): boolean {
  return (
    !isPdfAttachment(attachment) &&
    !isDocumentAttachment(attachment) &&
    !isImageAttachment(attachment) &&
    !isTextAttachment(attachment)
  )
}

function nameOf(attachments: Attachment[], id: string): string | undefined {
  return attachments.find((a) => a.id === id)?.name
}
