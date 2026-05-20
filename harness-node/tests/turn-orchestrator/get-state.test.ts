import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { execute } from '../../src/turn-orchestrator/get-state.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';

describe('turn::get_state', () => {
  it('returns the turn_state record for a known session via persistence.loadRecord', async () => {
    const rec = newRecord('sess-abc');
    rec.state = 'function_awaiting_approval';
    const iii = {
      trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
        if (
          req.function_id === 'state::get' &&
          (req.payload as Record<string, unknown>).key === 'session/sess-abc/turn_state'
        ) {
          return rec;
        }
        return null;
      }),
    } as unknown as ISdk;

    const out = await execute(iii, { session_id: 'sess-abc' });
    expect(out).toEqual(rec);
  });

  it('returns null when no record exists for the session', async () => {
    const iii = {
      trigger: vi.fn(async () => null),
    } as unknown as ISdk;
    const out = await execute(iii, { session_id: 'unknown' });
    expect(out).toBeNull();
  });

  it('throws on missing session_id', async () => {
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    await expect(execute(iii, {})).rejects.toThrow(/session_id/);
  });
});
