import { describe, expect, it } from 'vitest'
import type { SystemMessage } from '@/types/chat'
import {
  activityFromTriggerRecord,
  parseNotification,
  registrationFromCall,
} from './model'

describe('trigger activity model', () => {
  it('normalizes nested lifecycle and explicit delivery', () => {
    const registration = registrationFromCall({
      id: 'call-1',
      subscriptionId: 'sub-1',
      effectiveOnce: false,
      note: 'verify the watched key',
      input: {
        trigger_type: 'cron',
        config: { expression: '0 0 9 * * *' },
        target: { function_id: 'reports::refresh' },
        lifecycle: { once: true, max_fires: 3, expires_at: 2_000 },
      },
    })
    expect(registration.activity).toMatchObject({
      kind: 'registration',
      triggerType: 'cron',
      delivery: { kind: 'call', functionId: 'reports::refresh' },
      lifecycle: {
        state: 'active',
        once: false,
        fires: 0,
        maxFires: 3,
        expiresAt: 2_000,
      },
      note: 'verify the watched key',
    })
  })

  it('prefers durable source data and classifies lifecycle-only records', () => {
    const message: SystemMessage = {
      id: 'e_trigexpired_sub-1',
      role: 'system',
      kind: 'trigger-fired',
      content: 'expired',
      createdAt: 3,
      trigger: {
        subscription_id: 'sub-1',
        trigger_type: 'database::row-changed',
        config: { table: 'orders' },
        target: 'harness::send',
        once: false,
        retired: true,
        fired_at: 3,
        outcome: 'expired',
        retirement_reason: 'expired',
      },
    }
    expect(activityFromTriggerRecord(message)).toMatchObject({
      kind: 'retirement',
      triggerType: 'database::row-changed',
      config: { table: 'orders' },
      outcome: 'expired',
      retirementReason: 'expired',
      lifecycle: { state: 'retired', once: false, fires: 0 },
    })
  })

  it('preserves an explicit null config from the durable record', () => {
    const registration = registrationFromCall({
      id: 'call-1',
      input: { trigger_type: 'cron', config: { expression: '* * * * * *' } },
    })
    const message: SystemMessage = {
      id: 'e_trigfired_sub-1_1',
      role: 'system',
      kind: 'trigger-fired',
      content: 'fired',
      createdAt: 3,
      trigger: {
        subscription_id: 'sub-1',
        trigger_type: 'cron',
        config: null,
        target: 'harness::send',
        once: false,
        fires: 1,
        retired: false,
        fired_at: 3,
      },
    }
    expect(activityFromTriggerRecord(message, registration)?.config).toBeNull()
  })

  it('uses the fire ordinal instead of a newer live-row count', () => {
    const registration = registrationFromCall({
      id: 'call-1',
      subscriptionId: 'sub-1',
      input: { trigger_type: 'cron' },
    })
    registration.activity.lifecycle.fires = 10
    const message: SystemMessage = {
      id: 'e_trigfired_sub-1_2',
      role: 'system',
      kind: 'trigger-fired',
      content: 'fired',
      createdAt: 3,
      trigger: {
        subscription_id: 'sub-1',
        target: 'harness::send',
        once: false,
        retired: false,
        fired_at: 3,
      },
    }
    expect(
      activityFromTriggerRecord(message, registration)?.lifecycle.fires,
    ).toBe(2)
  })

  it('prefers the durable fire count and does not count a skipped delivery', () => {
    const registration = registrationFromCall({
      id: 'call-1',
      subscriptionId: 'sub-1',
      input: { trigger_type: 'cron' },
    })
    registration.activity.lifecycle.fires = 9
    const deliveredTrigger: NonNullable<SystemMessage['trigger']> = {
      subscription_id: 'sub-1',
      target: 'harness::send',
      once: false,
      fires: 2,
      retired: false,
      fired_at: 3,
      outcome: 'delivered',
    }
    const delivered: SystemMessage = {
      id: 'e_trigfired_sub-1_7',
      role: 'system',
      kind: 'trigger-fired',
      content: 'fired',
      createdAt: 3,
      trigger: deliveredTrigger,
    }
    const skipped: SystemMessage = {
      ...delivered,
      id: 'e_trigskip_sub-1_condition_3',
      trigger: {
        ...deliveredTrigger,
        fires: 0,
        outcome: 'skipped',
      },
    }
    expect(
      activityFromTriggerRecord(delivered, registration)?.lifecycle.fires,
    ).toBe(2)
    expect(
      activityFromTriggerRecord(skipped, registration)?.lifecycle.fires,
    ).toBe(0)
  })

  it('only parses the canonical successful wake envelope', () => {
    expect(parseNotification('[notification] ready: {"id":1}')).toEqual({
      name: 'ready',
      payload: { id: 1 },
    })
    expect(
      parseNotification(
        '[notification] binding sub-1 fired but was NOT delivered: condition failed',
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
