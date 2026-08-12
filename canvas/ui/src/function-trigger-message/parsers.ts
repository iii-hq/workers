/**
 * Pure payload logic for the canvas chat renderer — no React, no DOM, no
 * console-ui value imports, so every function here is testable in plain node.
 *
 * Wire shapes tolerated (per console/docs/custom-function-call-message.md §4):
 * raw handler JSON, the harness `{content, details}` envelope (unwrapped via
 * ../lib/types), and the `{error: {...}}` transport wrapper. Anything
 * unrecognizable parses to "nothing" so the renderer returns null and falls
 * through to the console's own card instead of showing an empty one.
 */

import { unwrapEnvelope, type CanvasFormat } from '../lib/types'

export const CANVAS_PREFIX = 'canvas::'

/** canvas::list rows shown before collapsing to "+N more". */
export const LIST_CAP = 20

/** Scenes bigger than this are not parsed for an element count. */
export const SCENE_PARSE_CAP = 512 * 1024

export function asRecord(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {}
  return value as Record<string, unknown>
}

export function asString(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined
}

export function asNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}

/** The fields of a CanvasRecord any payload managed to carry. */
export interface CanvasRecordView {
  id?: string
  name?: string
  format?: CanvasFormat
  source?: string
  family?: string
  updated_at?: number
}

export function parseRecordView(value: unknown): CanvasRecordView {
  const rec = asRecord(unwrapEnvelope(value))
  const format = asString(rec.format)
  return {
    id: asString(rec.id),
    name: asString(rec.name),
    format:
      format === 'mermaid' || format === 'freeform' ? format : undefined,
    source: asString(rec.source),
    family: asString(rec.family) ?? undefined,
    updated_at: asNumber(rec.updated_at),
  }
}

/** Output view over an input-view fallback (running / partial outputs). */
export function mergeViews(
  primary: CanvasRecordView,
  fallback: CanvasRecordView,
): CanvasRecordView {
  return {
    id: primary.id ?? fallback.id,
    name: primary.name ?? fallback.name,
    format: primary.format ?? fallback.format,
    source: primary.source ?? fallback.source,
    family: primary.family ?? fallback.family,
    updated_at: primary.updated_at ?? fallback.updated_at,
  }
}

export function hasContent(view: CanvasRecordView): boolean {
  return Boolean(view.source || view.name || view.id)
}

/**
 * Freeform when declared, or when nothing is declared but the source is
 * parseable excalidraw scene JSON (an update input can carry source alone).
 */
export function looksFreeform(view: CanvasRecordView): boolean {
  if (view.format === 'freeform') return true
  if (view.format === 'mermaid' || view.family) return false
  return sceneElementCount(view.source) != null
}

/** Live (non-deleted) element count of an excalidraw scene, or null. */
export function sceneElementCount(source: string | undefined): number | null {
  if (!source || source.length > SCENE_PARSE_CAP) return null
  let parsed: unknown
  try {
    parsed = JSON.parse(source)
  } catch {
    return null
  }
  const elements = Array.isArray(parsed)
    ? parsed
    : Array.isArray(asRecord(parsed).elements)
      ? (asRecord(parsed).elements as unknown[])
      : null
  if (!elements) return null
  return elements.filter((el) => asRecord(el).isDeleted !== true).length
}

export function capList<T>(
  items: readonly T[],
  cap: number = LIST_CAP,
): { shown: T[]; hidden: number } {
  if (items.length <= cap) return { shown: [...items], hidden: 0 }
  return { shown: items.slice(0, cap), hidden: items.length - cap }
}

export function parseListResponse(value: unknown): CanvasRecordView[] | null {
  const rec = asRecord(unwrapEnvelope(value))
  if (!Array.isArray(rec.canvases)) return null
  return rec.canvases.map((entry) => parseRecordView(entry))
}

export function parseDeleteResponse(
  value: unknown,
): { id: string; deleted: boolean } | null {
  const rec = asRecord(unwrapEnvelope(value))
  const id = asString(rec.id)
  if (id === undefined || typeof rec.deleted !== 'boolean') return null
  return { id, deleted: rec.deleted }
}

export function parseSyntaxFamilies(value: unknown): string[] | null {
  const rec = asRecord(unwrapEnvelope(value))
  if (!Array.isArray(rec.families)) return null
  const families = rec.families
    .map((entry) => asString(asRecord(entry).family) ?? asString(entry))
    .filter((family): family is string => family !== undefined)
  return families
}

export interface ValidateView {
  valid: boolean
  family: string | null
  issues: { line: number | null; message: string }[]
}

export function parseValidateResponse(value: unknown): ValidateView | null {
  const rec = asRecord(unwrapEnvelope(value))
  if (typeof rec.valid !== 'boolean') return null
  const issues = Array.isArray(rec.issues)
    ? rec.issues.flatMap((entry) => {
        const issue = asRecord(entry)
        const message = asString(issue.message)
        if (message === undefined) return []
        return [{ line: asNumber(issue.line) ?? null, message }]
      })
    : []
  return { valid: rec.valid, family: asString(rec.family) ?? null, issues }
}

/**
 * The human-readable error string of a failed call, or null for success
 * shapes. Handles the transport `{error: {kind, message, details}}` wrapper
 * and worker `{error: "..."}` bodies, on the raw output or inside the
 * envelope.
 */
export function errorDisplay(output: unknown): string | null {
  if (output == null) return null
  const err =
    asRecord(output).error ?? asRecord(unwrapEnvelope(output)).error
  if (err == null) return null
  if (typeof err === 'string') return err
  const rec = asRecord(err)
  const message = asString(rec.message)
  const reason = asString(asRecord(rec.details).reason)
  if (message) {
    return reason && reason !== message ? `${message} — ${reason}` : message
  }
  try {
    return JSON.stringify(err)
  } catch {
    return 'call failed'
  }
}

/** Unix seconds → `YYYY-MM-DD`, deterministic for tests. */
export function formatDay(secs: number | undefined): string | null {
  if (secs === undefined) return null
  return new Date(secs * 1000).toISOString().slice(0, 10)
}
