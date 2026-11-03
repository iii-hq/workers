import { describe, expect, it } from 'vitest'
import { observerFiredOf } from '@/lib/sessions/stored-records'
import type { SystemMessage } from '@/types/chat'
import {
  activityFromObserverRecord,
  isConditionWait,
  parseNotification,
  registrationFromCall,
} from './model'

describe('observer activity model', () => {
  it('normalizes nested lifecycle and explicit delivery', () => {
    const registration = registrationFromCall({
      id: 'call-1',
      observerId: 'obs-1',
      effectiveOnce: false,
      note: 'verify the watched key',
      input: {
        function_id: 'cron::on-tick',
        args: { expression: '0 0 9 * * * *' },
        action: 'the morning report is due',
        target: { function_id: 'reports::refresh' },
        lifecycle: { once: true, max_fires: 3 },
      },
    })
    expect(registration.activity).toMatchObject({
      kind: 'registration',
      functionId: 'cron::on-tick',
      args: { expression: '0 0 9 * * * *' },
      action: 'the morning report is due',
      delivery: { kind: 'call', functionId: 'reports::refresh' },
      lifecycle: {
        state: 'active',
        once: false,
        fires: 0,
        maxFires: 3,
      },
      observerId: 'obs-1',
      note: 'verify the watched key',
    })
  })

  it('prefers durable source data and classifies lifecycle-only records', () => {
    const message: SystemMessage = {
      id: 'e_obsexpired_obs-1',
      role: 'system',
      kind: 'observer-fired',
      content: 'expired',
      createdAt: 3,
      observer: {
        observer_id: 'obs-1',
        function_id: 'database::on-row-changed',
        args: { db: 'primary', table: 'orders' },
        target: 'harness::send',
        once: false,
        retired: true,
        fired_at: 3,
        outcome: 'expired',
        retirement_reason: 'expired',
      },
    }
    expect(activityFromObserverRecord(message)).toMatchObject({
      kind: 'retirement',
      functionId: 'database::on-row-changed',
      args: { db: 'primary', table: 'orders' },
      outcome: 'expired',
      retirementReason: 'expired',
      lifecycle: { state: 'retired', once: false, fires: 0 },
    })
  })

  it('reads a state record without a source as a state::on-changed watch', () => {
    const message: SystemMessage = {
      id: 'e_obsfired_obs-1_1',
      role: 'system',
      kind: 'observer-fired',
      content: 'fired',
      createdAt: 3,
      observer: {
        observer_id: 'obs-1',
        target: 'harness::send',
        scope: 'run',
        key: 'done',
        once: true,
        retired: true,
        fired_at: 3,
      },
    }
    expect(activityFromObserverRecord(message)).toMatchObject({
      functionId: 'state::on-changed',
      filters: {
        'input.scope': { $eq: 'run' },
        'input.key': { $eq: 'done' },
      },
    })
  })

  it('uses the fire ordinal instead of a newer live-row count', () => {
    const registration = registrationFromCall({
      id: 'call-1',
      observerId: 'obs-1',
      input: { function_id: 'cron::on-tick' },
    })
    registration.activity.lifecycle.fires = 10
    const message: SystemMessage = {
      id: 'e_obsfired_obs-1_2',
      role: 'system',
      kind: 'observer-fired',
      content: 'fired',
      createdAt: 3,
      observer: {
        observer_id: 'obs-1',
        target: 'harness::send',
        once: false,
        retired: false,
        fired_at: 3,
      },
    }
    expect(
      activityFromObserverRecord(message, registration)?.lifecycle.fires,
    ).toBe(2)
  })

  it('prefers the durable fire count and does not count a skipped delivery', () => {
    const registration = registrationFromCall({
      id: 'call-1',
      observerId: 'obs-1',
      input: { function_id: 'cron::on-tick' },
    })
    registration.activity.lifecycle.fires = 9
    const deliveredRecord: NonNullable<SystemMessage['observer']> = {
      observer_id: 'obs-1',
      target: 'harness::send',
      once: false,
      fires: 2,
      retired: false,
      fired_at: 3,
      outcome: 'delivered',
    }
    const delivered: SystemMessage = {
      id: 'e_obsfired_obs-1_7',
      role: 'system',
      kind: 'observer-fired',
      content: 'fired',
      createdAt: 3,
      observer: deliveredRecord,
    }
    const skipped: SystemMessage = {
      ...delivered,
      id: 'e_obsskip_obs-1_condition_3',
      observer: {
        ...deliveredRecord,
        fires: 0,
        outcome: 'skipped',
      },
    }
    expect(
      activityFromObserverRecord(delivered, registration)?.lifecycle.fires,
    ).toBe(2)
    expect(
      activityFromObserverRecord(skipped, registration)?.lifecycle.fires,
    ).toBe(0)
  })

  it('recognizes a condition wait under either record id prefix', () => {
    const waiting = (id: string) =>
      activityFromObserverRecord({
        id,
        role: 'system',
        kind: 'observer-fired',
        content: 'skipped',
        createdAt: 3,
        observer: {
          observer_id: 'obs-1',
          target: 'harness::send',
          once: false,
          retired: false,
          fired_at: 3,
          outcome: 'skipped',
        },
      })
    const gate = 'condition_0123456789abcdef0123456789abcdef'
    const current = waiting(`e_obsskip_obs-1_${gate}`)
    const stored = waiting(`e_trigskip_obs-1_${gate}`)
    expect(current && isConditionWait(current)).toBe(true)
    expect(stored && isConditionWait(stored)).toBe(true)
  })

  it('reads a record written before 0.26 with its source translated', () => {
    const record = observerFiredOf({
      subscription_id: 'sub-1',
      trigger_type: 'cron',
      config: { expression: '0 0 9 * * * *' },
      target: 'reports::refresh',
      once: false,
      retired: false,
      fired_at: 3,
    })
    expect(record).toMatchObject({
      observer_id: 'sub-1',
      function_id: 'cron::on-tick',
      args: { expression: '0 0 9 * * * *' },
    })
    expect(record).not.toHaveProperty('trigger_type')
    expect(
      observerFiredOf({
        subscription_id: 'sub-2',
        trigger_type: 'state',
        config: { scope: 'run', key: 'done' },
        target: 'harness::send',
        once: true,
        retired: true,
        fired_at: 3,
      }),
    ).toMatchObject({
      function_id: 'state::on-changed',
      filters: {
        'input.scope': { $eq: 'run' },
        'input.key': { $eq: 'done' },
      },
    })
  })

  it('only parses the canonical successful wake envelope', () => {
    expect(parseNotification('[notification] ready: {"id":1}')).toEqual({
      name: 'ready',
      payload: { id: 1 },
    })
    expect(
      parseNotification(
        '[notification] observer obs-1 fired but was NOT delivered: condition failed',
      ),
    ).toBeNull()
    expect(parseNotification('[notification] batch: [1,2]')).toEqual({
      name: 'batch',
      payload: [1, 2],
    })
    expect(parseNotification('[notification] {"id":2}')).toEqual({
      payload: { id: 2 },
    })
  })
})
