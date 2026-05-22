import { afterEach, describe, expect, it, vi } from 'vitest';
import { handleResolveRequest } from '../../src/approval-gate/resolve.js';
import {
  clearApprovalResumeRegistry,
  registerApprovalResume,
} from '../../src/turn-orchestrator/approval-resume.js';
import {
  handleAbortSignalWrite,
  isAbortSignalWrite,
} from '../../src/turn-orchestrator/on-abort-signal.js';
import type { ISdk } from '../../src/runtime/iii.js';
import { newRecord, turnStateKey } from '../../src/turn-orchestrator/state.js';

async function flushMicrotasks(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
}

function fakeIii(): {
  iii: ISdk;
  wakeTriggers: Array<{ session_id: string; function_id: string }>;
  stateStore: Map<string, unknown>;
} {
  const stateStore = new Map<string, unknown>();
  const wakeTriggers: Array<{ session_id: string; function_id: string }> = [];
  const handlers = new Map<string, (payload: unknown) => Promise<unknown>>();

  const iii = {
    registerFunction: vi.fn((fnId: string, handler: (payload: unknown) => Promise<unknown>) => {
      handlers.set(fnId, handler);
      return { unregister: vi.fn() };
    }),
    trigger: vi.fn(
      async ({
        function_id,
        payload,
        action,
      }: {
        function_id: string;
        payload: unknown;
        action?: unknown;
      }) => {
        if (function_id === 'state::set') {
          const p = payload as { scope: string; key: string; value: unknown };
          const fullKey = `${p.scope}/${p.key}`;
          const old_value = stateStore.get(fullKey) ?? null;
          stateStore.set(fullKey, p.value);
          if (p.scope === 'agent') {
            const event = {
              event_type: old_value == null ? 'state:created' : 'state:updated',
              scope: p.scope,
              key: p.key,
              old_value,
              new_value: p.value,
              message_type: 'state',
            };
            if (isAbortSignalWrite(event)) {
              queueMicrotask(() => {
                void handleAbortSignalWrite(iii as unknown as ISdk, event);
              });
            }
          }
          return null;
        }

        if (function_id === 'state::get') {
          const p = payload as { scope: string; key: string };
          return stateStore.get(`${p.scope}/${p.key}`) ?? null;
        }

        if (function_id.startsWith('turn::') && action != null) {
          const p = payload as { session_id: string };
          wakeTriggers.push({ session_id: p.session_id, function_id });
          return null;
        }

        const handler = handlers.get(function_id);
        if (handler) {
          await handler(payload);
          return null;
        }

        return null;
      },
    ),
  };

  return { iii: iii as unknown as ISdk, wakeTriggers, stateStore };
}

describe('approval resume reactive trigger', () => {
  afterEach(() => {
    clearApprovalResumeRegistry();
  });

  it('approval::resolve via resume fn automatically enqueues turn::{state}', async () => {
    const { iii, wakeTriggers, stateStore } = fakeIii();
    const rec = newRecord('sess-x');
    rec.state = 'function_awaiting_approval';
    stateStore.set(`agent/${turnStateKey('sess-x')}`, rec);
    registerApprovalResume(iii, 'sess-x', 'fc-1');

    const out = await handleResolveRequest(iii, {
      session_id: 'sess-x',
      function_call_id: 'fc-1',
      decision: 'allow',
    });
    expect(out).toEqual({ ok: true });

    await flushMicrotasks();

    expect(wakeTriggers).toHaveLength(1);
    expect(wakeTriggers[0]).toMatchObject({
      session_id: 'sess-x',
      function_id: 'turn::function_awaiting_approval',
    });
  });

  it('writing session/<sid>/abort_signal=true enqueues turn::{state}', async () => {
    const { iii, wakeTriggers, stateStore } = fakeIii();
    const rec = newRecord('sess-abort');
    rec.state = 'assistant_streaming';
    stateStore.set(`agent/${turnStateKey('sess-abort')}`, rec);

    await iii.trigger({
      function_id: 'state::set',
      payload: {
        scope: 'agent',
        key: 'session/sess-abort/abort_signal',
        value: true,
      },
    });

    await flushMicrotasks();

    expect(wakeTriggers).toHaveLength(1);
    expect(wakeTriggers[0]).toMatchObject({
      session_id: 'sess-abort',
      function_id: 'turn::assistant_streaming',
    });
  });

  it('writing session/<sid>/abort_signal=false does NOT trigger (condition rejects clears)', async () => {
    const { iii, wakeTriggers, stateStore } = fakeIii();
    const rec = newRecord('sess-clear');
    rec.state = 'function_execute';
    stateStore.set(`agent/${turnStateKey('sess-clear')}`, rec);

    await iii.trigger({
      function_id: 'state::set',
      payload: { scope: 'agent', key: 'session/sess-clear/abort_signal', value: true },
    });
    await flushMicrotasks();
    wakeTriggers.length = 0;

    await iii.trigger({
      function_id: 'state::set',
      payload: { scope: 'agent', key: 'session/sess-clear/abort_signal', value: false },
    });
    await flushMicrotasks();

    expect(wakeTriggers).toHaveLength(0);
  });

  it('writing an unrelated agent-scope key does NOT trigger', async () => {
    const { iii, wakeTriggers } = fakeIii();

    await iii.trigger({
      function_id: 'state::set',
      payload: {
        scope: 'agent',
        key: 'session/sess-x/turn_state',
        value: { state: 'function_execute' },
      },
    });
    await Promise.resolve();

    expect(wakeTriggers).toHaveLength(0);
  });
});
