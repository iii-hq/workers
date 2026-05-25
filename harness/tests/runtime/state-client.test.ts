import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { createState } from '../../src/runtime/state.js';

function makeIii(triggerImpl: (...args: unknown[]) => unknown): ISdk {
  return {
    trigger: vi.fn(triggerImpl),
  } as unknown as ISdk;
}

describe('createState', () => {
  it('tolerant get returns null and does not throw on trigger failure', async () => {
    const iii = makeIii(() => {
      throw new Error('backend down');
    });
    await expect(createState(iii).get({ scope: 's', key: 'k' })).resolves.toBeNull();
  });

  it('strict get propagates trigger failure', async () => {
    const iii = makeIii(() => {
      throw new Error('backend down');
    });
    await expect(
      createState(iii, { tolerant: false }).get({ scope: 's', key: 'k' }),
    ).rejects.toThrow('backend down');
  });

  it('tolerant list returns [] on trigger failure', async () => {
    const iii = makeIii(() => {
      throw new Error('list failed');
    });
    await expect(createState(iii).list({ scope: 's' })).resolves.toEqual([]);
  });

  it('strict list propagates trigger failure', async () => {
    const iii = makeIii(() => {
      throw new Error('list failed');
    });
    await expect(createState(iii, { tolerant: false }).list({ scope: 's' })).rejects.toThrow(
      'list failed',
    );
  });

  it('get normalizes undefined to null', async () => {
    const iii = makeIii(async () => undefined);
    await expect(createState(iii).get({ scope: 's', key: 'missing' })).resolves.toBeNull();
  });

  it('list parses flat arrays from state::list', async () => {
    const rows = [{ id: 'a' }, { id: 'b' }];
    const iii = makeIii(async ({ function_id }: { function_id: string }) => {
      if (function_id === 'state::list') return rows;
      return null;
    });
    await expect(createState(iii).list({ scope: 'agent' })).resolves.toEqual(rows);
  });
});
