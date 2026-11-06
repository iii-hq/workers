import { describe, expect, it } from 'vitest'
import type { SessionObserverInfo } from '@/lib/backend/observers'
import { deliveryLabel, lifecycleNote } from './ObserverDetailDialog'

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
