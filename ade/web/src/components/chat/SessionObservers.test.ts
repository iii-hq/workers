import { describe, expect, it } from 'vitest'
import type { SessionObserverInfo } from '@/lib/backend/observers'
import {
  deliveryLabel,
  lifecycleNote,
  stateWatch,
  summarizeObserver,
} from './SessionObservers'

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

const watch = (scope: string | undefined, key?: string) => ({
  ...(scope !== undefined ? { 'input.scope': { $eq: scope } } : {}),
  ...(key !== undefined ? { 'input.key': { $eq: key } } : {}),
})

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

describe('stateWatch', () => {
  it('reads a keyed state observer, tolerating a missing scope', () => {
    expect(stateWatch(observer({ filters: watch('run', 'done') }))).toEqual({
      scope: 'run',
      key: 'done',
    })
    expect(stateWatch(observer({ filters: watch(undefined, 'done') }))).toEqual(
      {
        scope: undefined,
        key: 'done',
      },
    )
  })

  it('is null for keyless filters and other functions', () => {
    expect(stateWatch(observer({ filters: watch('run') }))).toBeNull()
    expect(
      stateWatch(
        observer({
          functionId: 'cron::on-tick',
          filters: watch(undefined, 'irrelevant'),
        }),
      ),
    ).toBeNull()
  })
})

describe('summarizeObserver', () => {
  it('renders an UNKNOWN future function generically', () => {
    // The compatibility requirement: a function that does not exist yet gets
    // the same treatment as the known ones — scalar args, no interpretation,
    // no crash.
    expect(
      summarizeObserver(
        observer({
          functionId: 'mqtt::on-message',
          args: { topic: 'sensors/#', qos: 1, retained: true },
        }),
      ),
    ).toBe('topic: sensors/# · qos: 1 · retained: true')
  })

  it('caps at three entries, skips nested args, and reads filters', () => {
    expect(
      summarizeObserver(
        observer({
          args: { a: 1, nested: { deep: true }, b: 2 },
          filters: watch('run', 'done'),
        }),
      ),
    ).toBe('a: 1 · b: 2 · input.scope = run')
  })

  it('is null without args or filters', () => {
    expect(summarizeObserver(observer({}))).toBeNull()
    expect(summarizeObserver(observer({ args: {} }))).toBeNull()
    expect(
      summarizeObserver(observer({ args: { only: { nested: 1 } } })),
    ).toBeNull()
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
