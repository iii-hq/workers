import { describe, expect, it } from 'vitest'
import type { SessionTriggerInfo } from '@/lib/backend/triggers'
import type {
  FunctionTriggerMessage,
  Message,
  SystemMessage,
  TriggerFiredData,
  UserMessage,
} from '@/types/chat'
import { parseNotification } from './Message'
import { resolveRegistrations, subscriptionIdOf } from './MessageList'

/* ── fixtures ─────────────────────────────────────────────────────── */

function fired(overrides: Partial<TriggerFiredData>): TriggerFiredData {
  return {
    subscription_id: 'sub_1',
    target: 'db::exec',
    once: false,
    retired: false,
    fired_at: 1,
    ...overrides,
  }
}

function fireMsg(id: string, t: Partial<TriggerFiredData>): SystemMessage {
  return {
    id,
    role: 'system',
    kind: 'trigger-fired',
    content: 'x',
    createdAt: 1,
    trigger: fired(t),
  }
}

function registerCall(
  id: string,
  input: unknown,
  output: unknown,
): FunctionTriggerMessage {
  return {
    id,
    role: 'function-trigger',
    functionId: 'engine::register_trigger',
    input,
    output,
    createdAt: 1,
  }
}

function notifMsg(id: string, content: string): UserMessage {
  return { id, role: 'user', content, createdAt: 1, notification: true }
}

function row(overrides: Partial<SessionTriggerInfo>): SessionTriggerInfo {
  return {
    id: 'sub_1',
    triggerType: 'database::row-changed',
    delivery: { kind: 'notify' },
    once: false,
    ...overrides,
  }
}

/* ── subscriptionIdOf ─────────────────────────────────────────────── */

describe('subscriptionIdOf', () => {
  it('reads the result envelope text block', () => {
    const output = {
      content: [
        { type: 'text', text: '{"once":true,"subscription_id":"sub_a"}' },
      ],
      details: undefined,
    }
    expect(subscriptionIdOf(output)).toBe('sub_a')
  })

  it('reads the advisory-note envelope (id beside note text)', () => {
    const output = {
      content: [
        {
          type: 'text',
          text: '{"note":"registration SUCCEEDED; ...","once":false,"subscription_id":"sub_b"}',
        },
      ],
    }
    expect(subscriptionIdOf(output)).toBe('sub_b')
  })

  it('falls back to envelope details', () => {
    const output = {
      content: [{ type: 'text', text: 'not json' }],
      details: { subscription_id: 'sub_c' },
    }
    expect(subscriptionIdOf(output)).toBe('sub_c')
  })

  it('handles bare objects and JSON strings', () => {
    expect(subscriptionIdOf({ subscription_id: 'sub_d' })).toBe('sub_d')
    expect(subscriptionIdOf('{"subscription_id":"sub_e"}')).toBe('sub_e')
  })

  it('returns null for garbage', () => {
    expect(subscriptionIdOf(undefined)).toBeNull()
    expect(subscriptionIdOf('not json')).toBeNull()
    expect(subscriptionIdOf({ other: 1 })).toBeNull()
    expect(subscriptionIdOf(['sub_f'])).toBeNull()
    expect(subscriptionIdOf({ subscription_id: 42 })).toBeNull()
  })
})

/* ── parseNotification ────────────────────────────────────────────── */

describe('parseNotification', () => {
  it('splits name and payload, trimming the name', () => {
    const p = parseNotification(
      '[notification] rx-done : {"op":"insert","n":1}',
    )
    expect(p).toEqual({ name: 'rx-done', payload: { op: 'insert', n: 1 } })
  })

  it('rejects non-object payloads and non-notification content', () => {
    expect(parseNotification('[notification] x: [1,2]')).toBeNull()
    expect(parseNotification('[notification] x: not json')).toBeNull()
    expect(parseNotification('plain user text')).toBeNull()
  })
})

/* ── resolveRegistrations ─────────────────────────────────────────── */

describe('resolveRegistrations', () => {
  const regInput = {
    trigger_type: 'database::row-changed',
    config: { db: 'mysql', table: 't' },
    label: 'ledger',
  }
  const regOutput = {
    content: [{ type: 'text', text: '{"subscription_id":"sub_1"}' }],
  }

  it('prefers the harness row when it carries config', () => {
    const messages: Message[] = [
      registerCall('m1', regInput, regOutput),
      fireMsg('m2', { subscription_id: 'sub_1' }),
    ]
    const rows = new Map([['sub_1', row({ config: { db: 'mysql' } })]])
    const out = resolveRegistrations(messages, rows)
    expect(out.get('m2')).toEqual({
      summary: 'database::row-changed',
      detail: {
        config: { db: 'mysql' },
        conditions: undefined,
        once: false,
        label: undefined,
        function_id: undefined,
      },
    })
  })

  it('preserves the call target on row-backed registrations', () => {
    const messages: Message[] = [fireMsg('m1', { subscription_id: 'sub_1' })]
    const rows = new Map([
      [
        'sub_1',
        row({
          config: { db: 'mysql' },
          delivery: { kind: 'call', functionId: 'database::executeBatch' },
        }),
      ],
    ])
    const out = resolveRegistrations(messages, rows)
    const detail = out.get('m1')?.detail as Record<string, unknown> | undefined
    expect(detail?.function_id).toBe('database::executeBatch')
  })

  it('falls back to the register call for a config-less ghost row', () => {
    const messages: Message[] = [
      registerCall('m1', regInput, regOutput),
      fireMsg('m2', { subscription_id: 'sub_1' }),
    ]
    const rows = new Map([['sub_1', row({ config: undefined })]])
    const out = resolveRegistrations(messages, rows)
    expect(out.get('m2')).toEqual({
      summary: 'from register call',
      detail: regInput,
    })
  })

  it('recovers from the transcript when no row exists at all', () => {
    const messages: Message[] = [
      registerCall('m1', regInput, regOutput),
      fireMsg('m2', { subscription_id: 'sub_1' }),
    ]
    const out = resolveRegistrations(messages, undefined)
    expect(out.get('m2')?.summary).toBe('from register call')
  })

  it('correlates a notification to a notify fire by name', () => {
    const messages: Message[] = [
      registerCall('m1', regInput, regOutput),
      fireMsg('m2', {
        subscription_id: 'sub_1',
        target: 'harness::send',
        label: 'rx-done',
      }),
      notifMsg('m3', '[notification] rx-done: {"op":"insert"}'),
    ]
    const out = resolveRegistrations(messages, undefined)
    expect(out.get('m3')?.summary).toBe('from register call')
  })

  it('correlates when the notification precedes its fire record (idle wake)', () => {
    const messages: Message[] = [
      registerCall('m1', regInput, regOutput),
      notifMsg('m2', '[notification] rx-done: {"op":"insert"}'),
      fireMsg('m3', {
        subscription_id: 'sub_1',
        target: 'harness::send',
        label: 'rx-done',
      }),
    ]
    const out = resolveRegistrations(messages, undefined)
    expect(out.get('m2')?.summary).toBe('from register call')
  })

  it('prefers the subscription id embedded in the entry id over name matching', () => {
    const otherInput = { ...regInput, label: 'other' }
    const otherOutput = {
      content: [{ type: 'text', text: '{"subscription_id":"sub_2"}' }],
    }
    const messages: Message[] = [
      registerCall('m1', regInput, regOutput),
      registerCall('m2', otherInput, otherOutput),
      // Fire named rx-done points at sub_2, but the entry id says sub_1.
      fireMsg('m3', {
        subscription_id: 'sub_2',
        target: 'harness::send',
        label: 'rx-done',
      }),
      notifMsg('e_notify_sub_1', '[notification] rx-done: {"op":"insert"}'),
    ]
    const out = resolveRegistrations(messages, undefined)
    expect(out.get('e_notify_sub_1')?.detail).toEqual(regInput)
  })

  it('does not correlate a notification to a call-target fire', () => {
    const messages: Message[] = [
      registerCall('m1', regInput, regOutput),
      fireMsg('m2', { subscription_id: 'sub_1', label: 'rx-done' }), // target db::exec
      notifMsg('m3', '[notification] rx-done: {"op":"insert"}'),
    ]
    const out = resolveRegistrations(messages, undefined)
    expect(out.get('m3')).toBeUndefined()
  })

  it('leaves messages without any source unresolved', () => {
    const messages: Message[] = [fireMsg('m1', { subscription_id: 'sub_x' })]
    const out = resolveRegistrations(messages, undefined)
    expect(out.size).toBe(0)
  })
})
