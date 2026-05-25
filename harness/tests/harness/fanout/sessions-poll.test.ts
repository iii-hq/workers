import { describe, expect, it, vi } from 'vitest';
import { spawnSessionsPoll } from '../../../src/harness/fanout/sessions-poll.js';
import { FanoutState } from '../../../src/harness/ui-subscribe.js';
import type { ISdk } from '../../../src/runtime/iii.js';

type Handler = (event: unknown) => Promise<unknown>;

// Session-create fanout now rides a dedicated `session_index` scope (marker
// written once at creation). The state trigger has NO condition_function_id, so
// the engine hands every write on that scope to the handler — the handler is
// the sole gate. These tests hammer that gate and the registration shape.
function setup(subscribers: string[] = []) {
  const handlers = new Map<string, Handler>();
  const triggers: Array<{ type?: string; function_id?: string; config?: Record<string, unknown> }> =
    [];
  const sent: Array<{ function_id: string; payload: unknown }> = [];
  const iii = {
    registerFunction: vi.fn((id: string, h: Handler) => {
      handlers.set(id, h);
      return { unregister() {} };
    }),
    registerTrigger: vi.fn((t) => {
      triggers.push(t);
      return { unregister() {} };
    }),
    trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
      sent.push(req);
      return null;
    }),
  } as unknown as ISdk;

  const state = new FanoutState();
  for (const b of subscribers) state.subscribe(b, null);
  spawnSessionsPoll(iii, state);
  return { handlers, triggers, sent };
}

const createEvent = (over: Record<string, unknown> = {}) => ({
  event_type: 'state:created' as const,
  scope: 'session_index' as const,
  key: 'sess-1',
  old_value: null,
  new_value: { created_at_ms: 1 },
  message_type: 'state',
  ...over,
});

function changedCalls(sent: Array<{ function_id: string; payload: unknown }>) {
  return sent.filter((s) => s.function_id.startsWith('ui::sessions::changed::'));
}

describe('spawnSessionsPoll registration (eliminates the per-write predicate RPC)', () => {
  it('registers a scope-only session_index trigger with NO condition_function_id and no predicate fn', () => {
    const { handlers, triggers } = setup();

    expect([...handlers.keys()]).not.toContain('harness::session::is_create_event');
    expect([...handlers.keys()]).toContain('harness::fanout::session_created');

    const t = triggers.find((x) => x.function_id === 'harness::fanout::session_created');
    expect(t?.type).toBe('state');
    expect(t?.config?.scope).toBe('session_index');
    expect(t?.config?.condition_function_id).toBeUndefined();
  });
});

describe('session_created handler (sole gate)', () => {
  it('fans out the new session id to every all-sessions subscriber', async () => {
    const { handlers, sent } = setup(['b1', 'b2']);
    const handler = handlers.get('harness::fanout::session_created');

    await handler?.(createEvent({ key: 'sess-1' }));

    const changed = changedCalls(sent);
    expect(changed.map((c) => c.function_id).sort()).toEqual([
      'ui::sessions::changed::b1',
      'ui::sessions::changed::b2',
    ]);
    expect(changed[0]?.payload).toEqual({ added: ['sess-1'], removed: [] });
  });

  it.each([
    ['state:updated (not a new session)', { event_type: 'state:updated' }],
    // The dangerous one: a delete must NOT report the session as "added".
    ['state:deleted (removed session)', { event_type: 'state:deleted', new_value: null }],
    ['empty key', { key: '' }],
  ])('does NOT fan out on %s', async (_label, over) => {
    const { handlers, sent } = setup(['b1']);
    const handler = handlers.get('harness::fanout::session_created');

    await handler?.(createEvent(over));

    expect(changedCalls(sent)).toHaveLength(0);
  });
});
