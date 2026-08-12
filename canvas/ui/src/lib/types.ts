/**
 * Wire types shared by the page, the chat renderer, and the loaders.
 *
 * `CanvasRecord` mirrors the Rust contract in canvas/src/store.rs field for
 * field — a rename on either side is a breaking change for the other.
 */

export type CanvasFormat = 'mermaid' | 'freeform'

/** One stored canvas, exactly as the worker returns it. */
export interface CanvasRecord {
  /** Stable 8-character slug; never changes across updates. */
  id: string
  /** Human-readable canvas name. */
  name: string
  /** Diagram format. */
  format: CanvasFormat
  /** Editable source: mermaid text, or excalidraw scene JSON for freeform. */
  source: string
  /** Mermaid diagram family; null for freeform. */
  family: string | null
  /** Creation time, unix seconds. */
  created_at: number
  /** Last update time, unix seconds. */
  updated_at: number
}

/** `canvas::list` response. */
export interface CanvasListResponse {
  canvases: CanvasRecord[]
  count: number
}

/** `canvas::delete` response. */
export interface CanvasDeleteResponse {
  id: string
  deleted: boolean
}

/** One `canvas::syntax` reference entry. */
export interface FamilySyntax {
  family: string
  summary: string
  example: string
}

/** `canvas::syntax` response. */
export interface CanvasSyntaxResponse {
  families: FamilySyntax[]
}

/** One `canvas::validate` issue. */
export interface ValidationIssue {
  line: number | null
  message: string
}

/** `canvas::validate` response. */
export interface CanvasValidateResponse {
  valid: boolean
  family: string | null
  issues: ValidationIssue[]
}

/** The seven public function ids, in the worker's registration order. */
export const CANVAS_FUNCTION_IDS = [
  'canvas::create',
  'canvas::get',
  'canvas::list',
  'canvas::update',
  'canvas::delete',
  'canvas::syntax',
  'canvas::validate',
] as const

export type CanvasFunctionId = (typeof CANVAS_FUNCTION_IDS)[number]

/**
 * A function result reaches the console wrapped by the harness as
 * `{ content: [...], details: <the real response> }`, not as the response
 * itself. The console has its own unwrap, but injected assets can only import
 * from `@iii-dev/console-ui`, so the same two-line rule lives here.
 */
export function unwrapEnvelope(value: unknown): unknown {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return value
  const obj = value as Record<string, unknown>
  if (Array.isArray(obj.content) && 'details' in obj) return obj.details
  return value
}
