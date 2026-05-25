import { describe, expect, it, vi } from 'vitest';
import { TriggerAction } from '../../src/runtime/iii.js';
import type { ISdk } from '../../src/runtime/iii.js';
import * as persistence from '../../src/turn-orchestrator/persistence.js';
import { newRecord, turnStateKey } from '../../src/turn-orchestrator/state.js';

function fakeIii(): {
  iii: ISdk;
  wakeInvocations: Array<{ session_id: string; function_id: string; action?: unknown }>;
  stateStore: Map<string, unknown>;
} {
  const stateStore = new Map<string, unknown>();
  const wakeInvocations: Array<{ session_id: string; function_id: string; action?: unknown }> = [];
  const iii = {
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
        if (function_id === 'state::get') {
          const p = payload as { scope: string; key: string };
          const v = stateStore.get(`${p.scope}/${p.key}`);
          return v === undefined ? null : structuredClone(v);
        }

        if (function_id === 'state::set') {
          const p = payload as { scope: string; key: string; value: unknown };
          stateStore.set(`${p.scope}/${p.key}`, structuredClone(p.value));
          return null;
        }

        if (function_id === 'state::update') {
          return { old_value: 0 };
        }

        if (function_id.startsWith('turn::')) {
          const p = payload as { session_id: string };
          wakeInvocations.push({ session_id: p.session_id, function_id, action });
          return null;
        }

        return null;
      },
    ),
  };

  return { iii: iii as unknown as ISdk, wakeInvocations, stateStore };
}

describe('saveRecord wake integration', () => {
  it('writing a new stepable turn_state enqueues turn::provisioning', async () => {
    const { iii, wakeInvocations } = fakeIii();
    const rec = newRecord('sess-a');
    rec.state = 'provisioning';

    await persistence.saveRecord(iii, rec);

    expect(wakeInvocations).toEqual([
      {
        session_id: 'sess-a',
        function_id: 'turn::provisioning',
        action: TriggerAction.Enqueue({ queue: 'turn-step' }),
      },
    ]);
  });

  it('subsequent transitions enqueue turn::{newState}', async () => {
    const { iii, wakeInvocations } = fakeIii();
    const rec = newRecord('sess-b');
    rec.state = 'provisioning';
    await persistence.saveRecord(iii, rec);

    rec.state = 'assistant_streaming';
    await persistence.saveRecord(iii, rec);

    expect(wakeInvocations).toEqual([
      {
        session_id: 'sess-b',
        function_id: 'turn::provisioning',
        action: TriggerAction.Enqueue({ queue: 'turn-step' }),
      },
      {
        session_id: 'sess-b',
        function_id: 'turn::assistant_streaming',
        action: TriggerAction.Enqueue({ queue: 'turn-step' }),
      },
    ]);
  });

  it('parking in function_awaiting_approval does NOT wake', async () => {
    const { iii, wakeInvocations } = fakeIii();
    const rec = newRecord('sess-c');
    rec.state = 'function_awaiting_approval';

    await persistence.saveRecord(iii, rec);

    expect(wakeInvocations).toEqual([]);
  });

  it('terminal stopped state does NOT wake', async () => {
    const { iii, wakeInvocations } = fakeIii();
    const rec = newRecord('sess-d');
    rec.state = 'stopped';

    await persistence.saveRecord(iii, rec);

    expect(wakeInvocations).toEqual([]);
  });

  it('same-state re-save does NOT wake', async () => {
    const { iii, wakeInvocations } = fakeIii();
    const rec = newRecord('sess-e');
    rec.state = 'function_execute';
    await persistence.saveRecord(iii, rec);
    wakeInvocations.length = 0;

    await persistence.saveRecord(iii, rec);

    expect(wakeInvocations).toEqual([]);
  });
});

function turnStateGets(iii: ISdk, session_id: string): number {
  const trigger = iii.trigger as unknown as {
    mock: { calls: Array<[{ function_id: string; payload?: { key?: string } }]> };
  };
  return trigger.mock.calls.filter(
    ([arg]) => arg.function_id === 'state::get' && arg.payload?.key === turnStateKey(session_id),
  ).length;
}

describe('saveRecord read elimination (#5)', () => {
  it('2-arg saveRecord reads turn_state exactly once (no double load)', async () => {
    const { iii } = fakeIii();
    const rec = newRecord('sess-r1');
    rec.state = 'provisioning';

    await persistence.saveRecord(iii, rec);

    expect(turnStateGets(iii, 'sess-r1')).toBe(1);
  });

  it('saveRecord with a threaded previous reads turn_state zero times', async () => {
    const { iii } = fakeIii();
    const previous = newRecord('sess-r2');
    previous.state = 'provisioning';
    const next = { ...previous, state: 'assistant_streaming' as const };

    await persistence.saveRecord(iii, next, previous);

    expect(turnStateGets(iii, 'sess-r2')).toBe(0);
  });
});
