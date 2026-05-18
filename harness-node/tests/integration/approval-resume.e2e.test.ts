import { describe, expect, it, vi } from 'vitest';
import { handleDecisionWritten } from '../../src/approval-gate/on-decision-written.js';
import type { ISdk } from '../../src/runtime/iii.js';

function fakeIii(): { iii: ISdk; publishes: Array<{ topic: string; data: unknown }> } {
  const stateStore = new Map<string, unknown>();
  const publishes: Array<{ topic: string; data: unknown }> = [];
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
              event_type: old_value == null ? 'created' : 'updated',
              scope: p.scope,
              key: p.key,
              old_value,
              new_value: p.value,
              message_type: 'state',
            });
          });
        }
        return null;
      }

      if (function_id === 'state::get') {
        const p = payload as { scope: string; key: string };
        return stateStore.get(`${p.scope}/${p.key}`) ?? null;
      }

      if (function_id === 'iii::durable::publish') {
        publishes.push(payload as { topic: string; data: unknown });
        return null;
      }

      return null;
    }),
  };

  return { iii: iii as unknown as ISdk, publishes };
}

describe('approval resume reactive trigger', () => {
  it('writing a decision to approvals/<sid>/<cid> automatically publishes turn::step_requested', async () => {
    const { iii, publishes } = fakeIii();

    await iii.trigger({
      function_id: 'state::set',
      payload: {
        scope: 'approvals',
        key: 'sess-x/fc-1',
        value: { decision: 'allow', reason: null },
      },
    });

    await Promise.resolve();

    expect(publishes).toHaveLength(1);
    expect(publishes[0]).toMatchObject({
      topic: 'turn::step_requested',
      data: { session_id: 'sess-x' },
    });
  });
});
