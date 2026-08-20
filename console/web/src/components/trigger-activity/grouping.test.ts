import { describe, expect, it } from 'vitest'
import type { MessageListRow } from '@/components/chat/function-trigger-groups'
import type { SystemMessage, TriggerFiredData, UserMessage } from '@/types/chat'
import { triggerActivityRows } from './grouping'

function notification(id: string): UserMessage {
  return {
    id,
    role: 'user',
    content: '[notification] build: {"ok":true}',
    createdAt: 1,
    notification: true,
  }
}

function fired(id: string, subscriptionId = 'sub_1'): SystemMessage {
  const trigger: TriggerFiredData = {
    subscription_id: subscriptionId,
    target: 'harness::send',
    once: false,
    retired: false,
    fired_at: 1,
  }
  return {
    id,
    role: 'system',
    kind: 'trigger-fired',
    content: 'build · notified this chat',
    trigger,
    createdAt: 1,
  }
}

const row = (message: UserMessage | SystemMessage): MessageListRow => ({
  kind: 'message',
  message,
})

describe('triggerActivityRows', () => {
  it('collapses one current wake pair at the first member position', () => {
    const rows = triggerActivityRows([
      row(notification('e_fire_sub_1_3')),
      row(fired('e_trigfired_sub_1_3')),
    ])
    expect(rows).toHaveLength(1)
    expect(rows[0]).toMatchObject({
      kind: 'trigger-activity',
      id: 'e_fire_sub_1_3',
      message: { id: 'e_trigfired_sub_1_3' },
      notification: { id: 'e_fire_sub_1_3' },
    })
  })

  it('matches records that arrive before notifications', () => {
    const rows = triggerActivityRows([
      row(fired('e_trigfired_sub_1_3')),
      row(notification('e_fire_sub_1_3')),
    ])
    expect(rows).toHaveLength(1)
    expect(rows[0]).toMatchObject({
      kind: 'trigger-activity',
      id: 'e_trigfired_sub_1_3',
    })
  })

  it('keeps recurring ordinals as separate activities', () => {
    const rows = triggerActivityRows([
      row(notification('e_fire_sub_1_0')),
      row(fired('e_trigfired_sub_1_0')),
      row(notification('e_fire_sub_1_1')),
      row(fired('e_trigfired_sub_1_1')),
    ])
    expect(rows).toHaveLength(2)
    expect(rows.every((item) => item.kind === 'trigger-activity')).toBe(true)
  })

  it('collapses expiry and invalidation pairs', () => {
    const rows = triggerActivityRows([
      row(notification('e_expire_sub_1')),
      row(fired('e_trigexpired_sub_1')),
      row(notification('e_stalespawn_sub_2')),
      row(fired('e_trigstale_sub_2', 'sub_2')),
    ])
    expect(rows).toHaveLength(2)
    expect(rows.map((item) => item.kind)).toEqual([
      'trigger-activity',
      'trigger-activity',
    ])
  })

  it('supports the historical notify id scheme', () => {
    const rows = triggerActivityRows([
      row(notification('e_notify_sub_1_7')),
      row(fired('e_trigfired_sub_1_7')),
    ])
    expect(rows).toHaveLength(1)
    expect(rows[0]?.kind).toBe('trigger-activity')
  })

  it('leaves unpaired entries visible', () => {
    const rows = [
      row(notification('e_fire_sub_1_0')),
      row(fired('e_trigfired_sub_2_0', 'sub_2')),
    ]
    expect(triggerActivityRows(rows)).toEqual(rows)
  })

  it('does not pair matching labels with different durable ids', () => {
    const rows = [
      row(notification('e_fire_sub_1_0')),
      row(fired('e_trigfired_sub_1_1')),
    ]
    expect(triggerActivityRows(rows)).toEqual(rows)
  })
})
