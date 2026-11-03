import { describe, expect, it } from 'vitest'
import type { MessageListRow } from '@/components/chat/function-call-groups'
import type {
  FunctionCallMessage,
  ObserverFiredData,
  SystemMessage,
  ThoughtMessage,
  UserMessage,
} from '@/types/chat'
import {
  collapsedTimelineActivities,
  groupTimelineActivities,
  observerActivityRows,
  timelineActivityPresentationKey,
} from './grouping'

function notification(id: string): UserMessage {
  return {
    id,
    role: 'user',
    content: '[notification] build: {"ok":true}',
    createdAt: 1,
    notification: true,
  }
}

function fired(id: string, observerId = 'obs_1'): SystemMessage {
  const observer: ObserverFiredData = {
    observer_id: observerId,
    target: 'harness::send',
    once: false,
    retired: false,
    fired_at: 1,
  }
  return {
    id,
    role: 'system',
    kind: 'observer-fired',
    content: 'build · notified this chat',
    observer,
    createdAt: 1,
  }
}

const row = (message: UserMessage | SystemMessage): MessageListRow => ({
  kind: 'message',
  message,
})

describe('observerActivityRows', () => {
  it('collapses one current wake pair at the first member position', () => {
    const rows = observerActivityRows([
      row(notification('e_fire_obs_1_3')),
      row(fired('e_obsfired_obs_1_3')),
    ])
    expect(rows).toHaveLength(1)
    expect(rows[0]).toMatchObject({
      kind: 'observer-activity',
      id: 'e_fire_obs_1_3',
      message: { id: 'e_obsfired_obs_1_3' },
      notification: { id: 'e_fire_obs_1_3' },
    })
  })

  it('matches records that arrive before notifications', () => {
    const rows = observerActivityRows([
      row(fired('e_obsfired_obs_1_3')),
      row(notification('e_fire_obs_1_3')),
    ])
    expect(rows).toHaveLength(1)
    expect(rows[0]).toMatchObject({
      kind: 'observer-activity',
      id: 'e_obsfired_obs_1_3',
    })
  })

  it('keeps recurring ordinals as separate activities', () => {
    const rows = observerActivityRows([
      row(notification('e_fire_obs_1_0')),
      row(fired('e_obsfired_obs_1_0')),
      row(notification('e_fire_obs_1_1')),
      row(fired('e_obsfired_obs_1_1')),
    ])
    expect(rows).toHaveLength(2)
    expect(rows.every((item) => item.kind === 'observer-activity')).toBe(true)
  })

  it('collapses expiry and invalidation pairs', () => {
    const rows = observerActivityRows([
      row(notification('e_expire_obs_1')),
      row(fired('e_obsexpired_obs_1')),
      row(notification('e_stalespawn_obs_2')),
      row(fired('e_obsstale_obs_2', 'obs_2')),
    ])
    expect(rows).toHaveLength(2)
    expect(rows.map((item) => item.kind)).toEqual([
      'observer-activity',
      'observer-activity',
    ])
  })

  it('supports the historical notify id scheme', () => {
    const rows = observerActivityRows([
      row(notification('e_notify_obs_1_7')),
      row(fired('e_obsfired_obs_1_7')),
    ])
    expect(rows).toHaveLength(1)
    expect(rows[0]?.kind).toBe('observer-activity')
  })

  it('normalizes unpaired wake halves into stable activity rows', () => {
    const rows = [
      row(notification('e_fire_obs_1_0')),
      row(fired('e_obsfired_obs_2_0', 'obs_2')),
    ]
    expect(observerActivityRows(rows)).toMatchObject([
      { kind: 'observer-activity', id: 'e_fire_obs_1_0' },
      { kind: 'observer-activity', id: 'e_obsfired_obs_2_0' },
    ])
  })

  it('does not pair matching labels with different durable ids', () => {
    const rows = [
      row(notification('e_fire_obs_1_0')),
      row(fired('e_obsfired_obs_1_1')),
    ]
    expect(observerActivityRows(rows)).toMatchObject([
      { kind: 'observer-activity', id: 'e_fire_obs_1_0' },
      { kind: 'observer-activity', id: 'e_obsfired_obs_1_1' },
    ])
  })
})

describe('groupTimelineActivities', () => {
  const call = (id: string): FunctionCallMessage => ({
    id,
    role: 'function-call',
    functionId: 'state::get',
    input: {},
    output: {},
    createdAt: 1,
  })

  it('groups a fired event with the function calls it woke', () => {
    const paired = observerActivityRows([
      row(notification('e_fire_obs_1_3')),
      row(fired('e_obsfired_obs_1_3')),
      {
        kind: 'function-call-group',
        id: 'function-call-group:call-1',
        calls: [call('call-1'), call('call-2')],
      },
    ])
    const grouped = groupTimelineActivities(paired)
    expect(grouped).toHaveLength(1)
    expect(grouped[0]).toMatchObject({
      kind: 'activity-group',
      id: 'e_fire_obs_1_3',
      items: [
        { kind: 'observer-activity' },
        { kind: 'function-call', id: 'call-1' },
        { kind: 'function-call', id: 'call-2' },
      ],
    })
  })

  it('uses the harness call id as stable React presentation identity', () => {
    const message = {
      ...call('optimistic-message-id'),
      functionCallId: 'harness-call-id',
    }
    expect(
      timelineActivityPresentationKey({
        kind: 'function-call',
        id: message.id,
        message,
      }),
    ).toBe('harness-call-id')
  })

  it('keeps the fire with its calls when the durable record arrives later', () => {
    const paired = observerActivityRows([
      row(notification('e_fire_obs_1_3')),
      {
        kind: 'function-call-group',
        id: 'function-call-group:call-1',
        calls: [call('call-1')],
      },
      row(fired('e_obsfired_obs_1_3')),
    ])
    const grouped = groupTimelineActivities(paired)
    expect(grouped).toHaveLength(1)
    expect(grouped[0]).toMatchObject({
      kind: 'activity-group',
      items: [
        { kind: 'observer-activity' },
        { kind: 'function-call', id: 'call-1' },
      ],
    })
  })

  it('starts a new phase when a fired activity follows existing calls', () => {
    const grouped = groupTimelineActivities([
      {
        kind: 'function-call-group',
        id: 'function-call-group:call-1',
        calls: [call('call-1')],
      },
      {
        kind: 'observer-activity',
        id: 'fire-2',
        message: fired('e_obsfired_obs_1_2'),
      },
      {
        kind: 'function-call-group',
        id: 'function-call-group:call-2',
        calls: [call('call-2')],
      },
    ])

    expect(grouped).toHaveLength(2)
    expect(grouped[0]).toMatchObject({
      kind: 'activity-group',
      id: 'call-1',
      items: [{ kind: 'function-call', id: 'call-1' }],
    })
    expect(grouped[1]).toMatchObject({
      kind: 'activity-group',
      id: 'fire-2',
      items: [
        { kind: 'observer-activity', id: 'fire-2' },
        { kind: 'function-call', id: 'call-2' },
      ],
    })
  })

  it('keeps a wake group stable while thinking hands off to its first call', () => {
    const thought: ThoughtMessage = {
      id: 'thought-1',
      role: 'thought',
      content: 'Choosing the next tool',
      streaming: true,
      durationMs: 0,
      createdAt: 1,
    }
    const wake = {
      kind: 'observer-activity' as const,
      id: 'fire-1',
      message: fired('e_obsfired_obs_1_1'),
    }
    const calls = {
      kind: 'function-call-group' as const,
      id: 'function-call-group:call-1',
      calls: [call('call-1')],
    }

    const visible = groupTimelineActivities([
      wake,
      { kind: 'message', message: thought },
      calls,
    ])
    const hidden = groupTimelineActivities([wake, calls])

    expect(visible.map((row) => row.kind)).toEqual([
      'activity-group',
      'message',
    ])
    expect(visible[0]).toMatchObject({
      id: 'fire-1',
      items: [
        { kind: 'observer-activity', id: 'fire-1' },
        { kind: 'function-call', id: 'call-1' },
      ],
    })
    expect(visible[1]).toEqual({ kind: 'message', message: thought })
    expect(hidden).toHaveLength(1)
    expect(hidden[0]).toMatchObject({
      id: 'fire-1',
      items: [
        { kind: 'observer-activity', id: 'fire-1' },
        { kind: 'function-call', id: 'call-1' },
      ],
    })
  })

  it('hides an earlier fire behind the same show-all rule as earlier calls', () => {
    const items = [
      {
        kind: 'observer-activity' as const,
        id: 'fire-1',
        message: fired('e_obsfired_obs_1_1'),
      },
      {
        kind: 'function-call' as const,
        id: 'call-1',
        message: call('call-1'),
      },
    ]
    expect(collapsedTimelineActivities(items, () => false)).toEqual([items[1]])
  })
})
