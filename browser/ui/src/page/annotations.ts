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
  label?: string,
): Annotation[] {
  const pin: Annotation = {
    id: newAnnotationId(),
    x: clamp(x),
    y: clamp(y),
    note: '',
  }
  if (label) pin.label = label
  return [...list, pin]
}

export function labelAnnotation(
  list: readonly Annotation[],
  id: string,
  label: string,
): Annotation[] {
  return list.map((a) => (a.id === id ? { ...a, label } : a))
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
    const label = a.label?.trim() ?? ''
    if (note && label) return `${index + 1}. ${note} (${label})`
    return `${index + 1}. ${note || label || '(no note)'}`
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

/** Longest side of a per-pin crop, in picture pixels. */
const CROP_SIZE = 640

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
  set.annotations.forEach((a, index) => {
    paintPin(
      context,
      a.x * canvas.width,
      a.y * canvas.height,
      index + 1,
      radius,
      options,
    )
  })
  return toPng(canvas)
}

/**
 * One pin's surroundings as a PNG: a window of the picture centred on the
 * pin (clamped to the edges) with that pin painted on, so each note can
 * travel as its own attachment and still show what it points at.
 */
export async function renderAnnotationCrop(
  set: AnnotationSet,
  id: string,
  options: AnnotationExportOptions = {},
): Promise<Blob> {
  const index = set.annotations.findIndex((a) => a.id === id)
  const pin = set.annotations[index]
  if (!pin) throw new Error('annotation not in set')
  const image = await loadImage(set.imageUrl)
  const width = Math.min(image.naturalWidth, CROP_SIZE)
  const height = Math.min(
    image.naturalHeight,
    Math.round((CROP_SIZE * 2) / 3),
  )
  const left = clampInt(
    pin.x * image.naturalWidth - width / 2,
    0,
    image.naturalWidth - width,
  )
  const top = clampInt(
    pin.y * image.naturalHeight - height / 2,
    0,
    image.naturalHeight - height,
  )
  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  const context = canvas.getContext('2d')
  if (!context) throw new Error('canvas 2d context unavailable')
  context.drawImage(image, left, top, width, height, 0, 0, width, height)
  paintPin(
    context,
    pin.x * image.naturalWidth - left,
    pin.y * image.naturalHeight - top,
    index + 1,
    Math.max(12, Math.round(Math.min(width, height) / 24)),
    options,
  )
  return toPng(canvas)
}

function paintPin(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  number: number,
  radius: number,
  options: AnnotationExportOptions,
) {
  context.font = `bold ${Math.round(radius * 1.15)}px system-ui, sans-serif`
  context.textAlign = 'center'
  context.textBaseline = 'middle'
  context.beginPath()
  context.arc(x, y, radius, 0, Math.PI * 2)
  context.fillStyle = options.color ?? accentColor()
  context.fill()
  context.lineWidth = Math.max(2, radius / 6)
  context.strokeStyle = '#ffffff'
  context.stroke()
  context.fillStyle = options.numberColor ?? '#ffffff'
  context.fillText(String(number), x, y + radius * 0.05)
}

const clampInt = (value: number, min: number, max: number) =>
  Math.round(Math.min(Math.max(value, min), Math.max(min, max)))

function toPng(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (blob) resolve(blob)
      else reject(new Error('canvas export failed'))
    }, 'image/png')
  })
}

/** The theme's accent, so the exported pins match the ones on screen. */
function accentColor(): string {
  const value =
    typeof getComputedStyle === 'function'
      ? getComputedStyle(document.documentElement)
          .getPropertyValue('--color-accent')
          .trim()
      : ''
  return value === '' ? '#b8420f' : value
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

/** A file name for one pin: its number, then the note (or the label). */
export function annotationPinFileName(
  annotation: Annotation,
  index: number,
  extension: string,
): string {
  const text = (annotation.note.trim() || annotation.label?.trim() || '')
    .replace(/[^a-z0-9]+/gi, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 40)
  return `pin-${index + 1}${text ? `-${text}` : ''}.${extension}`
}
