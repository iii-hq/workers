/**
 * Pure scene helpers for the freeform (excalidraw) surface.
 *
 * NO vendor imports here — this module is bundled into page.js (FreeformPane)
 * AND into freeform.js (the converter), so it must stay dependency-free.
 * Everything is total: any string goes in, a usable scene shape comes out.
 */

/** What `FreeformPane` hands excalidraw as `initialData`. */
export interface ParsedScene {
  elements: unknown[]
  appState: Record<string, unknown>
  files: Record<string, unknown>
}

/**
 * appState keys that must never be replayed into a mounting excalidraw:
 * `collaborators` deserializes as a plain object where the runtime expects a
 * Map (the classic `.forEach is not a function` crash), and the
 * geometry/viewport keys describe the pane the scene was SAVED in, not the
 * pane it is opening in.
 */
const DROPPED_APP_STATE_KEYS = new Set([
  'collaborators',
  'width',
  'height',
  'offsetTop',
  'offsetLeft',
])

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function blankScene(): ParsedScene {
  return { elements: [], appState: {}, files: {} }
}

/** Strip the appState keys that break (or mislead) a fresh mount. */
export function sanitizeAppState(
  raw: Record<string, unknown>,
): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(raw)) {
    if (!DROPPED_APP_STATE_KEYS.has(key)) out[key] = value
  }
  return out
}

/**
 * Parse a `CanvasRecord.source` (excalidraw scene JSON) into `initialData`.
 *
 * Tolerant by contract: an empty string, invalid JSON, or any non-scene shape
 * yields a blank scene — a freshly created freeform record starts with
 * `source: ""` and must open as an empty whiteboard, never as an error.
 */
export function parseSceneSource(source: string): ParsedScene {
  if (typeof source !== 'string' || source.trim() === '') return blankScene()
  let parsed: unknown
  try {
    parsed = JSON.parse(source)
  } catch {
    return blankScene()
  }
  if (!isPlainObject(parsed)) return blankScene()
  return {
    elements: Array.isArray(parsed.elements) ? parsed.elements : [],
    appState: isPlainObject(parsed.appState)
      ? sanitizeAppState(parsed.appState)
      : {},
    files: isPlainObject(parsed.files) ? parsed.files : {},
  }
}

/**
 * True when a mermaid→excalidraw conversion produced the library's LAST-RESORT
 * output: unsupported diagram families come back as a single rasterized image
 * element instead of editable shapes. The page surfaces a small note when this
 * flag is set.
 */
export function isImageFallback(
  elements: ReadonlyArray<{ type?: unknown }>,
): boolean {
  return elements.length > 0 && elements.every((el) => el.type === 'image')
}

const MAX_FILENAME_STEM = 64

/**
 * Download filename for a scene export: the record name slugified to a safe
 * `[a-z0-9._-]` stem (empty names fall back to `canvas`), capped at 64 chars.
 */
export function exportFilename(name: string, ext: 'png' | 'svg'): string {
  const stem = (typeof name === 'string' ? name : '')
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, '-')
    .replace(/-{2,}/g, '-')
    .replace(/^[-.]+|[-.]+$/g, '')
    .slice(0, MAX_FILENAME_STEM)
  return `${stem === '' ? 'canvas' : stem}.${ext}`
}
