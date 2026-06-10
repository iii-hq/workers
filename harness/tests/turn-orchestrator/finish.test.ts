import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { createTurnStatePorts } from '../../src/turn-orchestrator/state-runtime/ports.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';
import { installMockTurnStore } from './_helpers/mockTurnStore.js';

describe('TurnStatePorts.finishSession', () => {
  it('emits agent_end as a signal (no transcript reload) and sets state to stopped', async () => {
    const store = installMockTurnStore();
    const emitted: Array<{ type: string; messages?: unknown }> = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
        if (req.function_id === 'stream::set') {
          emitted.push((req.payload as { data: { type: string; messages?: unknown } }).data);
        }
        return null;
      }),
    } as unknown as ISdk;

    const rec = newRecord('s1');
    rec.state = 'finishing';
    await createTurnStatePorts(iii).finishSession(rec);

    expect(rec.state).toBe('stopped');
    const agentEnd = emitted.find((e) => e.type === 'agent_end');
    expect(agentEnd).toBeDefined();
    // agent_end is a turn-end signal; no consumer reads .messages, so the
    // session is no longer reloaded just to populate it.
    expect(agentEnd?.messages).toEqual([]);
    expect(store.loadMessages).not.toHaveBeenCalled();
  });
});
