/**
 * Pure page logic, kept out of components so it is testable without a DOM:
 * family badge labels, export filename slugs, relative timestamps, error
 * normalization, and the starter source every new canvas is created with.
 */

import type { CanvasFormat } from '../lib/types'

/**
 * Mermaid family → short badge label. Keys are the NORMALIZED family (see
 * `familyBadgeLabel`): lowercased, version/beta suffix stripped, then a
 * trailing `diagram` stripped — so `stateDiagram-v2`, `stateDiagram` and
 * `state` all land on the same key.
 */
const FAMILY_LABELS: Record<string, string> = {
  architecture: 'arch',
  block: 'block',
  c4: 'c4',
  class: 'class',
  er: 'er',
  flowchart: 'flow',
  gantt: 'gantt',
  gitgraph: 'git',
  graph: 'flow',
  journey: 'journey',
  kanban: 'kanban',
  mindmap: 'mind',
  packet: 'packet',
  pie: 'pie',
  quadrantchart: 'quad',
  requirement: 'req',
  sankey: 'sankey',
  sequence: 'seq',
  state: 'state',
  timeline: 'time',
  xychart: 'xy',
}

const MAX_BADGE_CHARS = 10

/** The short label the sidebar badge shows for one canvas. */
export function familyBadgeLabel(
  format: CanvasFormat,
  family: string | null,
): string {
  if (format === 'freeform') return 'freeform'
  if (!family) return 'mermaid'
  const bare = family
    .toLowerCase()
    .replace(/-(v\d+|beta)$/, '')
    .replace(/diagram$/, '')
  if (bare.length === 0) return 'mermaid'
  return FAMILY_LABELS[bare] ?? bare.slice(0, MAX_BADGE_CHARS)
}

/**
 * Download filename for an export: the canvas name slugged to safe ascii
 * (accents folded, everything else collapsed to single dashes), falling
 * back to `canvas` when nothing survives.
 */
export function exportFilename(name: string, ext: 'svg' | 'png'): string {
  const slug = name
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[\u0300-\u036f]/g, '')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 64)
    .replace(/-+$/, '')
  return `${slug || 'canvas'}.${ext}`
}

/** Compact "how long ago" for the sidebar rows; absolute date past 30 days. */
export function relativeTime(unixSecs: number, nowSecs: number): string {
  const delta = Math.max(0, nowSecs - unixSecs)
  if (delta < 60) return 'just now'
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`
  if (delta < 86400) return `${Math.floor(delta / 3600)}h ago`
  if (delta < 86400 * 30) return `${Math.floor(delta / 86400)}d ago`
  return new Date(unixSecs * 1000).toISOString().slice(0, 10)
}

/** Normalize whatever a rejected bus call throws into a readable string. */
export function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err
  if (err && typeof err === 'object') {
    const message = (err as { message?: unknown }).message
    if (typeof message === 'string') return message
    try {
      return JSON.stringify(err)
    } catch {
      // fall through to String()
    }
  }
  return String(err)
}

/** What `canvas::create` is seeded with from the sidebar's new button. */
export const STARTER_FLOWCHART = `flowchart TD
    start([start]) --> step[do the thing]
    step --> check{worked?}
    check -- yes --> ship([ship it])
    check -- no --> step
`

/** Name new canvases are created under (rename comes later, via update). */
export const NEW_CANVAS_NAME = 'untitled'
