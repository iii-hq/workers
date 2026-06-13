import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ISdk, TriggerConfig, TriggerHandler } from '../../src/runtime/iii.js';
import { createTurnStore } from '../../src/turn-orchestrator/state-runtime/store.js';
import { newRecord, type TurnStateRecord } from '../../src/turn-orchestrator/state.js';
import {
  TURN_COMPLETED_TRIGGER_TYPE,
  emitTurnCompleted,
  registerTurnCompletedTriggerType,
  resetTurnCompletedBindingsForTests,
  terminalStatus,
} from '../../src/turn-orchestrator/turn-completed.js';
import { syntheticAssistant } from '../../src/turn-orchestrator/synthetic-assistant.js';

type Delivery = { function_id: string; payload: unknown };

function fixture(): {
  iii: ISdk;
  deliveries: Delivery[];
  handler: () => TriggerHandler<unknown>;
} {
  const deliveries: Delivery[] = [];
  let handler: TriggerHandler<unknown> | undefined;
  const iii = {
    registerTriggerType: vi.fn((input: { id: string }, h: TriggerHandler<unknown>) => {
      expect(input.id).toBe(TURN_COMPLETED_TRIGGER_TYPE);
      handler = h;
      return {};
    }),
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      if (function_id === 'state::get') return null;
      if (function_id === 'state::set') return { old_value: null, new_value: payload };
      if (function_id === 'stream::set') return null;
      if (function_id === 'state::update') return { old_value: 0 };
      // FSM step wakes (turn::*) are saveRecord's own business, not fan-out.
      if (function_id.startsWith('turn::')) return null;
      deliveries.push({ function_id, payload });
      return null;
    }),
  } as unknown as ISdk;
  return {
    iii,
    deliveries,
    handler: () => {
      if (!handler) throw new Error('trigger type not registered');
      return handler;
    },
  };
}

function binding(id: string, function_id: string, config: unknown): TriggerConfig<unknown> {
  return { id, function_id, config };
}

beforeEach(() => {
  resetTurnCompletedBindingsForTests();
});

describe('terminalStatus', () => {
  it('derives failed / cancelled / completed', () => {
    const rec = newRecord('s1');
    rec.state = 'failed';
    expect(terminalStatus(rec)).toBe('failed');

    rec.state = 'stopped';
    rec.last_assistant = syntheticAssistant({
      stop_reason: 'aborted',
      text: 'run aborted by user',
    });
    expect(terminalStatus(rec)).toBe('cancelled');

    rec.last_assistant = syntheticAssistant({ stop_reason: 'error', text: 'boom' });
    expect(terminalStatus(rec)).toBe('failed');

    rec.last_assistant = null;
    expect(terminalStatus(rec)).toBe('completed');
  });
});

describe('turn_completed fan-out', () => {
  it('delivers to bindings, honoring the session_id filter', async () => {
    const f = fixture();
    registerTurnCompletedTriggerType(f.iii);
    await f.handler().registerTrigger(binding('b1', 'approval::on_turn_completed', {}));
    await f.handler().registerTrigger(binding('b2', 'other::handler', { session_id: 's-other' }));

    await emitTurnCompleted(f.iii, {
      session_id: 's1',
      turn_id: 't_1',
      status: 'completed',
      timestamp: 1,
    });

    expect(f.deliveries).toEqual([
      {
        function_id: 'approval::on_turn_completed',
        payload: { session_id: 's1', turn_id: 't_1', status: 'completed', timestamp: 1 },
      },
    ]);
  });

  it('a failing consumer never throws back into the emitter', async () => {
    const f = fixture();
    registerTurnCompletedTriggerType(f.iii);
    await f.handler().registerTrigger(binding('b1', 'approval::on_turn_completed', {}));
    const trigger = f.iii.trigger as unknown as ReturnType<typeof vi.fn>;
    trigger.mockRejectedValueOnce(new Error('consumer down'));

    await expect(
      emitTurnCompleted(f.iii, {
        session_id: 's1',
        turn_id: 't_1',
        status: 'failed',
        reason: 'boom',
        timestamp: 1,
      }),
    ).resolves.toBeUndefined();
  });
});

describe('saveRecord terminal emission', () => {
  async function saveThrough(
    f: ReturnType<typeof fixture>,
    rec: TurnStateRecord,
    previous?: TurnStateRecord | null,
  ): Promise<void> {
    await createTurnStore(f.iii).saveRecord(rec, previous);
  }

  it('emits once when a turn transitions into stopped', async () => {
    const f = fixture();
    registerTurnCompletedTriggerType(f.iii);
    await f.handler().registerTrigger(binding('b1', 'approval::on_turn_completed', {}));

    const prev = newRecord('s1');
    prev.state = 'finishing';
    const rec = structuredClone(prev);
    rec.state = 'stopped';

    await saveThrough(f, rec, prev);

    expect(f.deliveries).toHaveLength(1);
    expect(f.deliveries[0]?.payload).toMatchObject({
      session_id: 's1',
      turn_id: rec.turn_id,
      status: 'completed',
    });
  });

  it('carries the failure reason on failed transitions', async () => {
    const f = fixture();
    registerTurnCompletedTriggerType(f.iii);
    await f.handler().registerTrigger(binding('b1', 'approval::on_turn_completed', {}));

    const prev = newRecord('s1');
    prev.state = 'function_execute';
    const rec = structuredClone(prev);
    rec.state = 'failed';
    rec.error = { kind: 'transition_error', message: 'boom' };

    await saveThrough(f, rec, prev);

    expect(f.deliveries[0]?.payload).toMatchObject({ status: 'failed', reason: 'boom' });
  });

  it('stays silent on non-terminal saves and terminal rewrites', async () => {
    const f = fixture();
    registerTurnCompletedTriggerType(f.iii);
    await f.handler().registerTrigger(binding('b1', 'approval::on_turn_completed', {}));

    const prev = newRecord('s1');
    prev.state = 'provisioning';
    const streaming = structuredClone(prev);
    streaming.state = 'assistant_streaming';
    await saveThrough(f, streaming, prev);

    const stopped = structuredClone(streaming);
    stopped.state = 'stopped';
    const rewrite = structuredClone(stopped);
    await saveThrough(f, rewrite, stopped);

    expect(f.deliveries).toEqual([]);
  });
});
