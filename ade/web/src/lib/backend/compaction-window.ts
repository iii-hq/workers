/**
 * The `/compact` window — what the console hands to `context::compact`.
 *
 * Mirrors the harness's `assemble_context` (harness/src/turn_loop.rs): the
 * LATEST `compaction` custom entry anchors the summariser (`summary` becomes
 * `previous_summary`, so it is updated in place instead of re-summarised from
 * scratch) and opens the candidate window at its `tail_start_entry_id`.
 * A `model_notice` custom entry is text the model was shown, so it rides as
 * the user message the harness replays (harness/src/window.rs), and every
 * `message_order` record is applied before the cut, so the window is in the
 * order the model saw it; other custom entries and `role: 'custom'` messages
 * are never model-bound and are skipped. Pure functions, no I/O.
 */

import {
  COMPACTION_CUSTOM_TYPE,
  MODEL_NOTICE_CUSTOM_TYPE,
} from '@/lib/sessions/entry-mapper'
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

const MESSAGE_ORDER_CUSTOM_TYPE = 'message_order'

/** A persisted reorder: `moved` sits right after `after` (window.rs `MessageOrder`). */
interface MessageOrder {
  after: string
  moved: string[]
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
 * anchor), means the whole path rather than compacting nothing. Like the
 * harness's `window::build`, every `message_order` on the path is applied
 * first and the cut is made in that model order: at the start entry when it
 * is model-bound, else at the first model-bound entry logged from there on.
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
  const list: WindowEntry[] = []
  const loggedAt = new Map<string, number>()
  const orders: MessageOrder[] = []
  items.forEach((item, index) => {
    const order = item.message ? null : messageOrder(item.custom)
    if (order) orders.push(order)
    const message = item.message ?? noticeMessage(item.custom)
    if (!message || message.role === 'custom') return
    loggedAt.set(item.entry_id, index)
    list.push({ entry_id: item.entry_id, message })
  })
  for (const order of orders) applyOrder(list, order)
  let cut = list.findIndex((e) => e.entry_id === items[start]?.entry_id)
  if (cut < 0) {
    cut = list.findIndex((e) => (loggedAt.get(e.entry_id) ?? -1) >= start)
  }
  return cut < 0 ? [] : list.slice(cut)
}

/** A `message_order` record's order, as window.rs `MessageOrder::from_data` reads it. */
function messageOrder(custom: TranscriptItem['custom']): MessageOrder | null {
  if (custom?.custom_type !== MESSAGE_ORDER_CUSTOM_TYPE) return null
  const data = (custom.data ?? {}) as { after?: unknown; moved?: unknown }
  if (typeof data.after !== 'string' || !Array.isArray(data.moved)) return null
  const moved = data.moved.filter((id): id is string => typeof id === 'string')
  return moved.length > 0 ? { after: data.after, moved } : null
}

/**
 * Move `order.moved` right after `order.after`, as window.rs `apply_order`
 * does: a no-op when the anchor is not in `list`; moved ids not in `list`, and
 * the anchor itself (the log is caller-writable), are skipped.
 */
function applyOrder(list: WindowEntry[], order: MessageOrder): void {
  if (!list.some((e) => e.entry_id === order.after)) return
  const moved: WindowEntry[] = []
  for (const id of order.moved) {
    if (id === order.after) continue
    const pos = list.findIndex((e) => e.entry_id === id)
    if (pos >= 0) moved.push(...list.splice(pos, 1))
  }
  const anchor = list.findIndex((e) => e.entry_id === order.after)
  list.splice(anchor + 1, 0, ...moved)
}

/**
 * The user message a `model_notice` entry replays, as the harness's
 * `notice_message` builds it: the stored message, else one text block of its
 * text.
 */
function noticeMessage(custom: TranscriptItem['custom']): AgentMessage | null {
  if (custom?.custom_type !== MODEL_NOTICE_CUSTOM_TYPE) return null
  const data = (custom.data ?? {}) as { message?: unknown; text?: unknown }
  const stored = data.message as Partial<AgentMessage> | undefined
  if (stored?.role === 'user' && Array.isArray(stored.content)) {
    return {
      role: 'user',
      content: stored.content,
      timestamp: stored.timestamp ?? 0,
    }
  }
  if (typeof data.text !== 'string' || !data.text) return null
  return {
    role: 'user',
    content: [{ type: 'text', text: data.text }],
    timestamp: 0,
  }
}
