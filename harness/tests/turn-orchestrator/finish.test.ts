import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import * as persistence from '../../src/turn-orchestrator/persistence.js';
import { finishSession } from '../../src/turn-orchestrator/finish.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';

describe('finishSession', () => {
  it('emits agent_end with the transcript and sets state to stopped', async () => {
    const messages = [
      { role: 'user' as const, content: [{ type: 'text' as const, text: 'hi' }], timestamp: 1 },
    ];
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue(messages as never);
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
    await finishSession(iii, rec);

    expect(rec.state).toBe('stopped');
    const agentEnd = emitted.find((e) => e.type === 'agent_end');
    expect(agentEnd).toBeDefined();
    expect(agentEnd?.messages).toEqual(messages);
  });
});
