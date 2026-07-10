import { describe, expect, it } from 'vitest'
import type { TriggerFiredData } from '@/types/chat'
import { mergeFiredTriggers, type SessionTriggerInfo } from './triggers'

const live = (
  id: string,
  over: Partial<SessionTriggerInfo> = {},
): SessionTriggerInfo => ({
  id,
  triggerType: 'state',
  functionId: 'harness::react',
  config: {},
  configSummary: '',
  ...over,
})

const rec = (over: Partial<TriggerFiredData> = {}): TriggerFiredData => ({
  subscription_id: 'sub_1',
  target: 'spawn',
  once: true,
  retired: true,
  fired_at: 1,
  ...over,
})

describe('mergeFiredTriggers', () => {
  it('appends a ghost row for a retired fire absent from the poll', () => {
    const merged = mergeFiredTriggers(
      [],
      [rec({ trigger_id: 't-1', model: 'm', scope: 'sc', key: 'k' })],
    )
    expect(merged).toHaveLength(1)
    expect(merged[0]).toMatchObject({
      id: 't-1',
      fired: true,
      once: true,
      triggerType: 'state',
      functionId: 'harness::react',
      config: { scope: 'sc', key: 'k' },
    })
  })

  it('annotates a still-polled retired trigger in place instead of ghosting', () => {
    const polled = [live('t-1', { label: 'facts', once: true })]
    const merged = mergeFiredTriggers(polled, [
      rec({ trigger_id: 't-1', fired_at: 7 }),
    ])
    expect(merged).toHaveLength(1)
    // Same row (full config/metadata retained), just marked fired.
    expect(merged[0]).toMatchObject({
      id: 't-1',
      label: 'facts',
      fired: true,
      firedAt: 7,
    })
  })

  it('ignores non-retired fires (binding still live)', () => {
    expect(mergeFiredTriggers([], [rec({ retired: false })])).toEqual([])
  })

  it('collapses repeat fires of the same trigger to one newest ghost', () => {
    const merged = mergeFiredTriggers(
      [],
      [
        rec({ trigger_id: 't-1', fired_at: 1 }),
        rec({ trigger_id: 't-1', fired_at: 2 }),
      ],
    )
    expect(merged).toHaveLength(1)
    expect(merged[0]).toMatchObject({ id: 't-1', firedAt: 2 })
  })

  it('prefers the full last-seen row for a ghost so workflow structure survives', () => {
    const full = live('t-1', {
      label: 'insights',
      once: true,
      metadata: {
        join: { id: 'J1', expect: ['insights', 'glossary'], key: 'insights' },
        model: 'm',
        task: 'merge everything',
      },
    })
    const seen = new Map([[full.id, full]])
    const merged = mergeFiredTriggers([], [rec({ trigger_id: 't-1' })], seen)
    expect(merged).toHaveLength(1)
    // Full metadata retained (join grouping / DAG structure), fired flagged.
    expect(merged[0]).toMatchObject({
      id: 't-1',
      fired: true,
      firedAt: 1,
      label: 'insights',
      metadata: { join: { id: 'J1' }, task: 'merge everything' },
    })
    // Without the cache (e.g. after a reload) the thin record ghost stands.
    const thin = mergeFiredTriggers([], [rec({ trigger_id: 't-1' })])
    expect(thin[0].metadata?.join).toBeUndefined()
  })

  it('falls back to a synthetic id when the record has no trigger id', () => {
    const merged = mergeFiredTriggers(
      [],
      [rec({ subscription_id: 'sub_9', trigger_id: undefined })],
    )
    expect(merged[0]).toMatchObject({ id: 'fired:sub_9', fired: true })
  })

  it('never renders an empty ghost title: label-less non-state falls back to "trigger"', () => {
    const merged = mergeFiredTriggers(
      [],
      [rec({ trigger_id: 't-3', key: undefined, label: undefined })],
    )
    expect(merged[0]).toMatchObject({ id: 't-3', triggerType: 'trigger' })
  })

  it('marks a notify fire as a notify-target ghost', () => {
    const merged = mergeFiredTriggers(
      [],
      [
        rec({
          trigger_id: 't-2',
          target: 'notify',
          label: 'ping',
          key: undefined,
        }),
      ],
    )
    expect(merged[0]).toMatchObject({
      id: 't-2',
      fired: true,
      functionId: 'harness::notify_agent',
      label: 'ping',
    })
  })
})
