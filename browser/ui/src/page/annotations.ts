/**
 * Annotations over one captured frame: the pure half the browser page owns.
 * The pin type and the layer/list components come from console-ui; the
 * helpers here build, export and describe a set. Mirrors
 * console/web/src/lib/annotations.ts.
 */

import type { Annotation } from '@iii-dev/console-ui'

export type { Annotation }

export interface AnnotationSet {
  /** What was annotated: a page url, a file, a session. */
  subject: string
  /** The picture the pins sit on, as a `data:` url. */
  imageUrl: string
  /** The picture's pixel size, for the exported file and the wire. */
  width: number
  height: number
  annotations: readonly Annotation[]
  capturedAt: number
}

export function newAnnotationId(): string {
  return `a-${Math.random().toString(36).slice(2, 10)}`
}

const clamp = (value: number) => Math.min(1, Math.max(0, value))

export function addAnnotation(
  list: readonly Annotation[],
  x: number,
  y: number,
): Annotation[] {
  return [
    ...list,
    { id: newAnnotationId(), x: clamp(x), y: clamp(y), note: '' },
  ]
}

export function moveAnnotation(
  list: readonly Annotation[],
  id: string,
  x: number,
  y: number,
): Annotation[] {
  return list.map((a) => (a.id === id ? { ...a, x: clamp(x), y: clamp(y) } : a))
}

export function noteAnnotation(
  list: readonly Annotation[],
  id: string,
  note: string,
): Annotation[] {
  return list.map((a) => (a.id === id ? { ...a, note } : a))
}

export function removeAnnotation(
  list: readonly Annotation[],
  id: string,
): Annotation[] {
  return list.filter((a) => a.id !== id)
}

/** Pin notes as the text that goes with the picture into a chat. */
export function annotationsMarkdown(set: AnnotationSet): string {
  const lines = set.annotations.map((a, index) => {
    const note = a.note.trim()
    return `${index + 1}. ${note === '' ? '(no note)' : note}`
  })
  const count = set.annotations.length
  return [
    `Annotations on ${set.subject} (${count} ${count === 1 ? 'pin' : 'pins'})`,
    ...lines,
  ].join('\n')
}

export interface AnnotationExportOptions {
  /** CSS colour for the pins. */
  color?: string
  /** CSS colour for the numbers. */
  numberColor?: string
}

/**
 * The picture with the pins painted on, as a PNG. Pin size follows the image
 * so a 4K capture and a phone capture read the same when scaled.
 */
export async function renderAnnotatedImage(
  set: AnnotationSet,
  options: AnnotationExportOptions = {},
): Promise<Blob> {
  const image = await loadImage(set.imageUrl)
  const canvas = document.createElement('canvas')
  canvas.width = image.naturalWidth
  canvas.height = image.naturalHeight
  const context = canvas.getContext('2d')
  if (!context) throw new Error('canvas 2d context unavailable')
  context.drawImage(image, 0, 0)
  const radius = Math.max(
    12,
    Math.round(Math.min(canvas.width, canvas.height) / 48),
  )
  context.font = `bold ${Math.round(radius * 1.15)}px system-ui, sans-serif`
  context.textAlign = 'center'
  context.textBaseline = 'middle'
  set.annotations.forEach((a, index) => {
    const x = a.x * canvas.width
    const y = a.y * canvas.height
    context.beginPath()
    context.arc(x, y, radius, 0, Math.PI * 2)
    context.fillStyle = options.color ?? '#b8420f'
    context.fill()
    context.lineWidth = Math.max(2, radius / 6)
    context.strokeStyle = '#ffffff'
    context.stroke()
    context.fillStyle = options.numberColor ?? '#ffffff'
    context.fillText(String(index + 1), x, y + radius * 0.05)
  })
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (blob) resolve(blob)
      else reject(new Error('canvas export failed'))
    }, 'image/png')
  })
}

function loadImage(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image()
    image.onload = () => resolve(image)
    image.onerror = () => reject(new Error('image failed to load'))
    image.src = url
  })
}

/** A file name for the export: subject, then the time. */
export function annotationFileName(
  set: AnnotationSet,
  extension: string,
): string {
  const subject = set.subject
    .replace(/^https?:\/\//, '')
    .replace(/[^a-z0-9.-]+/gi, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 48)
  const stamp = new Date(set.capturedAt).toISOString().replace(/[:.]/g, '-')
  return `annotations-${subject || 'capture'}-${stamp}.${extension}`
}
