// Pure helpers for the session rail.
//
// `groupByDate` partitions the rail into Today / Yesterday / Earlier sections
// keyed off `updated_at_ms`. Time of day is normalized to start-of-day in the
// host timezone so a session updated at 23:59 yesterday isn't bucketed with
// today's work just because Date.now() is close.
//
// `truncatePath` is the subtitle helper: long absolute paths get clipped from
// the LEFT (the meaningful part is the tail, e.g. `…/web/src`), prefixed with
// `…/` so the user knows there's more above.

import type { SessionRow } from "./types";

export type DateGroup = "today" | "yesterday" | "earlier";

export interface GroupedSessions {
  today: SessionRow[];
  yesterday: SessionRow[];
  earlier: SessionRow[];
}

const DAY_MS = 24 * 60 * 60 * 1000;

function startOfDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/**
 * Bucket sessions into today / yesterday / earlier based on `updated_at_ms`.
 * Caller passes `now` for deterministic tests (defaults to `Date.now()`).
 * Order within each bucket is preserved from input.
 */
export function groupByDate(
  rows: SessionRow[],
  now: number = Date.now(),
): GroupedSessions {
  const today = startOfDay(now);
  const yesterday = today - DAY_MS;
  const out: GroupedSessions = { today: [], yesterday: [], earlier: [] };
  for (const row of rows) {
    const day = startOfDay(row.updated_at_ms);
    if (day >= today) out.today.push(row);
    else if (day >= yesterday) out.yesterday.push(row);
    else out.earlier.push(row);
  }
  return out;
}

/**
 * Truncate from the LEFT so the tail (most informative segment) survives.
 * Returns `path` unchanged if its length already fits within `max`.
 * Otherwise returns `…/<tail>` where `<tail>` fits within `max - 2` chars.
 *
 * `max` is the total target length including the `…/` prefix.
 */
export function truncatePath(path: string, max: number): string {
  if (path.length <= max) return path;
  if (max <= 2) return path.slice(-Math.max(1, max));
  const tailLen = max - 2; // reserve room for `…/`
  const tail = path.slice(-tailLen);
  return `…/${tail}`;
}
