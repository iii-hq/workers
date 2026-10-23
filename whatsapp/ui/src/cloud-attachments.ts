import { formatBytes } from '@iii-dev/console-ui/format'

export type CloudMediaType = 'document' | 'image' | 'audio' | 'video' | 'sticker'
export interface CloudAttachmentSpec { mediaType: CloudMediaType; mimetype: string; maxBytes: number; caption: boolean }
export interface CloudUploadedAttachment { media_id: string; media_type: CloudMediaType; file_name: string; mimetype: string; file_length: number }

const DOCUMENT_MIMES: Record<string, string> = {
  txt: 'text/plain', pdf: 'application/pdf', doc: 'application/msword',
  docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  xls: 'application/vnd.ms-excel', xlsx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  ppt: 'application/vnd.ms-powerpoint', pptx: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
}
const MEDIA: Record<string, { mediaType: CloudMediaType; mimetype: string; maxBytes: number }> = {
  jpg: { mediaType: 'image', mimetype: 'image/jpeg', maxBytes: 5_000_000 },
  jpeg: { mediaType: 'image', mimetype: 'image/jpeg', maxBytes: 5_000_000 },
  png: { mediaType: 'image', mimetype: 'image/png', maxBytes: 5_000_000 },
  aac: { mediaType: 'audio', mimetype: 'audio/aac', maxBytes: 16_000_000 },
  amr: { mediaType: 'audio', mimetype: 'audio/amr', maxBytes: 16_000_000 },
  mp3: { mediaType: 'audio', mimetype: 'audio/mpeg', maxBytes: 16_000_000 },
  m4a: { mediaType: 'audio', mimetype: 'audio/mp4', maxBytes: 16_000_000 },
  ogg: { mediaType: 'audio', mimetype: 'audio/ogg', maxBytes: 16_000_000 },
  mp4: { mediaType: 'video', mimetype: 'video/mp4', maxBytes: 16_000_000 },
  '3gp': { mediaType: 'video', mimetype: 'video/3gpp', maxBytes: 16_000_000 },
  webp: { mediaType: 'sticker', mimetype: 'image/webp', maxBytes: 500_000 },
}

export const CLOUD_ATTACHMENT_ACCEPT = [...Object.keys(DOCUMENT_MIMES), ...Object.keys(MEDIA)].map((extension) => `.${extension}`).join(',')

/** Selection checks are advisory; the worker validates MIME, bytes and codecs again. */
export function cloudAttachmentSpec(file: Pick<File, 'name' | 'type' | 'size'>, workerMaxBytes = 64 * 1024 * 1024): CloudAttachmentSpec {
  const extension = file.name.split('.').at(-1)?.toLowerCase() ?? ''
  const declaredMIME = file.type.toLowerCase().split(';')[0].trim()
  if (extension === 'md' || extension === 'markdown' || declaredMIME === 'text/markdown' || declaredMIME === 'text/x-markdown') {
    throw new Error('Markdown is not supported by the WhatsApp Cloud API. Choose a TXT or PDF file you have explicitly prepared; this app does not rename or convert files.')
  }
  if (!file.name || new TextEncoder().encode(file.name).length > 255 || /[/\\\u0000-\u001f\u007f]/.test(file.name)) throw new Error('Use a filename of at most 255 UTF-8 bytes without paths or control characters.')
  const documentMIME = DOCUMENT_MIMES[extension]
  const spec = documentMIME ? { mediaType: 'document' as const, mimetype: documentMIME, maxBytes: 100_000_000 } : MEDIA[extension]
  if (!spec) throw new Error('This file format is not supported. Choose a supported document, image, audio, video or WebP sticker.')
  if (declaredMIME && declaredMIME !== 'application/octet-stream' && declaredMIME !== spec.mimetype) throw new Error('The file type does not match its extension. Choose the original file with a matching MIME type; files are not renamed or converted.')
  if (!Number.isSafeInteger(workerMaxBytes) || workerMaxBytes < 1 || workerMaxBytes > 64 * 1024 * 1024) throw new Error('The active upload limit could not be verified. Check the worker configuration before uploading.')
  const maxBytes = Math.min(spec.maxBytes, workerMaxBytes)
  if (file.size < 1) throw new Error('Choose a file that is not empty.')
  if (file.size > maxBytes) throw new Error(`This file exceeds the ${formatBytes(maxBytes)} upload limit for this file type and worker.`)
  return { ...spec, maxBytes, caption: ['document', 'image', 'video'].includes(spec.mediaType) }
}

export function readCloudAttachment(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onerror = () => reject(new Error('Could not read this file. Choose it again before sending.'))
    reader.onload = () => resolve((reader.result as string).split(',', 2)[1])
    reader.readAsDataURL(file)
  })
}
