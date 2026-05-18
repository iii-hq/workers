import { describe, expect, it } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { type TurnStateRecord, newRecord } from '../../src/turn-orchestrator/state.js';
import { handleAwaiting } from '../../src/turn-orchestrator/states/assistant.js';

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

describe('handleAwaiting', () => {
  it('starts a normal assistant turn without approval::consume resurrection', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'awaiting_assistant' };
    const { iii, calls } = fakeIii();

    await handleAwaiting(iii, rec);

    expect(rec.state).toBe('assistant_streaming');
    expect(rec.turn_count).toBe(1);
    expect(rec.turn_end_emitted).toBe(false);
    expect(calls.some((c) => c.function_id === 'approval::consume')).toBe(false);
    expect(calls.some((c) => c.function_id === 'stream::set')).toBe(true);
  });
});
