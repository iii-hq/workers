import { describe, expect, it } from 'vitest'
import type { SessionObserverInfo } from '@/lib/backend/observers'
import type {
  FunctionCallMessage,
  Message,
  ObserverFiredData,
  SystemMessage,
  UserMessage,
} from '@/types/chat'
import { parseNotification } from './Message'
import { observerIdOf, resolveRegistrations } from './MessageList'

/* ── fixtures ─────────────────────────────────────────────────────── */

function fired(overrides: Partial<ObserverFiredData>): ObserverFiredData {
  return {
    observer_id: 'obs_1',
    target: 'db::exec',
    once: false,
    retired: false,
    fired_at: 1,
    ...overrides,
  }
}

function fireMsg(id: string, t: Partial<ObserverFiredData>): SystemMessage {
  return {
    id,
    role: 'system',
    kind: 'observer-fired',
    content: 'x',
    createdAt: 1,
    observer: fired(t),
  }
}

function observeCall(
  id: string,
  input: unknown,
  output: unknown,
): FunctionCallMessage {
  return {
    id,
    role: 'function-call',
    functionId: 'harness::observe',
    input,
    output,
    createdAt: 1,
  }
}

function notifMsg(id: string, content: string): UserMessage {
  return { id, role: 'user', content, createdAt: 1, notification: true }
}

function row(overrides: Partial<SessionObserverInfo>): SessionObserverInfo {
  return {
    id: 'obs_1',
    functionId: 'database::on-row-changed',
    delivery: { kind: 'notify' },
    once: false,
    ...overrides,
  }
}

/* ── observerIdOf ─────────────────────────────────────────────────── */

describe('observerIdOf', () => {
  it('reads the result envelope text block', () => {
    const output = {
      content: [{ type: 'text', text: '{"once":true,"observer_id":"obs_a"}' }],
      details: undefined,
    }
    expect(observerIdOf(output)).toBe('obs_a')
  })

  it('reads the advisory-note envelope (id beside note text)', () => {
    const output = {
      content: [
        {
          type: 'text',
          text: '{"note":"observer created; ...","once":false,"observer_id":"obs_b"}',
        },
      ],
    }
    expect(observerIdOf(output)).toBe('obs_b')
  })

  it('falls back to envelope details', () => {
    const output = {
      content: [{ type: 'text', text: 'not json' }],
      details: { observer_id: 'obs_c' },
    }
    expect(observerIdOf(output)).toBe('obs_c')
  })

  it('handles bare objects and JSON strings', () => {
    expect(observerIdOf({ observer_id: 'obs_d' })).toBe('obs_d')
    expect(observerIdOf('{"observer_id":"obs_e"}')).toBe('obs_e')
  })

  it('returns null for garbage', () => {
    expect(observerIdOf(undefined)).toBeNull()
    expect(observerIdOf('not json')).toBeNull()
    expect(observerIdOf({ other: 1 })).toBeNull()
    expect(observerIdOf(['obs_f'])).toBeNull()
    expect(observerIdOf({ observer_id: 42 })).toBeNull()
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

  it('accepts arbitrary JSON payloads and rejects non-notification prose', () => {
    expect(parseNotification('[notification] x: [1,2]')).toEqual({
      name: 'x',
      payload: [1, 2],
    })
    expect(parseNotification('[notification] 42')).toEqual({ payload: 42 })
    expect(parseNotification('[notification] x: not json')).toBeNull()
    expect(parseNotification('plain user text')).toBeNull()
  })
})

/* ── resolveRegistrations ─────────────────────────────────────────── */

describe('resolveRegistrations', () => {
  const observeInput = {
    function_id: 'database::on-row-changed',
    args: { db: 'mysql', table: 't' },
    label: 'ledger',
    action: 'ledger row changed',
  }
  const observeOutput = {
    content: [{ type: 'text', text: '{"observer_id":"obs_1"}' }],
  }

  it('prefers the harness row when it carries args', () => {
    const messages: Message[] = [
      observeCall('m1', observeInput, observeOutput),
      fireMsg('m2', { observer_id: 'obs_1' }),
    ]
    const rows = new Map([['obs_1', row({ args: { db: 'mysql' } })]])
    const out = resolveRegistrations(messages, rows)
    expect(out.get('m2')).toMatchObject({
      summary: 'database::on-row-changed',
      detail: {
        function_id: 'database::on-row-changed',
        args: { db: 'mysql' },
        conditions: undefined,
        once: false,
        label: undefined,
        target: undefined,
      },
      activity: {
        functionId: 'database::on-row-changed',
        args: { db: 'mysql' },
        delivery: { kind: 'notify' },
        lifecycle: { state: 'active', once: false, fires: 0 },
      },
    })
  })

  it('preserves the call target on row-backed registrations', () => {
    const messages: Message[] = [fireMsg('m1', { observer_id: 'obs_1' })]
    const rows = new Map([
      [
        'obs_1',
        row({
          args: { db: 'mysql' },
          delivery: { kind: 'call', functionId: 'database::executeBatch' },
        }),
      ],
    ])
    const out = resolveRegistrations(messages, rows)
    const detail = out.get('m1')?.detail as Record<string, unknown> | undefined
    expect(detail?.target).toEqual({ function_id: 'database::executeBatch' })
  })

  it('falls back to the observe call for a source-less ghost row', () => {
    const messages: Message[] = [
      observeCall('m1', observeInput, observeOutput),
      fireMsg('m2', { observer_id: 'obs_1' }),
    ]
    const rows = new Map([['obs_1', row({})]])
    const out = resolveRegistrations(messages, rows)
    expect(out.get('m2')).toMatchObject({
      summary: 'from observe call',
      detail: observeInput,
      activity: {
        functionId: 'database::on-row-changed',
        args: { db: 'mysql', table: 't' },
        label: 'ledger',
        action: 'ledger row changed',
      },
    })
  })

  it('uses the richer observe call for a reconstructed ghost with args', () => {
    const input = {
      ...observeInput,
      conditions: [
        { function_id: 'state::equals', config: { path: '/ready' } },
      ],
      lifecycle: { max_fires: 3 },
    }
    const messages: Message[] = [
      observeCall('m1', input, observeOutput),
      fireMsg('m2', {
        observer_id: 'obs_1',
        retired: true,
        function_id: 'database::on-row-changed',
        args: { db: 'mysql', table: 't' },
      }),
    ]
    const rows = new Map([
      [
        'obs_1',
        row({
          fired: true,
          args: { db: 'mysql', table: 't' },
          conditions: undefined,
          maxFires: undefined,
        }),
      ],
    ])
    const activity = resolveRegistrations(messages, rows).get('m2')?.activity
    expect(activity).toMatchObject({
      conditions: [
        { function_id: 'state::equals', config: { path: '/ready' } },
      ],
      lifecycle: { maxFires: 3 },
    })
  })

  it('recovers from the transcript when no row exists at all', () => {
    const messages: Message[] = [
      observeCall('m1', observeInput, observeOutput),
      fireMsg('m2', { observer_id: 'obs_1' }),
    ]
    const out = resolveRegistrations(messages, undefined)
    expect(out.get('m2')?.summary).toBe('from observe call')
  })

  it('correlates a notification to a notify fire by name', () => {
    const messages: Message[] = [
      observeCall('m1', observeInput, observeOutput),
      fireMsg('m2', {
        observer_id: 'obs_1',
        target: 'harness::send',
        label: 'rx-done',
      }),
      notifMsg('m3', '[notification] rx-done: {"op":"insert"}'),
    ]
    const out = resolveRegistrations(messages, undefined)
    expect(out.get('m3')?.summary).toBe('from observe call')
  })

  it('correlates when the notification precedes its fire record (idle wake)', () => {
    const messages: Message[] = [
      observeCall('m1', observeInput, observeOutput),
      notifMsg('m2', '[notification] rx-done: {"op":"insert"}'),
      fireMsg('m3', {
        observer_id: 'obs_1',
        target: 'harness::send',
        label: 'rx-done',
      }),
    ]
    const out = resolveRegistrations(messages, undefined)
    expect(out.get('m2')?.summary).toBe('from observe call')
  })

  it('prefers the observer id embedded in the entry id over name matching', () => {
    const otherInput = { ...observeInput, label: 'other' }
    const otherOutput = {
      content: [{ type: 'text', text: '{"observer_id":"obs_2"}' }],
    }
    const messages: Message[] = [
      observeCall('m1', observeInput, observeOutput),
      observeCall('m2', otherInput, otherOutput),
      // Fire named rx-done points at obs_2, but the entry id says obs_1.
      fireMsg('m3', {
        observer_id: 'obs_2',
        target: 'harness::send',
        label: 'rx-done',
      }),
      notifMsg('e_notify_obs_1', '[notification] rx-done: {"op":"insert"}'),
    ]
    const out = resolveRegistrations(messages, undefined)
    expect(out.get('e_notify_obs_1')?.detail).toEqual(observeInput)
  })

  it('falls back to the name for an ambiguous ordinal legacy notify id', () => {
    const messages: Message[] = [
      observeCall('m1', observeInput, observeOutput),
      fireMsg('m2', {
        observer_id: 'obs_1',
        target: 'harness::send',
        label: 'rx-done',
      }),
      notifMsg('e_notify_obs_1_7', '[notification] rx-done: {"op":"insert"}'),
    ]
    const out = resolveRegistrations(messages, undefined)
    expect(out.get('e_notify_obs_1_7')?.detail).toEqual(observeInput)
  })

  it('does not correlate a notification to a call-target fire', () => {
    const messages: Message[] = [
      observeCall('m1', observeInput, observeOutput),
      fireMsg('m2', { observer_id: 'obs_1', label: 'rx-done' }), // target db::exec
      notifMsg('m3', '[notification] rx-done: {"op":"insert"}'),
    ]
    const out = resolveRegistrations(messages, undefined)
    expect(out.get('m3')).toBeUndefined()
  })

  it('leaves messages without any source unresolved', () => {
    const messages: Message[] = [fireMsg('m1', { observer_id: 'obs_x' })]
    const out = resolveRegistrations(messages, undefined)
    expect(out.size).toBe(0)
  })
})
