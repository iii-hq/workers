import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { handleFinishing, register } from '../../src/turn-orchestrator/finishing.js';
import { type TurnStateRecord, newRecord } from '../../src/turn-orchestrator/state.js';

type TriggerCall = { function_id: string; payload: unknown };

function recordingIii(): { iii: ISdk; calls: TriggerCall[] } {
  const calls: TriggerCall[] = [];
  const iii = {
    trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
      calls.push({ function_id: req.function_id, payload: req.payload });
      return null;
    }),
  } as unknown as ISdk;
  return { iii, calls };
}

describe('handleFinishing', () => {
  it('emits agent_end and transitions to stopped', async () => {
    const { iii, calls } = recordingIii();
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'finishing' };

    await handleFinishing(iii, rec);

    expect(rec.state).toBe('stopped');
    const agentEnd = calls.find(
      (c) =>
        c.function_id === 'stream::set' &&
        (c.payload as { data?: { type?: string } }).data?.type === 'agent_end',
    );
    expect(agentEnd).toBeDefined();
  });

  it('is re-runnable: a replay re-emits agent_end and lands on stopped', async () => {
    const { iii, calls } = recordingIii();
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'finishing' };

    await handleFinishing(iii, rec);
    rec.state = 'finishing';
    await handleFinishing(iii, rec);

    expect(rec.state).toBe('stopped');
    const agentEnds = calls.filter(
      (c) =>
        c.function_id === 'stream::set' &&
        (c.payload as { data?: { type?: string } }).data?.type === 'agent_end',
    );
    expect(agentEnds.length).toBe(2);
  });
});

describe('register', () => {
  it('registers turn::finishing', () => {
    const registered = new Map<string, unknown>();
    const iii = {
      registerFunction: (id: string, handler: unknown) => registered.set(id, handler),
    } as unknown as ISdk;

    register(iii);

    expect(registered.has('turn::finishing')).toBe(true);
  });
});
