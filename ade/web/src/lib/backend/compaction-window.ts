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
  /** The compaction entry itself; a null boundary opens the window after it. */
  entryId: string
  /** The persisted summary, or null when the entry carried none. */
  summary: string | null
  /**
   * First entry of the verbatim tail; null when everything before the entry
   * was summarised; undefined when the record carries no usable boundary.
   */
  tailStartEntryId: string | null | undefined
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
      entryId: item.entry_id,
      summary: typeof data.summary === 'string' ? data.summary : null,
      tailStartEntryId:
        typeof data.tail_start_entry_id === 'string' ||
        data.tail_start_entry_id === null
          ? data.tail_start_entry_id
          : undefined,
    }
  }
  return anchor
}

/**
 * Model-bound message entries of the candidate window — the same one the
 * harness assembles for the next turn. It opens at the anchor's
 * `tailStartEntryId`, or right after the anchor entry when that is null
 * (everything before it was summarised). A never-compacted session, a
 * record without a summary or a usable boundary, or a boundary no longer on
 * the path (a hand-written entry, a session forked before fork rewrote the
 * anchor), means the whole path rather than compacting nothing.
 */
export function compactionWindow(
  items: readonly TranscriptItem[],
  anchor: CompactionAnchor | null,
): WindowEntry[] {
  let start = 0
  if (anchor && anchor.summary !== null) {
    const tail = anchor.tailStartEntryId
    if (tail === null) {
      start = items.findIndex((item) => item.entry_id === anchor.entryId) + 1
    } else if (typeof tail === 'string') {
      start = Math.max(
        0,
        items.findIndex((item) => item.entry_id === tail),
      )
    }
  }
  const out: WindowEntry[] = []
  for (const item of items.slice(start)) {
    const message = item.message
    if (!message || message.role === 'custom') continue
    out.push({ entry_id: item.entry_id, message })
  }
  return out
}
