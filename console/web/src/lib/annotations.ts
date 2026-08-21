/**
 * Annotations: numbered pins with notes over one captured image.
 *
 * A pin keeps its position as fractions of the image (0..1 each way), so the
 * same set renders over the live frame, the exported picture and a later
 * reload at any size. Everything here is pure: the layer and list components
 * draw it, the browser page (or any page with a picture) owns the state.
 */

export interface Annotation {
  id: string
  /** Position as fractions of the image width and height. */
  x: number
  y: number
  note: string
}

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

/** The painted box of an `object-fit: contain` image inside its element. */
export function containedImageBox(
  element: { width: number; height: number },
  image: { width: number; height: number },
): { left: number; top: number; width: number; height: number } {
  if (image.width <= 0 || image.height <= 0) {
    return { left: 0, top: 0, width: element.width, height: element.height }
  }
  const scale = Math.min(
    element.width / image.width,
    element.height / image.height,
  )
  const width = image.width * scale
  const height = image.height * scale
  return {
    left: (element.width - width) / 2,
    top: (element.height - height) / 2,
    width,
    height,
  }
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
