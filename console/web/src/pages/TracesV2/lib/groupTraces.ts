// Pure helpers for the TRACES tab "Group by" affordance.
//
// The server-side aggregation (`engine::traces::group_by`) returns
// `TraceGroup[]`; this module formats and classifies that data for the
// UI without re-implementing any grouping logic. Extracted from the
// route component so it's testable in isolation when the console grows
// test infrastructure.

import type { TraceGroup } from '../api/traces'

/**
 * Preset span attributes the TRACES tab can group by — the identity keys
 * iii components stamp into OTel baggage (which copies to span attributes
 * unconditionally; the key conventions live in the engine's traces API).
 */
export const GROUP_BY_PRESETS = [
  'iii.message.id',
  'iii.session.id',
  'iii.function.id',
] as const

export type GroupByPreset = (typeof GROUP_BY_PRESETS)[number]

/**
 * 'none' | a preset | any other attribute key (custom grouping — e.g. an
 * `iii.tag.*` trace tag). The engine groups by whatever key it's given.
 */
export type GroupByOption = 'none' | GroupByPreset | (string & {})

/** The trace-tag namespace (see the SDK BaggageSpanProcessor prefixes). */
export const TRACE_TAG_PREFIX = 'iii.tag.'

/**
 * Human-readable label for the dropdown. Custom attributes render their
 * key, with the `iii.tag.` namespace shortened to `tag: <key>`.
 */
export function groupByLabel(option: GroupByOption): string {
  switch (option) {
    case 'none':
      return 'no grouping'
    case 'iii.message.id':
      return 'group by message'
    case 'iii.session.id':
      return 'group by session'
    case 'iii.function.id':
      return 'group by function'
    default:
      return option.startsWith(TRACE_TAG_PREFIX)
        ? `group by tag: ${option.slice(TRACE_TAG_PREFIX.length)}`
        : `group by ${option}`
  }
}

/**
 * The display attribute the engine should resolve per group
 * (`label_attribute`): id-keyed groupings get their human-readable
 * companion — session name for sessions, the harness's message-preview
 * tag for messages. Value-keyed groupings (function id, custom tags)
 * are already readable.
 */
export function defaultLabelAttribute(
  option: GroupByOption,
): string | undefined {
  switch (option) {
    case 'iii.session.id':
      return 'iii.session.name'
    case 'iii.message.id':
      return 'iii.tag.message'
    default:
      return undefined
  }
}

/**
 * Format the group's metadata into the inline summary string the row
 * shows, e.g. "(12 spans, 1.42s, 1 error)".
 *
 * Handles the wildcard session sentinel `*` from `ui::subscribe` /
 * `ui::unsubscribe` calls by emitting "all sessions (wildcard)" rather
 * than the literal asterisk.
 */
export function summarizeGroup(group: TraceGroup): string {
  const errors =
    group.error_count === 0
      ? '0 errors'
      : group.error_count === 1
        ? '1 error'
        : `${group.error_count} errors`
  const traces =
    group.trace_ids.length === 1
      ? '1 trace'
      : `${group.trace_ids.length} traces`
  return `${traces}, ${group.span_count} spans, ${formatDurationMs(group.duration_ms)}, ${errors}`
}

/**
 * Display string for the group's heading: the engine-resolved label
 * (session name / message preview) when present, else the raw grouped
 * value. Hides the wildcard sentinel behind a friendlier label.
 */
export function groupHeading(group: TraceGroup, option: GroupByOption): string {
  if (group.value === '*' && option === 'iii.session.id') {
    return 'all sessions (wildcard)'
  }
  if (group.label) return group.label
  return group.value
}

/**
 * True when the heading is a raw opaque id (no label resolved) — the row
 * renders it in the faint style so readable headings stand out.
 */
export function groupHeadingIsOpaque(
  group: TraceGroup,
  option: GroupByOption,
): boolean {
  if (group.label) return false
  return option === 'iii.session.id' || option === 'iii.message.id'
}

function formatDurationMs(ms: number): string {
  if (ms < 1) return '<1ms'
  if (ms < 1_000) return `${Math.round(ms)}ms`
  if (ms < 60_000) return `${(ms / 1_000).toFixed(2)}s`
  const seconds = Math.round(ms / 1_000)
  const m = Math.floor(seconds / 60)
  const s = seconds % 60
  return `${m}m${s}s`
}

/**
 * Heuristic: tells the caller whether `engine::traces::group_by` is
 * unavailable in this engine deploy (older version, or
 * `iii-observability` not configured). Used so the dropdown can hide
 * itself gracefully rather than surfacing an error.
 *
 * Matches the engine's `function_not_found` error_code. Earlier versions
 * of this helper also matched bare `group_by` and `404` substrings,
 * which was too broad — a legitimate "failed to group_by: timeout"
 * would have been silently swallowed.
 */
export function isGroupByUnavailable(err: unknown): boolean {
  const message = err instanceof Error ? err.message : String(err)
  return /function[_ ]not[_ ]found/i.test(message)
}
