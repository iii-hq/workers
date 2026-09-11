/**
 * Span-filter selection for the trace detail views: which span groups,
 * which workers, and which INTERNAL span families are hidden by the funnel
 * menu. ONE selection is shared by the timeline and waterfall views and
 * persisted in the server-side `console` configuration entry under
 * `traces.spanFilters`, so it follows the engine (and every browser tab
 * pointing at it) rather than living per-component.
 *
 * Three layers make up the selection:
 *
 * - **User prefs** ([`SpanFilterPrefs`], the persisted shape): groups and
 *   workers the user hid, `shownGroups` — producer-default-hidden groups
 *   the user explicitly turned back on — and `shownInternal` — internal
 *   span families the user explicitly revealed.
 * - **Producer defaults**: functions registered with `trace_hidden: true`
 *   metadata (read via `engine::functions::list`, see
 *   `@/lib/trace-hidden-functions` and
 *   workers/docs/sops/trace-hidden-functions.md) are hidden by default.
 * - **Internal spans**: spans stamped `iii.tag.hidden = <family>` at the
 *   CALL SITE (baggage — harness state bookkeeping, session-event fan-out)
 *   form the funnel's separate "internal" section and are ALWAYS hidden by
 *   default; `shownInternal` holds the families the user unhid.
 *
 * [`effectiveSpanFilters`] folds the producer defaults into the
 * [`SpanFilterSelection`] the views consume: hidden = userHidden ∪
 * (producerHidden − shownGroups). Internal spans need no folding — hidden
 * is the default, so the selection just carries `shownInternal` through.
 *
 * Pure module — parse/serialize/merge only. Transport lives in
 * `@/lib/console-config`, React state in `hooks/useSpanFilterSelection.ts`,
 * the hiding mechanics in `components/timeline/spanVisibility.ts`.
 */

export interface SpanFilterSelection {
  /** Hidden span-group keys (owning function id — see `traceTimelineFilters`). */
  hiddenGroups: ReadonlySet<string>
  /** Hidden worker names (`getWorkerName`). */
  hiddenWorkers: ReadonlySet<string>
  /** Internal span families (`iii.tag.hidden` values) the user UNHID —
   *  internal spans are hidden by default. */
  shownInternal: ReadonlySet<string>
}

/** The persisted user preferences behind the effective selection. */
export interface SpanFilterPrefs extends SpanFilterSelection {
  /** Producer-default-hidden groups the user explicitly unhid. */
  shownGroups: ReadonlySet<string>
}

/** Selection plus the mutations the funnel menu needs. */
export interface SpanFilterControls extends SpanFilterSelection {
  toggleGroup: (key: string) => void
  toggleWorker: (key: string) => void
  /** Toggle one internal family between hidden (default) and shown. */
  toggleInternal: (family: string) => void
  /**
   * "show all": clear every hide AND reveal the given internal families
   * (the caller passes the families currently in view — the hook cannot
   * know them, they derive from spans).
   */
  clear: (visibleInternal?: readonly string[]) => void
}

export const EMPTY_SPAN_FILTER_PREFS: SpanFilterPrefs = {
  hiddenGroups: new Set(),
  hiddenWorkers: new Set(),
  shownGroups: new Set(),
  shownInternal: new Set(),
}

function parseKeySet(value: unknown): ReadonlySet<string> {
  if (!Array.isArray(value)) return new Set()
  return new Set(value.filter((v): v is string => typeof v === 'string'))
}

/** Parse `traces.spanFilters` out of the raw console-config value. */
export function parseSpanFilters(
  configValue: Record<string, unknown>,
): SpanFilterPrefs {
  const traces = configValue.traces
  if (!traces || typeof traces !== 'object') return EMPTY_SPAN_FILTER_PREFS
  const filters = (traces as Record<string, unknown>).spanFilters
  if (!filters || typeof filters !== 'object') return EMPTY_SPAN_FILTER_PREFS
  const f = filters as Record<string, unknown>
  return {
    hiddenGroups: parseKeySet(f.hiddenGroups),
    hiddenWorkers: parseKeySet(f.hiddenWorkers),
    shownGroups: parseKeySet(f.shownGroups),
    shownInternal: parseKeySet(f.shownInternal),
  }
}

/** Write the prefs back into a (copied) console-config value. */
export function withSpanFilters(
  configValue: Record<string, unknown>,
  prefs: SpanFilterPrefs,
): Record<string, unknown> {
  const traces =
    configValue.traces && typeof configValue.traces === 'object'
      ? { ...(configValue.traces as Record<string, unknown>) }
      : {}
  traces.spanFilters = {
    hiddenGroups: [...prefs.hiddenGroups].sort(),
    hiddenWorkers: [...prefs.hiddenWorkers].sort(),
    shownGroups: [...prefs.shownGroups].sort(),
    shownInternal: [...prefs.shownInternal].sort(),
  }
  return { ...configValue, traces }
}

/**
 * Fold producer-default-hidden groups into the user prefs: a default is
 * hidden unless the user unhid it (`shownGroups`); anything the user hid
 * directly stays hidden regardless.
 */
export function effectiveSpanFilters(
  prefs: SpanFilterPrefs,
  producerHidden: ReadonlySet<string>,
): SpanFilterSelection {
  if (producerHidden.size === 0) return prefs
  const hiddenGroups = new Set(prefs.hiddenGroups)
  for (const key of producerHidden) {
    if (!prefs.shownGroups.has(key)) hiddenGroups.add(key)
  }
  return {
    hiddenGroups,
    hiddenWorkers: prefs.hiddenWorkers,
    shownInternal: prefs.shownInternal,
  }
}
