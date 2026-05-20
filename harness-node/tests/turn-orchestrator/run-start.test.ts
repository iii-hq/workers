import { describe, expect, it } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { execute } from '../../src/turn-orchestrator/run-start.js';

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

describe('execute', () => {
  it('saves initial session state to wake the reactive step trigger', async () => {
    const { iii, calls } = fakeIii();

    await execute(iii, {
      session_id: 's1',
      provider: 'openai',
      model: 'gpt-test',
      messages: [{ role: 'user', content: [{ type: 'text', text: 'hi' }], timestamp: 1 }],
    });

    const turnStateSet = calls.find(
      (c) =>
        c.function_id === 'state::set' &&
        (c.payload as { scope?: string; key?: string }).scope === 'agent' &&
        (c.payload as { scope?: string; key?: string }).key === 'session/s1/turn_state',
    );
    expect(turnStateSet).toBeDefined();
    expect((turnStateSet?.payload as { value: { state: string } }).value.state).toBe(
      'provisioning',
    );

    const publish = calls.find((c) => c.function_id === 'iii::durable::publish');
    expect(publish).toBeUndefined();
  });
});
