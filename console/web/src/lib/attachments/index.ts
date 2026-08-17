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

/** The one path an attachment takes. */
export type AttachmentKind = 'pdf' | 'document' | 'image' | 'text' | 'unknown'

/**
 * Which path this file takes — exactly one.
 *
 * The kinds overlap, and letting a file take two paths is not a harmless
 * duplicate: an SVG is both `image/svg+xml` and markup, so it was refused as a
 * picture no model can decode AND inlined as text that read perfectly well,
 * putting a "could not read" notice on a file the model had just read. A CSV is
 * both a spreadsheet and plain text, and went through the worker and the
 * browser both.
 *
 * Order is by how much is recovered. A PDF and an office document carry
 * structure only their worker can reconstruct. Markup — SVG, HTML, XML — is
 * text a model reads directly, and is worth more as characters than as a
 * picture it may not be able to decode. A raster image is worth more as pixels
 * than as the bytes underneath. Everything else that is text goes in as text.
 */
export function classifyAttachment(attachment: Attachment): AttachmentKind {
  if (isPdfAttachment(attachment)) return 'pdf'
  if (isDocumentAttachment(attachment)) return 'document'
  if (isMarkupAttachment(attachment)) return 'text'
  if (isImageAttachment(attachment)) return 'image'
  if (isTextAttachment(attachment)) return 'text'
  return 'unknown'
}

/**
 * Markup that a browser labels as an image. `image/svg+xml` is the case that
 * matters: no provider decodes an SVG as a picture, and every model reads it as
 * the markup it is.
 */
function isMarkupAttachment(attachment: Attachment): boolean {
  return (
    attachment.type === 'image/svg+xml' ||
    extensionOf(attachment.name) === 'svg'
  )
}

/**
 * Expand every attachment on a message.
 *
 * The passes run in sequence rather than concurrently. Each one is bounded by
 * its own per-send ceiling, and a composer holding a deck, a spreadsheet and
 * four screenshots would otherwise open a dozen simultaneous conversions on a
 * machine that is also running the model.
 */
export async function expandAttachments(
  attachments: Attachment[],
): Promise<ExpandedAttachments> {
  const withBytes = attachments.filter((a) => a.file)
  if (withBytes.length === 0) return EMPTY_EXPANSION

  const byKind = new Map<AttachmentKind, Attachment[]>()
  for (const attachment of withBytes) {
    const kind = classifyAttachment(attachment)
    byKind.set(kind, [...(byKind.get(kind) ?? []), attachment])
  }
  const of = (kind: AttachmentKind) => byKind.get(kind) ?? []

  const blocks: string[] = []
  const images: AttachmentImageBlock[] = []
  const read: AttachmentReadSummary[] = []
  const failures: AttachmentFailure[] = []

  const pdfs = of('pdf')
  if (pdfs.length > 0) {
    const expanded = await expandPdfAttachments(pdfs)
    blocks.push(...expanded.blocks)
    failures.push(...expanded.failures)
    read.push(
      ...expanded.read.map((summary) => ({
        id: summary.id,
        label: summaryLabel(nameOf(pdfs, summary.id) ?? 'document', summary),
      })),
    )
  }

  const documents = of('document')
  if (documents.length > 0) {
    const expanded = await expandDocumentAttachments(documents)
    blocks.push(...expanded.blocks)
    read.push(...expanded.read)
    failures.push(...expanded.failures)
  }

  const pictures = of('image')
  if (pictures.length > 0) {
    const expanded = await expandImageAttachments(pictures)
    blocks.push(...expanded.blocks)
    images.push(...expanded.images)
    read.push(...expanded.read)
    failures.push(...expanded.failures)
  }

  const texts = of('text')
  if (texts.length > 0) {
    const expanded = await expandTextAttachments(texts)
    blocks.push(...expanded.blocks)
    read.push(...expanded.read)
    failures.push(...expanded.failures)
  }

  // Anything left over reached the agent as nothing at all before this router
  // existed. Naming it in the message is the whole point: an unreadable
  // attachment the model knows about beats a silent one it does not.
  for (const attachment of of('unknown')) {
    const kind = attachment.type || extensionOf(attachment.name) || 'unknown'
    const reason = `${kind} is not a file the console can read into a message`
    blocks.push(failureBlock(attachment.name, reason))
    failures.push({ name: attachment.name, reason })
  }

  return { blocks, images, read, failures }
}

function nameOf(attachments: Attachment[], id: string): string | undefined {
  return attachments.find((a) => a.id === id)?.name
}
