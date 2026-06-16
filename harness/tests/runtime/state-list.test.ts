import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { createState } from '../../src/runtime/state.js';

function makeIii(listResult: unknown): ISdk {
  return {
    trigger: vi.fn(async ({ function_id }: { function_id: string }) =>
      function_id === 'state::list' ? listResult : null,
    ),
  } as unknown as ISdk;
}

describe('createState().list', () => {
  it('returns the flat array of stored values (official iii shape)', async () => {
    const rows = [{ session_id: 's1', state: 'stopped' }];
    await expect(createState(makeIii(rows)).list({ scope: 'turn_state' })).resolves.toEqual(rows);
  });

  it('returns [] for non-array responses', async () => {
    await expect(createState(makeIii(null)).list({ scope: 's' })).resolves.toEqual([]);
    await expect(createState(makeIii({ ok: true })).list({ scope: 's' })).resolves.toEqual([]);
    await expect(
      createState(makeIii({ items: [{ id: 'm1' }] })).list({ scope: 's' }),
    ).resolves.toEqual([]);
  });
});
