/**
 * The `/compact` window — what the console hands to `context::compact`.
 *
 * Mirrors the harness's `assemble_context` (harness/src/turn_loop.rs): the
 * LATEST `compaction` custom entry anchors the summariser (`summary` becomes
 * `previous_summary`, so it is updated in place instead of re-summarised from
 * scratch) and opens the candidate window at its `tail_start_entry_id`.
 * Compaction entries and `role: 'custom'` messages are never model-bound, so
 * they are skipped. Pure functions, no I/O.
 */

import { COMPACTION_CUSTOM_TYPE } from '@/lib/sessions/entry-mapper'
import type { AgentMessage, TranscriptItem } from '@/lib/sessions/types'

export interface CompactionAnchor {
  /** The persisted summary, or null when the entry carried none. */
  summary: string | null
  /** First entry of the verbatim tail; null when everything was summarised. */
  tailStartEntryId: string | null
}

/** One model-bound row of the window: the entry id keeps `tail_start_index` mappable. */
export interface WindowEntry {
  entry_id: string
  message: AgentMessage
}

/**
 * The latest compaction anchor on the path, or null when the session was
 * never compacted. Later entries win, the way the harness reads them.
 */
export function latestCompactionAnchor(
  items: readonly TranscriptItem[],
): CompactionAnchor | null {
  let anchor: CompactionAnchor | null = null
  for (const item of items) {
    if (item.custom?.custom_type !== COMPACTION_CUSTOM_TYPE) continue
    const data =
      item.custom.data && typeof item.custom.data === 'object'
        ? (item.custom.data as Record<string, unknown>)
        : {}
    anchor = {
      summary: typeof data.summary === 'string' ? data.summary : null,
      tailStartEntryId:
        typeof data.tail_start_entry_id === 'string'
          ? data.tail_start_entry_id
          : null,
    }
  }
  return anchor
}

/**
 * Model-bound message entries from `tailStartEntryId` onward — the same
 * candidate window the harness assembles for the next turn. `null` (never
 * compacted, or everything was summarised) means the whole path. A boundary
 * id that is no longer on the path (fork, deletion) falls back to the whole
 * path rather than compacting nothing.
 */
export function compactionWindow(
  items: readonly TranscriptItem[],
  tailStartEntryId: string | null,
): WindowEntry[] {
  const boundaryOnPath =
    tailStartEntryId !== null &&
    items.some((item) => item.entry_id === tailStartEntryId)
  let started = !boundaryOnPath
  const out: WindowEntry[] = []
  for (const item of items) {
    if (!started && item.entry_id === tailStartEntryId) started = true
    if (!started) continue
    const message = item.message
    if (!message || message.role === 'custom') continue
    out.push({ entry_id: item.entry_id, message })
  }
  return out
}
