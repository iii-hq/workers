import { describe, expect, it } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { FUNCTION_ID, SYNC_FUNCTION_ID, execute } from '../../src/turn-orchestrator/run-start.js';

type TriggerCall = { function_id: string; payload: unknown };

function fakeIii(): { iii: ISdk; calls: TriggerCall[] } {
  const calls: TriggerCall[] = [];
  const iii = {
    trigger: async <T, R>(req: { function_id: string; payload: T }): Promise<R> => {
      calls.push({ function_id: req.function_id, payload: req.payload });
      return null as R;
    },
  } as unknown as ISdk;
  return { iii, calls };
}

describe('run-start constants', () => {
  it('exposes only start function ids', () => {
    expect(FUNCTION_ID).toBe('run::start');
    expect(SYNC_FUNCTION_ID).toBe('run::start_and_wait');
  });
});

describe('execute', () => {
  it('saves initial session state and publishes the first step', async () => {
    const { iii, calls } = fakeIii();

    await execute(iii, {
      session_id: 's1',
      provider: 'openai',
      model: 'gpt-test',
      messages: [{ role: 'user', content: [{ type: 'text', text: 'hi' }], timestamp: 1 }],
    });

    const stateSets = calls.filter((c) => c.function_id === 'state::set');
    expect(stateSets.length).toBeGreaterThan(0);
    const publish = calls.find((c) => c.function_id === 'iii::durable::publish');
    expect(publish?.payload).toEqual({
      topic: 'turn::step_requested',
      data: { session_id: 's1' },
    });
  });
});
