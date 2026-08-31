import type { MessageListRow } from '@/components/chat/function-trigger-groups'
import type {
  AssistantMessage,
  FunctionTriggerMessage,
  Message,
  SystemMessage,
  UserMessage,
} from '@/types/chat'

export interface TriggerActivityRow {
  kind: 'trigger-activity'
  id: string
  message: SystemMessage | UserMessage
  notification?: UserMessage
}

export type TriggerAwareMessageListRow = MessageListRow | TriggerActivityRow

export type TimelineActivityItem =
  | { kind: 'function-trigger'; id: string; message: FunctionTriggerMessage }
  | TriggerActivityRow

export interface TimelineActivityGroupRow {
  kind: 'activity-group'
  id: string
  items: TimelineActivityItem[]
  summary?: AssistantMessage
}

/**
 * React-facing identity for an activity. Durable transcript ids may replace
 * optimistic ids during reconciliation; the harness function-call id remains
 * stable across that handoff and is therefore the preferred presentation key.
 */
export function timelineActivityPresentationKey(
  item: TimelineActivityItem,
): string {
  if (item.kind === 'function-trigger') {
    return item.message.functionTriggerId ?? item.id
  }
  return item.id
}

export type TimelineMessageListRow =
  | Extract<MessageListRow, { kind: 'message' }>
  | TimelineActivityGroupRow

type TimelineMessageRow = Extract<MessageListRow, { kind: 'message' }>

type PairMember =
  | { side: 'notification'; key: string; message: UserMessage }
  | { side: 'record'; key: string; message: SystemMessage }

/**
 * Canonical identity shared by the two transcript entries of one trigger
 * delivery. It is also suitable as the turn clock key while those entries
 * arrive out of order.
 */
export function triggerActivityPairKey(message: Message): string | null {
  if (message.role === 'user' && message.notification) {
    return notificationPairKey(message.id)
  }
  if (
    message.role === 'system' &&
    message.kind === 'trigger-fired' &&
    message.trigger
  ) {
    return recordPairKey(message.id)
  }
  return null
}

/**
 * Match the two durable entries that represent one Harness wake. The IDs are
 * part of the persistence contract and include the fire ordinal, so recurring
 * bindings cannot be accidentally coalesced. Legacy `e_notify_*` transcripts
 * use the same suffix as their `e_trigfired_*` record.
 */
export function triggerPairMember(row: MessageListRow): PairMember | null {
  if (row.kind !== 'message') return null
  const message = row.message

  if (message.role === 'user' && message.notification) {
    const key = triggerActivityPairKey(message)
    return key ? { side: 'notification', key, message } : null
  }

  if (
    message.role === 'system' &&
    message.kind === 'trigger-fired' &&
    message.trigger
  ) {
    const key = triggerActivityPairKey(message)
    return key ? { side: 'record', key, message } : null
  }

  return null
}

function notificationPairKey(entryId: string): string | null {
  const fire = /^e_fire_(.+_\d+)$/.exec(entryId)
  if (fire?.[1]) return `fire:${fire[1]}`

  const legacy = /^e_notify_(.+)$/.exec(entryId)
  if (legacy?.[1]) return `fire:${legacy[1]}`

  const expiry = /^e_expire_(.+)$/.exec(entryId)
  if (expiry?.[1]) return `expiry:${expiry[1]}`

  const stale = /^e_stalespawn_(.+)$/.exec(entryId)
  if (stale?.[1]) return `stale:${stale[1]}`

  return null
}

function recordPairKey(entryId: string): string | null {
  const fire = /^e_trigfired_(.+)$/.exec(entryId)
  if (fire?.[1]) return `fire:${fire[1]}`

  const expiry = /^e_trigexpired_(.+)$/.exec(entryId)
  if (expiry?.[1]) return `expiry:${expiry[1]}`

  const stale = /^e_trigstale_(.+)$/.exec(entryId)
  if (stale?.[1]) return `stale:${stale[1]}`

  return null
}

/**
 * Collapse an exact notification/record pair into one presentation row after
 * function-call grouping has run. Keeping this second prevents a hidden wake
 * message from joining call batches that were separated in the transcript.
 * Unpaired entries pass through unchanged for debuggability and compatibility.
 */
export function triggerActivityRows(
  rows: readonly MessageListRow[],
): TriggerAwareMessageListRow[] {
  const pairs = new Map<
    string,
    { notification?: UserMessage; record?: SystemMessage; firstIndex: number }
  >()

  rows.forEach((row, index) => {
    const member = triggerPairMember(row)
    if (!member) return
    const pair = pairs.get(member.key) ?? { firstIndex: index }
    if (member.side === 'notification') pair.notification ??= member.message
    else pair.record ??= member.message
    pairs.set(member.key, pair)
  })

  const complete = new Map(
    [...pairs].filter(
      (
        pair,
      ): pair is [
        string,
        {
          notification: UserMessage
          record: SystemMessage
          firstIndex: number
        },
      ] => Boolean(pair[1].notification && pair[1].record),
    ),
  )
  const output: TriggerAwareMessageListRow[] = []
  rows.forEach((row, index) => {
    const member = triggerPairMember(row)
    if (!member) {
      output.push(row)
      return
    }
    const pair = complete.get(member.key)
    if (!pair) {
      // Give each half of a wake the same activity-group shape it will have
      // after pairing. Late durable data then updates the existing
      // TriggerActivityCard instead of reparenting and remounting it.
      output.push({
        kind: 'trigger-activity',
        id: member.message.id,
        message: member.message,
        ...(member.side === 'notification'
          ? { notification: member.message }
          : {}),
      })
      return
    }
    if (index !== pair.firstIndex) return
    output.push({
      kind: 'trigger-activity',
      // Preserve the first durable row's React identity when its counterpart
      // arrives later, so open details, selected tabs, and focus survive.
      id: member.message.id,
      message: pair.record,
      notification: pair.notification,
    })
  })
  return output
}

/**
 * Put paired trigger fires and function calls in the same collapsible phase.
 * The first pass has already grouped contiguous calls; this pass treats those
 * groups and trigger activities as one stream, stopping at ordinary prose or
 * at an intermediate assistant summary. Visible thoughts are interstitials:
 * they keep their side of the stable activity group while it absorbs later
 * calls. A fire immediately before the calls it woke therefore participates in
 * the existing "show all" behavior without remounting those calls on handoff.
 */
export function groupTimelineActivities(
  rows: readonly TriggerAwareMessageListRow[],
): TimelineMessageListRow[] {
  const output: TimelineMessageListRow[] = []
  let items: TimelineActivityItem[] = []
  let leadingThoughts: TimelineMessageRow[] = []
  let trailingThoughts: TimelineMessageRow[] = []

  const flush = (summary?: AssistantMessage) => {
    output.push(...leadingThoughts)
    if (items.length > 0) {
      output.push({
        kind: 'activity-group',
        // Match the first durable row's presentation identity. A standalone
        // wake that later becomes a paired/grouped activity then keeps the same
        // outer React key instead of replaying its mount transition.
        id: timelineActivityPresentationKey(items[0]),
        items,
        ...(summary ? { summary } : {}),
      })
    }
    output.push(...trailingThoughts)
    items = []
    leadingThoughts = []
    trailingThoughts = []
  }

  for (const row of rows) {
    if (row.kind === 'message' && row.message.role === 'thought') {
      const thoughts = items.length === 0 ? leadingThoughts : trailingThoughts
      thoughts.push(row)
      continue
    }
    if (row.kind === 'trigger-activity') {
      items.push(row)
      continue
    }
    if (row.kind === 'function-trigger-group') {
      items.push(
        ...row.calls.map(
          (message): TimelineActivityItem => ({
            kind: 'function-trigger',
            id: message.id,
            message,
          }),
        ),
      )
      // A fired activity can own the calls that follow it, but a later fire
      // starts a new phase. Closing here keeps that directional relationship
      // and prevents two groups from merging when an ephemeral thought between
      // them leaves the transcript.
      if (row.summary && trailingThoughts.length > 0) {
        flush()
        output.push({ kind: 'message', message: row.summary })
      } else {
        flush(row.summary)
      }
      continue
    }
    flush()
    output.push(row)
  }

  flush()
  return output
}

/** Activity items that stay visible while a phase is collapsed. */
export function collapsedTimelineActivities(
  items: readonly TimelineActivityItem[],
  hasPersistentDisplay: (call: FunctionTriggerMessage) => boolean,
): TimelineActivityItem[] {
  const lastIndex = items.length - 1
  return items.filter((item, index) => {
    if (index === lastIndex) return true
    if (item.kind !== 'function-trigger') return false
    const call = item.message
    return (
      call.running === true ||
      call.pendingApproval === true ||
      hasPersistentDisplay(call)
    )
  })
}
