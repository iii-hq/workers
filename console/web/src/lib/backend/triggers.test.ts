import { describe, expect, it } from 'vitest'
import type { TriggerFiredData } from '@/types/chat'
import {
  deliveryOf,
  listSessionTriggers,
  mergeFiredTriggers,
  type SessionTriggerInfo,
} from './triggers'

function live(overrides: Partial<SessionTriggerInfo> = {}): SessionTriggerInfo {
  return {
    id: 'sub_live',
    triggerId: 'trg_live',
    triggerType: 'state',
    delivery: { kind: 'notify' },
    config: { scope: 'run', key: 'done' },
    once: true,
    fires: 0,
    createdAt: 1,
    ...overrides,
  }
}

function fired(overrides: Partial<TriggerFiredData> = {}): TriggerFiredData {
  return {
    subscription_id: 'sub_live',
    trigger_id: 'trg_live',
    target: 'harness::send',
    once: true,
    retired: true,
    fired_at: 42,
    ...overrides,
  }
}

describe('deliveryOf', () => {
  it('maps live targets: absent/harness::send = notify, anything else = call', () => {
    expect(deliveryOf(undefined)).toEqual({ kind: 'notify' })
    expect(deliveryOf('harness::send')).toEqual({ kind: 'notify' })
    expect(deliveryOf('database::execute')).toEqual({
      kind: 'call',
      functionId: 'database::execute',
    })
  })

  it('keeps legacy record values rendering', () => {
    expect(deliveryOf('notify')).toEqual({ kind: 'notify' })
    expect(deliveryOf('spawn')).toEqual({
      kind: 'call',
      functionId: 'harness::spawn',
    })
  })
})

describe('listSessionTriggers', () => {
  it('maps binding rows generically, unknown future sources included', async () => {
    const client = {
      trigger: async (id: string, _payload?: unknown) => {
        expect(id).toBe('harness::triggers::list')
        return {
          subscriptions: [
            {
              subscription_id: 'sub_a',
              trigger_id: 'trg_a',
              trigger_type: 'state',
              config: { scope: 'run', key: 'done' },
              label: 'gate',
              action: 'run completion received',
              once: true,
              fires: 0,
              created_at: 10,
            },
            {
              subscription_id: 'sub_b',
              trigger_type: 'cron',
              config: { expression: '0 * * * * *' },
              target: 'state::set',
              once: false,
              max_fires: 6,
              fires: 2,
              created_at: 20,
            },
            {
              // A trigger source that does not exist yet must map unchanged.
              subscription_id: 'sub_c',
              trigger_type: 'mqtt::message',
              config: { topic: 'sensors/#' },
              once: false,
              fires: 0,
              created_at: 30,
            },
          ],
        } as never
      },
    }
    const rows = await listSessionTriggers(client, 's_owner')
    expect(rows).toHaveLength(3)
    expect(rows[0]).toMatchObject({
      id: 'sub_a',
      triggerId: 'trg_a',
      triggerType: 'state',
      delivery: { kind: 'notify' },
      label: 'gate',
      action: 'run completion received',
      once: true,
    })
    expect(rows[1].delivery).toEqual({
      kind: 'call',
      functionId: 'state::set',
    })
    expect(rows[1].maxFires).toBe(6)
    expect(rows[2]).toMatchObject({
      triggerType: 'mqtt::message',
      delivery: { kind: 'notify' },
      config: { topic: 'sensors/#' },
    })
  })

  it('preserves a failed call as an error instead of an empty snapshot', async () => {
    const client = {
      trigger: async () => {
        throw new Error('down')
      },
    }
    await expect(listSessionTriggers(client, 's')).rejects.toThrow('down')
  })
})

describe('mergeFiredTriggers', () => {
  it('returns the poll untouched when nothing retired', () => {
    const polled = [live()]
    expect(mergeFiredTriggers(polled, [fired({ retired: false })])).toBe(polled)
  })

  it('annotates a still-polled retired row in place', () => {
    const merged = mergeFiredTriggers(
      [live()],
      [
        fired({
          fires: 1,
          outcome: 'delivered',
          retirement_reason: 'once_consumed',
        }),
      ],
    )
    expect(merged).toHaveLength(1)
    expect(merged[0].fired).toBe(true)
    expect(merged[0].firedAt).toBe(42)
    expect(merged[0].fires).toBe(1)
    expect(merged[0].outcome).toBe('delivered')
    expect(merged[0].retirementReason).toBe('once_consumed')
  })

  it('ghosts a dropped row, retaining remembered details but preferring record source data', () => {
    const remembered = new Map([
      [
        'sub_live',
        live({
          label: 'remembered',
          conditions: [{ function_id: 'fp::when' }],
        }),
      ],
    ])
    const merged = mergeFiredTriggers(
      [],
      [
        fired({
          fires: 4,
          trigger_type: 'cron',
          config: { expression: '0 * * * * *' },
          action: 'scheduled report became due',
          outcome: 'expired',
          retirement_reason: 'expired',
        }),
      ],
      remembered,
    )
    expect(merged).toHaveLength(1)
    expect(merged[0].label).toBe('remembered')
    expect(merged[0].triggerType).toBe('cron')
    expect(merged[0].config).toEqual({ expression: '0 * * * * *' })
    expect(merged[0].action).toBe('scheduled report became due')
    expect(merged[0].conditions).toEqual([{ function_id: 'fp::when' }])
    expect(merged[0].fires).toBe(4)
    expect(merged[0].outcome).toBe('expired')
    expect(merged[0].retirementReason).toBe('expired')
    expect(merged[0].fired).toBe(true)
  })

  it('enriches a live recurring row without making it inactive', () => {
    const merged = mergeFiredTriggers(
      [live({ once: false })],
      [
        fired({
          once: false,
          retired: false,
          fires: 3,
          trigger_type: 'database::row-changed',
          config: { db: 'primary', table: 'orders' },
          outcome: 'delivery_failed',
          retirement_reason: undefined,
        }),
      ],
    )
    expect(merged[0]).toMatchObject({
      triggerType: 'database::row-changed',
      config: { db: 'primary', table: 'orders' },
      fires: 3,
      outcome: 'delivery_failed',
      retirementReason: undefined,
    })
    expect(merged[0].fired).toBeUndefined()
  })

  it('falls back to a thin record-only ghost after a reload', () => {
    const merged = mergeFiredTriggers(
      [],
      [fired({ scope: 'run', key: 'done', label: undefined })],
    )
    expect(merged).toHaveLength(1)
    expect(merged[0].triggerType).toBe('state')
    expect(merged[0].config).toEqual({ scope: 'run', key: 'done' })
    expect(merged[0].delivery).toEqual({ kind: 'notify' })
    expect(merged[0].fires).toBeUndefined()
    expect(merged[0].retirementReason).toBeUndefined()
  })

  it('reconstructs an enriched non-state ghost without a remembered row', () => {
    const merged = mergeFiredTriggers(
      [],
      [
        fired({
          fires: 9,
          trigger_type: 'cron',
          config: { expression: '*/5 * * * * *' },
          outcome: 'unregistered',
          retirement_reason: 'unregistered',
        }),
      ],
    )
    expect(merged[0]).toMatchObject({
      triggerType: 'cron',
      config: { expression: '*/5 * * * * *' },
      fires: 9,
      outcome: 'unregistered',
      retirementReason: 'unregistered',
      fired: true,
    })
  })

  it('renders a legacy spawn record as a call ghost', () => {
    const merged = mergeFiredTriggers([], [fired({ target: 'spawn' })])
    expect(merged[0].delivery).toEqual({
      kind: 'call',
      functionId: 'harness::spawn',
    })
  })

  it('collapses repeat fires to the newest record', () => {
    const merged = mergeFiredTriggers(
      [],
      [fired({ fired_at: 1 }), fired({ fired_at: 2 })],
    )
    expect(merged).toHaveLength(1)
    expect(merged[0].firedAt).toBe(2)
  })

  it('keeps the newest retirement when a dropped row has newer non-retired activity', () => {
    const merged = mergeFiredTriggers(
      [],
      [
        fired({
          fired_at: 1,
          outcome: 'expired',
          retirement_reason: 'expired',
        }),
        fired({
          fired_at: 2,
          outcome: 'unregistered',
          retirement_reason: 'unregistered',
        }),
        fired({
          retired: false,
          fired_at: 3,
          outcome: 'delivered',
          retirement_reason: undefined,
        }),
      ],
    )
    expect(merged).toHaveLength(1)
    expect(merged[0]).toMatchObject({
      fired: true,
      firedAt: 2,
      outcome: 'unregistered',
      retirementReason: 'unregistered',
    })
  })

  it('keeps newest-activity semantics while the row is still live', () => {
    const merged = mergeFiredTriggers(
      [live({ once: false })],
      [
        fired({
          fired_at: 1,
          outcome: 'expired',
          retirement_reason: 'expired',
        }),
        fired({
          retired: false,
          fired_at: 2,
          fires: 4,
          outcome: 'delivered',
          retirement_reason: undefined,
        }),
      ],
    )
    expect(merged).toHaveLength(1)
    expect(merged[0]).toMatchObject({
      fires: 4,
      outcome: 'delivered',
    })
    expect(merged[0].fired).toBeUndefined()
    expect(merged[0].retirementReason).toBeUndefined()
  })

  it('correlates on the subscription id even without an engine trigger id', () => {
    const merged = mergeFiredTriggers(
      [live({ id: 'sub_x', triggerId: undefined })],
      [fired({ subscription_id: 'sub_x', trigger_id: undefined })],
    )
    expect(merged).toHaveLength(1)
    expect(merged[0].fired).toBe(true)
    expect(merged[0].fires).toBe(0)
  })
})
