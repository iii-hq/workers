import { describe, expect, it } from 'vitest'
import type { SessionTriggerInfo } from '@/lib/backend/triggers'
import {
  deliveryLabel,
  lifecycleNote,
  stateWatch,
  summarizeTriggerConfig,
} from './SessionTriggers'

function trigger(overrides: Partial<SessionTriggerInfo>): SessionTriggerInfo {
  return {
    id: 'sub_1',
    triggerType: 'state',
    delivery: { kind: 'notify' },
    once: true,
    ...overrides,
  }
}

describe('deliveryLabel', () => {
  it('labels a wake and a call from the delivery data alone', () => {
    expect(deliveryLabel(trigger({ delivery: { kind: 'notify' } }))).toBe(
      'notifies this chat',
    )
    expect(
      deliveryLabel(
        trigger({ delivery: { kind: 'call', functionId: 'state::set' } }),
      ),
    ).toBe('calls state::set')
  })
})

describe('stateWatch', () => {
  it('reads a keyed state config, tolerating a missing scope', () => {
    expect(
      stateWatch(trigger({ config: { scope: 'run', key: 'done' } })),
    ).toEqual({ scope: 'run', key: 'done' })
    expect(stateWatch(trigger({ config: { key: 'done' } }))).toEqual({
      scope: undefined,
      key: 'done',
    })
  })

  it('is null for keyless configs and non-state types', () => {
    expect(stateWatch(trigger({ config: { scope: 'run' } }))).toBeNull()
    expect(
      stateWatch(
        trigger({ triggerType: 'cron', config: { key: 'irrelevant' } }),
      ),
    ).toBeNull()
  })
})

describe('summarizeTriggerConfig', () => {
  it('renders an UNKNOWN future trigger source generically', () => {
    // The compatibility requirement: a source that does not exist yet gets
    // the same treatment as the known ones — scalar config entries, no
    // interpretation, no crash.
    expect(
      summarizeTriggerConfig({ topic: 'sensors/#', qos: 1, retained: true }),
    ).toBe('topic: sensors/# · qos: 1 · retained: true')
  })

  it('caps at three scalars and skips nested values', () => {
    expect(
      summarizeTriggerConfig({
        a: 1,
        nested: { deep: true },
        b: 2,
        c: 3,
        d: 4,
      }),
    ).toBe('a: 1 · b: 2 · c: 3')
  })

  it('is null for empty or scalar-free configs', () => {
    expect(summarizeTriggerConfig(undefined)).toBeNull()
    expect(summarizeTriggerConfig({})).toBeNull()
    expect(summarizeTriggerConfig({ only: { nested: 1 } })).toBeNull()
  })
})

describe('lifecycleNote', () => {
  it('prefers the fired ghost marker', () => {
    expect(lifecycleNote(trigger({ fired: true, once: true }))).toBe(
      'fired · unregistered',
    )
  })

  it('joins once, fires, budget, and deadline from the row data', () => {
    const note = lifecycleNote(
      trigger({
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
    expect(lifecycleNote(trigger({ once: false, fires: 0 }))).toBeNull()
  })
})
