import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import type { AgentMessage } from '../../src/types/agent-message.js';
import * as events from '../../src/turn-orchestrator/events.js';
import * as persistence from '../../src/turn-orchestrator/persistence.js';
import { type TurnStateRecord, newRecord } from '../../src/turn-orchestrator/state.js';
import { handleTearingDown } from '../../src/turn-orchestrator/states/tearing-down.js';

type TriggerCall = { function_id: string; payload: unknown; timeoutMs?: number };

function fakeIii(): { iii: ISdk; calls: TriggerCall[] } {
  const calls: TriggerCall[] = [];
  const iii = {
    trigger: async <T, R>(req: {
      function_id: string;
      payload: T;
      timeoutMs?: number;
    }): Promise<R> => {
      calls.push({
        function_id: req.function_id,
        payload: req.payload,
        timeoutMs: req.timeoutMs,
      });
      return null as R;
    },
  } as unknown as ISdk;
  return { iii, calls };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('handleTearingDown', () => {
  it('transitions to stopped and emits agent_end with session messages', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'tearing_down' };
    const messages: AgentMessage[] = [{ role: 'user', content: 'hi' }];
    const { iii } = fakeIii();
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue(messages);
    const emitSpy = vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleTearingDown(iii, rec);

    expect(rec.state).toBe('stopped');
    expect(emitSpy).toHaveBeenCalledWith(iii, 's1', { type: 'agent_end', messages });
  });
});
