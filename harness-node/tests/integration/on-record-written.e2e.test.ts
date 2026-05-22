import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import {
  handleStepableRecordWrite,
  parseStepableWrite,
} from '../../src/turn-orchestrator/on-record-written.js';

function fakeIii(): { iii: ISdk; stepInvocations: Array<{ session_id: string }> } {
  const stateStore = new Map<string, unknown>();
  const stepInvocations: Array<{ session_id: string }> = [];
  const iii = {
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
          if (parseStepableWrite(event) !== null) {
            queueMicrotask(() => {
              void handleStepableRecordWrite(iii as unknown as ISdk, event);
            });
          }
        }
        return null;
      }

      if (function_id === 'iii::durable::publish') {
        const p = payload as { topic: string; data: { session_id: string } };
        if (p.topic === 'turn::step_requested') {
          stepInvocations.push({ session_id: p.data.session_id });
        }
        return null;
      }

      return null;
    }),
  };

  return { iii: iii as unknown as ISdk, stepInvocations };
}

describe('turn-step reactive wake', () => {
  it('writing session/<sid>/turn_state with a stepable state publishes turn::step_requested', async () => {
    const { iii, stepInvocations } = fakeIii();

    await iii.trigger({
      function_id: 'state::set',
      payload: {
        scope: 'agent',
        key: 'session/sess-a/turn_state',
        value: { state: 'provisioning' },
      },
    });

    await Promise.resolve();

    expect(stepInvocations).toEqual([{ session_id: 'sess-a' }]);
  });

  it('subsequent transitions also publish turn::step_requested', async () => {
    const { iii, stepInvocations } = fakeIii();

    await iii.trigger({
      function_id: 'state::set',
      payload: {
        scope: 'agent',
        key: 'session/sess-b/turn_state',
        value: { state: 'provisioning' },
      },
    });
    await Promise.resolve();

    await iii.trigger({
      function_id: 'state::set',
      payload: {
        scope: 'agent',
        key: 'session/sess-b/turn_state',
        value: { state: 'awaiting_assistant' },
      },
    });
    await Promise.resolve();

    expect(stepInvocations).toEqual([{ session_id: 'sess-b' }, { session_id: 'sess-b' }]);
  });

  it('parking in function_awaiting_approval does NOT wake', async () => {
    const { iii, stepInvocations } = fakeIii();

    await iii.trigger({
      function_id: 'state::set',
      payload: {
        scope: 'agent',
        key: 'session/sess-c/turn_state',
        value: { state: 'function_awaiting_approval' },
      },
    });
    await Promise.resolve();

    expect(stepInvocations).toEqual([]);
  });

  it('terminal stopped state does NOT wake', async () => {
    const { iii, stepInvocations } = fakeIii();

    await iii.trigger({
      function_id: 'state::set',
      payload: {
        scope: 'agent',
        key: 'session/sess-d/turn_state',
        value: { state: 'stopped' },
      },
    });
    await Promise.resolve();

    expect(stepInvocations).toEqual([]);
  });

  it('non-turn_state agent keys do NOT wake (no leakage from abort_signal etc.)', async () => {
    const { iii, stepInvocations } = fakeIii();

    await iii.trigger({
      function_id: 'state::set',
      payload: {
        scope: 'agent',
        key: 'session/sess-e/abort_signal',
        value: true,
      },
    });
    await Promise.resolve();

    expect(stepInvocations).toEqual([]);
  });
});
