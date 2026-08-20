import type { MessageListRow } from '@/components/chat/function-trigger-groups'
import type { SystemMessage, UserMessage } from '@/types/chat'

export interface TriggerActivityRow {
  kind: 'trigger-activity'
  id: string
  message: SystemMessage
  notification?: UserMessage
}

export type TriggerAwareMessageListRow = MessageListRow | TriggerActivityRow

type PairMember =
  | { side: 'notification'; key: string; message: UserMessage }
  | { side: 'record'; key: string; message: SystemMessage }

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
    const key = notificationPairKey(message.id)
    return key ? { side: 'notification', key, message } : null
  }

  if (
    message.role === 'system' &&
    message.kind === 'trigger-fired' &&
    message.trigger
  ) {
    const key = recordPairKey(message.id)
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
  if (complete.size === 0) return [...rows]

  const output: TriggerAwareMessageListRow[] = []
  rows.forEach((row, index) => {
    const member = triggerPairMember(row)
    if (!member) {
      output.push(row)
      return
    }
    const pair = complete.get(member.key)
    if (!pair) {
      output.push(row)
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
