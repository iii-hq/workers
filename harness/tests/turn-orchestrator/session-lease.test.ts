import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import {
  acquireSessionLease,
  releaseSessionLease,
} from '../../src/turn-orchestrator/state-runtime/session-lease.js';

/**
 * Fake iii with an in-memory atomic state::update set-CAS (plus get/set).
 * `failUpdate` makes update throw → tolerant client returns null, exercising the
 * outage path that must never false-win.
 */
function makeIii(opts: { failUpdate?: boolean } = {}): ISdk {
  const store = new Map<string, unknown>();
  const id = (p: { scope: string; key: string }) => `${p.scope}/${p.key}`;

  const trigger = vi.fn(
    async ({ function_id, payload }: { function_id: string; payload: Record<string, unknown> }) => {
      const k = id(payload as { scope: string; key: string });
      if (function_id === 'state::get') return store.has(k) ? store.get(k) : null;
      if (function_id === 'state::set') {
        if (payload.value === null || payload.value === undefined) store.delete(k);
        else store.set(k, payload.value);
        return { ok: true };
      }
      if (function_id === 'state::update') {
        if (opts.failUpdate) throw new Error('state-store down');
        const old_value = store.has(k) ? store.get(k) : null;
        let new_value: unknown = old_value;
        for (const op of (payload.ops ?? []) as Array<{ type: string; value?: unknown }>) {
          if (op.type === 'set') new_value = op.value;
        }
        if (new_value === null || new_value === undefined) store.delete(k);
        else store.set(k, new_value);
        return { old_value, new_value };
      }
      throw new Error(`unexpected function_id ${function_id}`);
    },
  );

  return { trigger } as unknown as ISdk;
}

describe('acquireSessionLease', () => {
  it('grants the lease on a fresh key and returns a nonce', async () => {
    const iii = makeIii();
    await expect(acquireSessionLease(iii, 's1')).resolves.toEqual(expect.any(String));
  });

  it('denies a second concurrent holder', async () => {
    const iii = makeIii();
    expect(await acquireSessionLease(iii, 's1')).not.toBeNull();
    await expect(acquireSessionLease(iii, 's1')).resolves.toBeNull();
  });

  it('re-grants after the owner releases', async () => {
    const iii = makeIii();
    const nonce = await acquireSessionLease(iii, 's1');
    expect(nonce).not.toBeNull();
    await releaseSessionLease(iii, 's1', nonce as string);
    await expect(acquireSessionLease(iii, 's1')).resolves.not.toBeNull();
  });

  it('ignores a release from a non-owner (wrong nonce)', async () => {
    const iii = makeIii();
    expect(await acquireSessionLease(iii, 's1')).not.toBeNull();
    await releaseSessionLease(iii, 's1', 'someone-else');
    await expect(acquireSessionLease(iii, 's1')).resolves.toBeNull();
  });

  // Regression: a transient state-store outage must NOT be read as a win, or
  // every concurrent contender claims the lease at once → duplicate side effects.
  it('does not grant the lease when the atomic update fails', async () => {
    const iii = makeIii({ failUpdate: true });
    await expect(acquireSessionLease(iii, 's1')).resolves.toBeNull();
  });

  it('never lets two contenders both win under a state-store outage', async () => {
    const iii = makeIii({ failUpdate: true });
    const [a, b] = await Promise.all([
      acquireSessionLease(iii, 's1'),
      acquireSessionLease(iii, 's1'),
    ]);
    expect([a, b]).toEqual([null, null]);
  });
});
