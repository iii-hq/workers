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

function fakeIii(): { iii: ISdk; stepTriggers: Array<{ session_id: string }> } {
  const stateStore = new Map<string, unknown>();
  const stepTriggers: Array<{ session_id: string }> = [];
  const handlers = new Map<string, (payload: unknown) => Promise<unknown>>();

  const iii = {
    registerFunction: vi.fn((fnId: string, handler: (payload: unknown) => Promise<unknown>) => {
      handlers.set(fnId, handler);
      return { unregister: vi.fn() };
    }),
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
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

      if (function_id === 'turn::step') {
        stepTriggers.push(payload as { session_id: string });
        return null;
      }

      const handler = handlers.get(function_id);
      if (handler) {
        await handler(payload);
        return null;
      }

      if (function_id === 'iii::durable::publish') {
        const p = payload as { topic: string; data: { session_id: string } };
        if (p.topic === 'turn::step_requested') {
          stepTriggers.push({ session_id: p.data.session_id });
        }
        return null;
      }

      return null;
    }),
  };

  return { iii: iii as unknown as ISdk, stepTriggers };
}

describe('approval resume reactive trigger', () => {
  afterEach(() => {
    clearApprovalResumeRegistry();
  });

  it('approval::resolve via resume fn automatically triggers turn::step', async () => {
    const { iii, stepTriggers } = fakeIii();
    registerApprovalResume(iii, 'sess-x', 'fc-1');

    const out = await handleResolveRequest(iii, {
      session_id: 'sess-x',
      function_call_id: 'fc-1',
      decision: 'allow',
    });
    expect(out).toEqual({ ok: true });

    await Promise.resolve();

    expect(stepTriggers).toHaveLength(1);
    expect(stepTriggers[0]).toMatchObject({ session_id: 'sess-x' });
  });

  it('writing session/<sid>/abort_signal=true wakes turn::step (via durable publish)', async () => {
    const { iii, stepTriggers } = fakeIii();

    await iii.trigger({
      function_id: 'state::set',
      payload: {
        scope: 'agent',
        key: 'session/sess-abort/abort_signal',
        value: true,
      },
    });

    await Promise.resolve();

    expect(stepTriggers).toHaveLength(1);
    expect(stepTriggers[0]).toMatchObject({ session_id: 'sess-abort' });
  });

  it('writing session/<sid>/abort_signal=false does NOT trigger (condition rejects clears)', async () => {
    const { iii, stepTriggers } = fakeIii();

    await iii.trigger({
      function_id: 'state::set',
      payload: { scope: 'agent', key: 'session/sess-clear/abort_signal', value: true },
    });
    await Promise.resolve();
    stepTriggers.length = 0;

    await iii.trigger({
      function_id: 'state::set',
      payload: { scope: 'agent', key: 'session/sess-clear/abort_signal', value: false },
    });
    await Promise.resolve();

    expect(stepTriggers).toHaveLength(0);
  });

  it('writing an unrelated agent-scope key does NOT trigger', async () => {
    const { iii, stepTriggers } = fakeIii();

    await iii.trigger({
      function_id: 'state::set',
      payload: {
        scope: 'agent',
        key: 'session/sess-x/turn_state',
        value: { state: 'function_execute' },
      },
    });
    await Promise.resolve();

    expect(stepTriggers).toHaveLength(0);
  });
});
