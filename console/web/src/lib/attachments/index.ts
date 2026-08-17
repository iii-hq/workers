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
import {
  type ExpandedImages,
  expandImageAttachments,
  isImageAttachment,
} from './images'
import { expandPdfAttachments, isPdfAttachment } from './pdf'
import { extensionOf, failureBlock, reportDropped } from './shared'
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

export interface ExpandOptions {
  /**
   * What the model on the other end can do with a picture. `false` refuses
   * images with an explanation instead of sending pixels nothing will look at;
   * `undefined` means the catalog did not say, and the image goes as before.
   */
  vision?: boolean
  /** Model id, so the refusal names what to switch away from. */
  model?: string | null
}

/**
 * Expand every attachment on a message.
 *
 * The four passes run concurrently and the files INSIDE each pass run one at a
 * time. That split is the point: two documents queue against the same worker
 * rather than opening simultaneous conversions on a machine that is also
 * running the model, while a PDF, a spreadsheet and a screenshot — different
 * workers, and in the image and text cases no worker at all — have nothing to
 * contend over. This sits on the send path, so the message waits for the
 * slowest pass rather than the sum of them.
 */
export async function expandAttachments(
  attachments: Attachment[],
  options: ExpandOptions = {},
): Promise<ExpandedAttachments> {
  const withBytes = attachments.filter((a) => a.file)
  if (withBytes.length === 0) return EMPTY_EXPANSION

  const byKind = new Map<AttachmentKind, Attachment[]>()
  for (const attachment of withBytes) {
    const kind = classifyAttachment(attachment)
    byKind.set(kind, [...(byKind.get(kind) ?? []), attachment])
  }
  const of = (kind: AttachmentKind) => byKind.get(kind) ?? []

  const pictures = of('image')
  const [pdfs, documents, imagery, texts] = await Promise.all([
    expandPdfAttachments(of('pdf')),
    expandDocumentAttachments(of('document')),
    // A model with no vision receives an image block and does nothing with it:
    // the picture is dropped somewhere downstream and the answer arrives as
    // though nothing was attached, the exact silence this whole path exists to
    // prevent. Refuse it here, in the message, naming the way out.
    options.vision === false
      ? refuseImages(pictures, options.model)
      : expandImageAttachments(pictures),
    expandTextAttachments(of('text')),
  ])

  const blocks: string[] = [
    ...pdfs.blocks,
    ...documents.blocks,
    ...imagery.blocks,
    ...texts.blocks,
  ]
  const read: AttachmentReadSummary[] = [
    ...pdfs.read,
    ...documents.read,
    ...imagery.read,
    ...texts.read,
  ]
  const failures: AttachmentFailure[] = [
    ...pdfs.failures,
    ...documents.failures,
    ...imagery.failures,
    ...texts.failures,
  ]
  const images: AttachmentImageBlock[] = [...imagery.images]

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

/** The image pass, replaced by an explanation, for a model that cannot see. */
function refuseImages(
  pictures: Attachment[],
  model: string | null | undefined,
): ExpandedImages {
  const refused: ExpandedImages = {
    images: [],
    blocks: [],
    read: [],
    failures: [],
  }
  const reason = `${model ?? 'the selected model'} cannot read images, so this one was not sent — switch to a model with vision`
  reportDropped(pictures, reason, refused)
  return refused
}
