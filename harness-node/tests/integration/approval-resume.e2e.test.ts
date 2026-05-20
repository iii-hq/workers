import { describe, expect, it, vi } from 'vitest';
import { handleDecisionWritten } from '../../src/approval-gate/on-decision-written.js';
import type { ISdk } from '../../src/runtime/iii.js';
import {
  handleAbortSignalWrite,
  isAbortSignalWrite,
} from '../../src/turn-orchestrator/on-abort-signal.js';

function fakeIii(): { iii: ISdk; stepTriggers: Array<{ session_id: string }> } {
  const stateStore = new Map<string, unknown>();
  const stepTriggers: Array<{ session_id: string }> = [];
  const iii = {
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      if (function_id === 'state::set') {
        const p = payload as { scope: string; key: string; value: unknown };
        const fullKey = `${p.scope}/${p.key}`;
        const old_value = stateStore.get(fullKey) ?? null;
        stateStore.set(fullKey, p.value);
        if (p.scope === 'approvals') {
          queueMicrotask(() => {
            void handleDecisionWritten(iii as unknown as ISdk, {
              event_type: old_value == null ? 'state:created' : 'state:updated',
              scope: p.scope,
              key: p.key,
              old_value,
              new_value: p.value,
              message_type: 'state',
            });
          });
        }
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

      // abort-signal handler still uses durable publish — capture it here too
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
  it('writing a decision to approvals/<sid>/<cid> automatically triggers turn::step directly', async () => {
    const { iii, stepTriggers } = fakeIii();

    await iii.trigger({
      function_id: 'state::set',
      payload: {
        scope: 'approvals',
        key: 'sess-x/fc-1',
        value: { decision: 'allow', reason: null },
      },
    });

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

    // Seed a previous true so this update sets it to false.
    await iii.trigger({
      function_id: 'state::set',
      payload: { scope: 'agent', key: 'session/sess-clear/abort_signal', value: true },
    });
    await Promise.resolve();
    stepTriggers.length = 0; // drop the first wake

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
