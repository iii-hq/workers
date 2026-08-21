export interface Size {
  width: number
  height: number
}

/** Scale and translation of the image's centre relative to the stage's centre. */
export interface ViewTransform {
  scale: number
  x: number
  y: number
}

export const MIN_SCALE = 0.05
export const MAX_SCALE = 32
export const ZOOM_STEP = 1.25

/** Contain the image in the stage without upscaling past its natural size. */
export function fitScale(image: Size, stage: Size): number {
  if (image.width <= 0 || image.height <= 0) return 1
  if (stage.width <= 0 || stage.height <= 0) return 1
  return Math.min(stage.width / image.width, stage.height / image.height, 1)
}

export function clampScale(scale: number, fit: number): number {
  const min = Math.min(MIN_SCALE, fit)
  if (!Number.isFinite(scale)) return fit
  return Math.min(MAX_SCALE, Math.max(min, scale))
}

/** A side shorter than the stage stays centred; a longer one never leaves a gap. */
export function clampOffset(
  view: ViewTransform,
  image: Size,
  stage: Size,
): ViewTransform {
  const width = image.width * view.scale
  const height = image.height * view.scale
  const maxX = Math.max(0, (width - stage.width) / 2)
  const maxY = Math.max(0, (height - stage.height) / 2)
  return {
    scale: view.scale,
    x: withoutNegativeZero(Math.min(maxX, Math.max(-maxX, view.x))),
    y: withoutNegativeZero(Math.min(maxY, Math.max(-maxY, view.y))),
  }
}

function withoutNegativeZero(value: number): number {
  return Object.is(value, -0) ? 0 : value
}

/** Zoom so the image point under `focus` (stage-centre coordinates) stays put. */
export function zoomAbout(
  view: ViewTransform,
  nextScale: number,
  focus: { x: number; y: number },
  image: Size,
  stage: Size,
  fit: number,
): ViewTransform {
  const scale = clampScale(nextScale, fit)
  const ratio = scale / view.scale
  return clampOffset(
    {
      scale,
      x: focus.x - (focus.x - view.x) * ratio,
      y: focus.y - (focus.y - view.y) * ratio,
    },
    image,
    stage,
  )
}

export function panBy(
  view: ViewTransform,
  dx: number,
  dy: number,
  image: Size,
  stage: Size,
): ViewTransform {
  return clampOffset({ ...view, x: view.x + dx, y: view.y + dy }, image, stage)
}

/** Centred at `scale`. */
export function centred(scale: number): ViewTransform {
  return { scale, x: 0, y: 0 }
}

export function zoomPercent(scale: number): string {
  return `${Math.round(scale * 100)}%`
}

const ZOOM_LADDER = [
  0.05, 0.1, 0.15, 0.25, 0.33, 0.5, 0.67, 0.75, 1, 1.25, 1.5, 2, 3, 4, 6, 8, 12,
  16, 24, 32,
] as const

/**
 * Zoom steps snap to a readable ladder around 100%. Past either end the step
 * is geometric and unclamped: the caller clamps against the fit, which may sit
 * below the ladder for very tall or wide images.
 */
export function steppedScale(scale: number, direction: 1 | -1): number {
  const epsilon = 1e-6
  if (direction === 1) {
    const next = ZOOM_LADDER.find((step) => step > scale + epsilon)
    return next ?? scale * ZOOM_STEP
  }
  for (let i = ZOOM_LADDER.length - 1; i >= 0; i -= 1) {
    if (ZOOM_LADDER[i] < scale - epsilon) return ZOOM_LADDER[i]
  }
  return scale / ZOOM_STEP
}

/** Wheel deltas in pixels whatever deltaMode the browser reports. */
export function wheelDeltaPixels(
  delta: number,
  deltaMode: number,
  pageHeight: number,
): number {
  if (deltaMode === 1) return delta * 16
  if (deltaMode === 2) return delta * Math.max(pageHeight, 1)
  return delta
}

export interface PointerPoint {
  id: number
  x: number
  y: number
}

/** Distance and midpoint of a two-pointer gesture. */
export function pinchGeometry(points: PointerPoint[]): {
  distance: number
  midpoint: { x: number; y: number }
} | null {
  if (points.length < 2) return null
  const [a, b] = points
  return {
    distance: Math.hypot(b.x - a.x, b.y - a.y),
    midpoint: { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 },
  }
}

/** A data URL this size or larger is refused rather than decoded. */
export const MAX_PREVIEW_BYTES = 48 * 1024 * 1024

export function dataUrlBytes(src: string): number | null {
  if (!src.startsWith('data:')) return null
  const comma = src.indexOf(',')
  if (comma < 0) return null
  const header = src.slice(0, comma)
  const payload = src.length - comma - 1
  if (!header.includes(';base64')) return payload
  const padding = src.endsWith('==') ? 2 : src.endsWith('=') ? 1 : 0
  return Math.floor((payload * 3) / 4) - padding
}
