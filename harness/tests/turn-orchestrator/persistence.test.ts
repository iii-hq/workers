import { describe, it, expect, vi } from 'vitest';
import * as persistence from '../../src/turn-orchestrator/persistence.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';

describe('writeRecord', () => {
  it('writes turn_state without emitting turn_state_changed', async () => {
    const calls: string[] = [];
    const iii = {
      trigger: vi.fn(async ({ function_id }: any) => {
        calls.push(function_id);
        return null;
      }),
    } as any;
    await persistence.writeRecord(iii, newRecord('s1'));
    expect(calls).toContain('state::set');
    expect(calls).not.toContain('stream::set'); // no agent::events emit
  });
});
