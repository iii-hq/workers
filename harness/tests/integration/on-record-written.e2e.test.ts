import { describe, expect, it, vi } from 'vitest';
import { TriggerAction } from '../../src/runtime/iii.js';
import type { ISdk } from '../../src/runtime/iii.js';
import { createTurnStore } from '../../src/turn-orchestrator/state-runtime/store.js';
import { TURN_STATE_SCOPE } from '../../src/turn-orchestrator/state.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';

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
          const storeKey = `${p.scope}/${p.key}`;
          const old_value = stateStore.has(storeKey)
            ? structuredClone(stateStore.get(storeKey))
            : null;
          const new_value = structuredClone(p.value);
          stateStore.set(storeKey, new_value);
          return { old_value, new_value };
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
    const store = createTurnStore(iii);
    const rec = newRecord('sess-a');
    rec.state = 'provisioning';

    await store.saveRecord(rec);

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
    const store = createTurnStore(iii);
    const rec = newRecord('sess-b');
    rec.state = 'provisioning';
    await store.saveRecord(rec);

    rec.state = 'assistant_streaming';
    await store.saveRecord(rec);

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
    const store = createTurnStore(iii);
    const rec = newRecord('sess-c');
    rec.state = 'function_awaiting_approval';

    await store.saveRecord(rec);

    expect(wakeInvocations).toEqual([]);
  });

  it('terminal stopped state does NOT wake', async () => {
    const { iii, wakeInvocations } = fakeIii();
    const store = createTurnStore(iii);
    const rec = newRecord('sess-d');
    rec.state = 'stopped';

    await store.saveRecord(rec);

    expect(wakeInvocations).toEqual([]);
  });

  it('same-state re-save does NOT wake', async () => {
    const { iii, wakeInvocations } = fakeIii();
    const store = createTurnStore(iii);
    const rec = newRecord('sess-e');
    rec.state = 'function_execute';
    await store.saveRecord(rec);
    wakeInvocations.length = 0;

    await store.saveRecord(rec);

    expect(wakeInvocations).toEqual([]);
  });
});

function turnStateGets(iii: ISdk, session_id: string): number {
  const trigger = iii.trigger as unknown as {
    mock: {
      calls: Array<[{ function_id: string; payload?: { scope?: string; key?: string } }]>;
    };
  };
  return trigger.mock.calls.filter(
    ([arg]) =>
      arg.function_id === 'state::get' &&
      arg.payload?.scope === TURN_STATE_SCOPE &&
      arg.payload?.key === session_id,
  ).length;
}

describe('turn_state persistence (create-fanout source)', () => {
  it('a newly-created session persists turn_state keyed by session id', async () => {
    const { iii, stateStore } = fakeIii();
    const store = createTurnStore(iii);
    const rec = newRecord('sess-new');
    rec.state = 'provisioning';

    await store.saveRecord(rec);

    expect(stateStore.has(`${TURN_STATE_SCOPE}/sess-new`)).toBe(true);
  });

  it('a transition on an existing session updates the same turn_state key', async () => {
    const { iii, stateStore } = fakeIii();
    const store = createTurnStore(iii);
    const rec = newRecord('sess-x');
    rec.state = 'provisioning';
    await store.saveRecord(rec);

    rec.state = 'assistant_streaming';
    await store.saveRecord(rec);

    expect(stateStore.has(`${TURN_STATE_SCOPE}/sess-x`)).toBe(true);
    expect((stateStore.get(`${TURN_STATE_SCOPE}/sess-x`) as { state: string }).state).toBe(
      'assistant_streaming',
    );
  });

  it('a threaded previous record (transition) keeps one turn_state entry', async () => {
    const { iii, stateStore } = fakeIii();
    const store = createTurnStore(iii);
    const previous = newRecord('sess-y');
    previous.state = 'provisioning';
    const next = { ...previous, state: 'assistant_streaming' as const };

    await store.saveRecord(next, previous);

    expect(stateStore.has(`${TURN_STATE_SCOPE}/sess-y`)).toBe(true);
  });
});

describe('saveRecord read elimination (#5)', () => {
  it('2-arg saveRecord does not pre-read turn_state (uses state::set old_value)', async () => {
    const { iii } = fakeIii();
    const store = createTurnStore(iii);
    const rec = newRecord('sess-r1');
    rec.state = 'provisioning';

    await store.saveRecord(rec);

    expect(turnStateGets(iii, 'sess-r1')).toBe(0);
  });

  it('saveRecord with a threaded previous reads turn_state zero times', async () => {
    const { iii } = fakeIii();
    const store = createTurnStore(iii);
    const previous = newRecord('sess-r2');
    previous.state = 'provisioning';
    const next = { ...previous, state: 'assistant_streaming' as const };

    await store.saveRecord(next, previous);

    expect(turnStateGets(iii, 'sess-r2')).toBe(0);
  });
});
