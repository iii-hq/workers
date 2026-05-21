import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
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
  it('proceeds with normal teardown without approval::consume resurrection', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'tearing_down' };
    const { iii, calls } = fakeIii();
    vi.spyOn(persistence, 'loadSandboxId').mockResolvedValue(null);
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);

    await handleTearingDown(iii, rec);

    expect(rec.state).toBe('stopped');
    expect(calls.some((c) => c.function_id === 'approval::consume')).toBe(false);
    expect(calls.some((c) => c.function_id === 'stream::set')).toBe(true);
  });

  it('stops the sandbox before ending the agent when a sandbox id exists', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'tearing_down' };
    const { iii, calls } = fakeIii();
    vi.spyOn(persistence, 'loadSandboxId').mockResolvedValue('sandbox-1');
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);

    await handleTearingDown(iii, rec);

    const sandboxCall = calls.find((c) => c.function_id === 'sandbox::stop');
    expect(sandboxCall?.payload).toEqual({ sandbox_id: 'sandbox-1', wait: true });
    expect(sandboxCall?.timeoutMs).toBe(60_000);
    expect(rec.state).toBe('stopped');
  });
});
