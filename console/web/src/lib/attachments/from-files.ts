/**
 * One file → one attachment, however it arrived.
 *
 * The composer takes files three ways — the paperclip, a drag onto the panel,
 * a paste — and all three have to produce the same thing, or a screenshot
 * pasted in behaves differently from the same screenshot picked from a dialog.
 */

import { uid } from '@/hooks/use-conversations'
import type { Attachment } from '@/types/chat'

/** Largest file read for a chip preview. Beyond this the chip shows an icon. */
const MAX_PREVIEW_BYTES = 1_000_000

/**
 * A data URL for the chip, for the two kinds where it is worth having: a
 * thumbnail of an image, and the first bytes of a text file. Never fails a
 * pick — a preview that cannot be read is simply absent.
 */
function readPreview(file: File): Promise<string | undefined> {
  if (file.size > MAX_PREVIEW_BYTES) return Promise.resolve(undefined)
  if (!/^(image|text)\//.test(file.type)) return Promise.resolve(undefined)
  return new Promise((resolve) => {
    const reader = new FileReader()
    reader.onload = () =>
      resolve(typeof reader.result === 'string' ? reader.result : undefined)
    reader.onerror = () => resolve(undefined)
    reader.readAsDataURL(file)
  })
}

/**
 * Build attachments for a set of picked, dropped or pasted files.
 *
 * The `File` is kept on the attachment so the send path can hand the bytes to
 * whichever worker reads that kind. It is browser-only and never persisted: a
 * conversation reloaded from history keeps the chip, not the document.
 */
export async function attachmentsFromFiles(
  files: File[],
): Promise<Attachment[]> {
  return Promise.all(
    files.map(async (file) => ({
      id: uid(),
      name: nameOf(file),
      size: file.size,
      type: file.type || 'application/octet-stream',
      dataUrl: await readPreview(file),
      file,
    })),
  )
}

/**
 * A pasted screenshot arrives as `image.png` on every platform, which makes
 * three of them indistinguishable in the chip strip. Only a genuinely nameless
 * file gets a generated name; a real one keeps its own.
 */
function nameOf(file: File): string {
  if (file.name) return file.name
  const extension = file.type.split('/')[1] ?? 'bin'
  return `pasted.${extension}`
}
