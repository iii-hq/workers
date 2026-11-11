import { describe, expect, it } from 'vitest'
import type { SessionObserverInfo } from '@/lib/backend/observers'
import type { SystemMessage } from '@/types/chat'
import {
  cronInWords,
  deliveryLabel,
  eventInWords,
  lifecycleNote,
  observerTrigger,
  whenInWords,
} from './ObserverDetailDialog'

function observer(
  overrides: Partial<SessionObserverInfo>,
): SessionObserverInfo {
  return {
    id: 'obs_1',
    functionId: 'state::on-changed',
    delivery: { kind: 'notify' },
    once: true,
    ...overrides,
  }
}

describe('deliveryLabel', () => {
  it('labels a wake and a call from the delivery data alone', () => {
    expect(deliveryLabel(observer({ delivery: { kind: 'notify' } }))).toBe(
      'notifies this chat',
    )
    expect(
      deliveryLabel(
        observer({ delivery: { kind: 'call', functionId: 'state::set' } }),
      ),
    ).toBe('calls state::set')
  })
})

describe('lifecycleNote', () => {
  it('labels a consumed once observer as automatic', () => {
    expect(
      lifecycleNote(
        observer({
          fired: true,
          outcome: 'delivered',
          retirementReason: 'once_consumed',
        }),
      ),
    ).toBe('once · consumed automatically')
  })

  it('keeps expiry, removal, and invalidation distinct', () => {
    expect(
      lifecycleNote(observer({ fired: true, retirementReason: 'expired' })),
    ).toBe('expired')
    expect(
      lifecycleNote(
        observer({ fired: true, retirementReason: 'unregistered' }),
      ),
    ).toBe('removed')
    expect(
      lifecycleNote(observer({ fired: true, retirementReason: 'invalidated' })),
    ).toBe('invalidated')
  })

  it('labels fire-budget and exhausted retirement separately', () => {
    expect(
      lifecycleNote(observer({ fired: true, retirementReason: 'max_fires' })),
    ).toBe('fire limit reached')
    expect(
      lifecycleNote(observer({ fired: true, retirementReason: 'exhausted' })),
    ).toBe('exhausted')
  })

  it('uses lifecycle outcomes when the matching reason is absent', () => {
    expect(lifecycleNote(observer({ fired: true, outcome: 'expired' }))).toBe(
      'expired',
    )
    expect(
      lifecycleNote(observer({ fired: true, outcome: 'unregistered' })),
    ).toBe('removed')
    expect(
      lifecycleNote(observer({ fired: true, outcome: 'invalidated' })),
    ).toBe('invalidated')
  })

  it('does not claim a manual removal for a legacy retired record', () => {
    const note = lifecycleNote(observer({ fired: true, once: true }))
    expect(note).toBe('retired')
    expect(note).not.toContain('removed')
    expect(note).not.toContain('consumed')
  })

  it('joins once, fires, budget, and deadline from the row data', () => {
    const note = lifecycleNote(
      observer({
        once: false,
        fires: 3,
        maxFires: 5,
        expiresAt: 1_800_000_000_000,
      }),
    )
    expect(note).toContain('3 fires')
    expect(note).toContain('max 5')
    expect(note).toContain('until ')
  })

  it('is null when nothing lifecycle-worthy is set', () => {
    expect(lifecycleNote(observer({ once: false, fires: 0 }))).toBeNull()
  })
})

describe('observer wording', () => {
  it('reads common cron schedules', () => {
    expect(cronInWords('*/5 * * * * * *')).toBe('every 5 seconds')
    expect(cronInWords('0 */15 * * * * *')).toBe('every 15 minutes')
    expect(cronInWords('0 0 * * * * *')).toBe('every hour')
    expect(cronInWords('0 30 9 * * * *')).toBe('every day at 09:30')
    expect(cronInWords('0 30 9 * * MON *')).toBeNull()
  })

  it('names the trigger of the sources it knows', () => {
    expect(
      observerTrigger(
        observer({
          functionId: 'state::on-changed',
          filters: { 'input.scope': { $eq: 'orders' } },
        }),
      ),
    ).toBe('When orders changes')
    expect(
      observerTrigger(
        observer({ functionId: 'harness::on-timer', args: { in_ms: 45000 } }),
      ),
    ).toBe('Once, after 45 seconds')
    expect(observerTrigger(observer({ functionId: 'kanban::on-change' }))).toBe(
      'When kanban::on-change runs',
    )
  })

  it('says today and tomorrow', () => {
    const now = new Date('2026-11-11T10:00:00').getTime()
    expect(whenInWords(new Date('2026-11-11T18:43:00').getTime(), now)).toMatch(
      /^today at /,
    )
    expect(whenInWords(new Date('2026-11-12T09:30:00').getTime(), now)).toMatch(
      /^tomorrow at /,
    )
  })

  it('never shows an expiry as the thing it watched for', () => {
    const event = (id: string): SystemMessage => ({
      id,
      role: 'system',
      kind: 'observer-fired',
      content: 'x',
      observer: {
        observer_id: 'sub_1',
        target: 'harness::send',
        action: 'The build finished',
        once: false,
        retired: true,
        fired_at: 1,
      },
      createdAt: 1,
    })
    expect(eventInWords(event('e_obsexpired_sub_1')).text).toBe(
      'Time ran out, stopped watching',
    )
    expect(eventInWords(event('e_obsfired_sub_1_1')).text).toBe(
      'The build finished',
    )
  })
})
