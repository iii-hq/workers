import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { spawnTracesChangedPump } from '../../../src/harness/fanout/traces-changed.js';
import { FanoutState } from '../../../src/harness/ui-subscribe.js';
import type { ISdk } from '../../../src/runtime/iii.js';

type Handler = (event: unknown) => Promise<unknown>;

const HANDLER_FN_ID = 'harness::fanout::traces_changed_handler';

// The traces-changed pump subscribes to the dedicated `agent::turn_end`
// stream and fans out an empty `ui::traces::changed::<browser_id>` signal to
// every all-sessions subscriber, coalescing a burst of turn_end frames into a
// single fan-out. These tests pin the registration shape, the coalescing, the
// all-subscribers targeting, eviction on `function_not_found`, and teardown.
function setup(opts: {
  subscribers?: Array<[string, string | null]>;
  triggerImpl?: (req: { function_id: string; payload: unknown }) => Promise<unknown>;
} = {}) {
  const handlers = new Map<string, Handler>();
  const triggers: Array<{ type?: string; function_id?: string; config?: Record<string, unknown> }> =
    [];
  const sent: Array<{ function_id: string; payload: unknown }> = [];
  const unregistered = { trigger: 0, handler: 0 };
  const iii = {
    registerFunction: vi.fn((id: string, h: Handler) => {
      handlers.set(id, h);
      return {
        unregister() {
          unregistered.handler++;
        },
      };
    }),
    registerTrigger: vi.fn((t) => {
      triggers.push(t);
      return {
        unregister() {
          unregistered.trigger++;
        },
      };
    }),
    trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
      sent.push(req);
      if (opts.triggerImpl) return opts.triggerImpl(req);
      return null;
    }),
  } as unknown as ISdk;

  const state = new FanoutState();
  for (const [b, sid] of opts.subscribers ?? []) state.subscribe(b, sid);
  const stop = spawnTracesChangedPump(iii, state);
  return { handlers, triggers, sent, state, stop, unregistered };
}

function changedCalls(sent: Array<{ function_id: string; payload: unknown }>) {
  return sent.filter((s) => s.function_id.startsWith('ui::traces::changed::'));
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('spawnTracesChangedPump registration', () => {
  it('subscribes to the dedicated agent::turn_end stream', () => {
    const { handlers, triggers } = setup();
    expect([...handlers.keys()]).toContain(HANDLER_FN_ID);
    const t = triggers.find((x) => x.function_id === HANDLER_FN_ID);
    expect(t?.type).toBe('stream');
    expect(t?.config?.stream_name).toBe('agent::turn_end');
  });
});

describe('coalescing', () => {
  it('collapses a burst of turn_end frames into a single fan-out per subscriber', async () => {
    const { handlers, sent } = setup({ subscribers: [['b1', null], ['b2', null]] });
    const handler = handlers.get(HANDLER_FN_ID);

    // Three frames inside the coalescing window.
    await handler?.({ type: 'turn_end' });
    await handler?.({ type: 'turn_end' });
    await handler?.({ type: 'turn_end' });
    expect(changedCalls(sent)).toHaveLength(0); // nothing yet — still debouncing

    await vi.advanceTimersByTimeAsync(400);

    const changed = changedCalls(sent);
    expect(changed.map((c) => c.function_id).sort()).toEqual([
      'ui::traces::changed::b1',
      'ui::traces::changed::b2',
    ]);
    expect(changed[0]?.payload).toEqual({});
  });

  it('fans out again on a frame that arrives after the previous window flushed', async () => {
    const { handlers, sent } = setup({ subscribers: [['b1', null]] });
    const handler = handlers.get(HANDLER_FN_ID);

    await handler?.({ type: 'turn_end' });
    await vi.advanceTimersByTimeAsync(400);
    await handler?.({ type: 'turn_end' });
    await vi.advanceTimersByTimeAsync(400);

    expect(changedCalls(sent)).toHaveLength(2);
  });
});

describe('targeting', () => {
  it('only fans out to all-sessions subscribers', async () => {
    const { handlers, sent } = setup({
      subscribers: [
        ['b1', null], // all-sessions → should receive
        ['b2', 'sess-x'], // specific session → should NOT receive
      ],
    });
    const handler = handlers.get(HANDLER_FN_ID);

    await handler?.({ type: 'turn_end' });
    await vi.advanceTimersByTimeAsync(400);

    expect(changedCalls(sent).map((c) => c.function_id)).toEqual(['ui::traces::changed::b1']);
  });
});

describe('eviction', () => {
  it('evicts a browser whose push rejects with function_not_found', async () => {
    const { handlers, state } = setup({
      subscribers: [['gone', null]],
      triggerImpl: async () => {
        throw new Error('function_not_found: ui::traces::changed::gone');
      },
    });
    const handler = handlers.get(HANDLER_FN_ID);
    expect(state.browserCount()).toBe(1);

    await handler?.({ type: 'turn_end' });
    await vi.advanceTimersByTimeAsync(400);

    expect(state.browserCount()).toBe(0);
  });

  it('does NOT evict on other errors', async () => {
    const { handlers, state } = setup({
      subscribers: [['b1', null]],
      triggerImpl: async () => {
        throw new Error('timeout');
      },
    });
    const handler = handlers.get(HANDLER_FN_ID);

    await handler?.({ type: 'turn_end' });
    await vi.advanceTimersByTimeAsync(400);

    expect(state.browserCount()).toBe(1);
  });
});

describe('teardown', () => {
  it('stop() clears a pending timer so no fan-out fires', async () => {
    const { handlers, sent, stop, unregistered } = setup({ subscribers: [['b1', null]] });
    const handler = handlers.get(HANDLER_FN_ID);

    await handler?.({ type: 'turn_end' });
    stop();
    await vi.advanceTimersByTimeAsync(400);

    expect(changedCalls(sent)).toHaveLength(0);
    expect(unregistered.trigger).toBe(1);
    expect(unregistered.handler).toBe(1);
  });
});
