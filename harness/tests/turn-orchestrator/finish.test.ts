import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { createTurnStatePorts } from '../../src/turn-orchestrator/state-runtime/ports.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';
import { installMockTurnStore } from './_helpers/mockTurnStore.js';

describe('TurnStatePorts.finishSession', () => {
  it('emits agent_end with the transcript and sets state to stopped', async () => {
    const messages = [
      { role: 'user' as const, content: [{ type: 'text' as const, text: 'hi' }], timestamp: 1 },
    ];
    installMockTurnStore({ loadMessages: vi.fn(async () => messages) });
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
    rec.state = 'steering_check';
    await createTurnStatePorts(iii).finishSession(rec);

    expect(rec.state).toBe('stopped');
    const agentEnd = emitted.find((e) => e.type === 'agent_end');
    expect(agentEnd).toBeDefined();
    expect(agentEnd?.messages).toEqual(messages);
  });
});
