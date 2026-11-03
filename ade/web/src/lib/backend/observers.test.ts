import { describe, expect, it } from 'vitest'
import type { ObserverFiredData } from '@/types/chat'
import {
  deliveryOf,
  listSessionObservers,
  mergeFiredObservers,
  type SessionObserverInfo,
} from './observers'

function live(
  overrides: Partial<SessionObserverInfo> = {},
): SessionObserverInfo {
  return {
    id: 'obs_live',
    functionId: 'state::on-changed',
    delivery: { kind: 'notify' },
    filters: {
      'input.scope': { $eq: 'run' },
      'input.key': { $eq: 'done' },
    },
    once: true,
    fires: 0,
    createdAt: 1,
    ...overrides,
  }
}

function fired(overrides: Partial<ObserverFiredData> = {}): ObserverFiredData {
  return {
    observer_id: 'obs_live',
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

describe('listSessionObservers', () => {
  it('maps observer rows generically, unknown future functions included', async () => {
    const client = {
      call: async (id: string, _payload?: unknown) => {
        expect(id).toBe('harness::observers::list')
        return {
          observers: [
            {
              observer_id: 'obs_a',
              function_id: 'state::on-changed',
              filters: {
                'input.scope': { $eq: 'run' },
                'input.key': { $eq: 'done' },
              },
              label: 'gate',
              action: 'run completion received',
              once: true,
              fires: 0,
              created_at: 10,
            },
            {
              observer_id: 'obs_b',
              function_id: 'cron::on-tick',
              args: { expression: '0 * * * * *' },
              target: 'state::set',
              once: false,
              max_fires: 6,
              fires: 2,
              created_at: 20,
            },
            {
              // A function that does not exist yet must map unchanged.
              observer_id: 'obs_c',
              function_id: 'mqtt::on-message',
              args: { topic: 'sensors/#' },
              once: false,
              fires: 0,
              created_at: 30,
            },
          ],
        } as never
      },
    }
    const rows = await listSessionObservers(client, 's_owner')
    expect(rows).toHaveLength(3)
    expect(rows[0]).toMatchObject({
      id: 'obs_a',
      functionId: 'state::on-changed',
      filters: { 'input.key': { $eq: 'done' } },
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
      functionId: 'mqtt::on-message',
      delivery: { kind: 'notify' },
      args: { topic: 'sensors/#' },
    })
  })

  it('preserves a failed call as an error instead of an empty snapshot', async () => {
    const client = {
      call: async () => {
        throw new Error('down')
      },
    }
    await expect(listSessionObservers(client, 's')).rejects.toThrow('down')
  })
})

describe('mergeFiredObservers', () => {
  it('returns the poll untouched when nothing retired', () => {
    const polled = [live()]
    expect(mergeFiredObservers(polled, [fired({ retired: false })])).toBe(
      polled,
    )
  })

  it('annotates a still-polled retired row in place', () => {
    const merged = mergeFiredObservers(
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
        'obs_live',
        live({
          label: 'remembered',
          conditions: [{ function_id: 'fp::when' }],
        }),
      ],
    ])
    const merged = mergeFiredObservers(
      [],
      [
        fired({
          fires: 4,
          function_id: 'cron::on-tick',
          args: { expression: '0 * * * * *' },
          action: 'scheduled report became due',
          outcome: 'expired',
          retirement_reason: 'expired',
        }),
      ],
      remembered,
    )
    expect(merged).toHaveLength(1)
    expect(merged[0].label).toBe('remembered')
    expect(merged[0].functionId).toBe('cron::on-tick')
    expect(merged[0].args).toEqual({ expression: '0 * * * * *' })
    expect(merged[0].action).toBe('scheduled report became due')
    expect(merged[0].conditions).toEqual([{ function_id: 'fp::when' }])
    expect(merged[0].fires).toBe(4)
    expect(merged[0].outcome).toBe('expired')
    expect(merged[0].retirementReason).toBe('expired')
    expect(merged[0].fired).toBe(true)
  })

  it('enriches a live recurring row without making it inactive', () => {
    const merged = mergeFiredObservers(
      [live({ once: false })],
      [
        fired({
          once: false,
          retired: false,
          fires: 3,
          function_id: 'database::on-row-changed',
          args: { db: 'primary', table: 'orders' },
          outcome: 'delivery_failed',
          retirement_reason: undefined,
        }),
      ],
    )
    expect(merged[0]).toMatchObject({
      functionId: 'database::on-row-changed',
      args: { db: 'primary', table: 'orders' },
      fires: 3,
      outcome: 'delivery_failed',
      retirementReason: undefined,
    })
    expect(merged[0].fired).toBeUndefined()
  })

  it('falls back to a thin record-only ghost after a reload', () => {
    const merged = mergeFiredObservers(
      [],
      [fired({ scope: 'run', key: 'done', label: undefined })],
    )
    expect(merged).toHaveLength(1)
    expect(merged[0].functionId).toBe('state::on-changed')
    expect(merged[0].filters).toEqual({
      'input.scope': { $eq: 'run' },
      'input.key': { $eq: 'done' },
    })
    expect(merged[0].delivery).toEqual({ kind: 'notify' })
    expect(merged[0].fires).toBeUndefined()
    expect(merged[0].retirementReason).toBeUndefined()
  })

  it('reconstructs an enriched non-state ghost without a remembered row', () => {
    const merged = mergeFiredObservers(
      [],
      [
        fired({
          fires: 9,
          function_id: 'cron::on-tick',
          args: { expression: '*/5 * * * * *' },
          outcome: 'unregistered',
          retirement_reason: 'unregistered',
        }),
      ],
    )
    expect(merged[0]).toMatchObject({
      functionId: 'cron::on-tick',
      args: { expression: '*/5 * * * * *' },
      fires: 9,
      outcome: 'unregistered',
      retirementReason: 'unregistered',
      fired: true,
    })
  })

  it('renders a legacy spawn record as a call ghost', () => {
    const merged = mergeFiredObservers([], [fired({ target: 'spawn' })])
    expect(merged[0].delivery).toEqual({
      kind: 'call',
      functionId: 'harness::spawn',
    })
  })

  it('collapses repeat fires to the newest record', () => {
    const merged = mergeFiredObservers(
      [],
      [fired({ fired_at: 1 }), fired({ fired_at: 2 })],
    )
    expect(merged).toHaveLength(1)
    expect(merged[0].firedAt).toBe(2)
  })

  it('keeps the newest retirement when a dropped row has newer non-retired activity', () => {
    const merged = mergeFiredObservers(
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
    const merged = mergeFiredObservers(
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

  it('correlates on the observer id', () => {
    const merged = mergeFiredObservers(
      [live({ id: 'obs_x' })],
      [fired({ observer_id: 'obs_x' })],
    )
    expect(merged).toHaveLength(1)
    expect(merged[0].fired).toBe(true)
    expect(merged[0].fires).toBe(0)
  })
})
