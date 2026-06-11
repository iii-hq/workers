import { describe, expect, it, vi } from 'vitest';
import { payloadStoreKey } from '../_helpers/stateStoreKey.js';
import {
  acquireLease,
  acquireLeaseWithWait,
  mintLeaseNonce,
  releaseLease,
} from '../../src/runtime/lease.js';

type Iii = Parameters<typeof acquireLease>[0];

/**
 * In-memory ISdk stub. state::update is atomic by construction — its
 * read-modify-write runs synchronously after the optional latency await, like
 * the engine's store write-lock; latencyMs shifts the interleaving.
 */
function makeStateIii(latencyMs = 0): { iii: Iii; store: Map<string, unknown> } {
  const store = new Map<string, unknown>();
  const maybeWait = () =>
    latencyMs > 0 ? new Promise((r) => setTimeout(r, latencyMs)) : Promise.resolve();

  const iii = {
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      const p = payload as Record<string, unknown>;
      const key = payloadStoreKey(p as { scope?: string; key?: string });
      if (function_id === 'state::get') {
        return store.has(key) ? store.get(key) : null;
      }
      if (function_id === 'state::set') {
        await maybeWait();
        const v = p.value;
        if (v === null || v === undefined) store.delete(key);
        else store.set(key, v);
        return { ok: true };
      }
      if (function_id === 'state::update') {
        await maybeWait();
        const ops = (p.ops ?? []) as Array<{ type: string; value?: unknown }>;
        const old_value = store.has(key) ? store.get(key) : null;
        let new_value: unknown = old_value;
        for (const op of ops) if (op.type === 'set') new_value = op.value;
        if (new_value === null || new_value === undefined) store.delete(key);
        else store.set(key, new_value);
        return { old_value, new_value };
      }
      return null;
    }),
  };
  return { iii: iii as unknown as Iii, store };
}

/** state::update always fails (tolerant wrapper → null), mimicking an outage. */
function makeFailingUpdateIii(): { iii: Iii } {
  const iii = {
    trigger: vi.fn(async ({ function_id }: { function_id: string }) => {
      if (function_id === 'state::update') return null;
      return null;
    }),
  };
  return { iii: iii as unknown as Iii };
}

const SCOPE = 'turn_lease';
const TTL = 30_000;

describe('mintLeaseNonce', () => {
  it('produces unique values across rapid calls', () => {
    const seen = new Set<string>();
    for (let i = 0; i < 1000; i++) seen.add(mintLeaseNonce());
    expect(seen.size).toBe(1000);
  });
});

describe('acquireLease / releaseLease', () => {
  it('grants a free lease and returns a nonce', async () => {
    const { iii } = makeStateIii();
    await expect(acquireLease(iii, SCOPE, 's1', TTL)).resolves.toEqual(expect.any(String));
  });

  it('denies a second acquirer while held', async () => {
    const { iii } = makeStateIii();
    expect(await acquireLease(iii, SCOPE, 's1', TTL)).not.toBeNull();
    await expect(acquireLease(iii, SCOPE, 's1', TTL)).resolves.toBeNull();
  });

  it('re-grants after the owner releases', async () => {
    const { iii } = makeStateIii();
    const nonce = await acquireLease(iii, SCOPE, 's1', TTL);
    expect(nonce).not.toBeNull();
    await releaseLease(iii, SCOPE, 's1', nonce as string);
    await expect(acquireLease(iii, SCOPE, 's1', TTL)).resolves.not.toBeNull();
  });

  it('ignores a release from a non-owner (wrong nonce)', async () => {
    const { iii } = makeStateIii();
    const held = await acquireLease(iii, SCOPE, 's1', TTL);
    expect(held).not.toBeNull();
    await releaseLease(iii, SCOPE, 's1', 'not-the-owner');
    // Lease is still held — a fresh acquire must still fail.
    await expect(acquireLease(iii, SCOPE, 's1', TTL)).resolves.toBeNull();
  });

  it('sends a single state::update set op at the root path', async () => {
    const captured: Array<{ type: string; path?: unknown; value?: unknown }> = [];
    const iii = {
      trigger: vi.fn(
        async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
          if (function_id === 'state::get') return null;
          if (function_id === 'state::update') {
            for (const op of ((payload as Record<string, unknown>).ops ?? []) as Array<{
              type: string;
              path?: unknown;
              value?: unknown;
            }>)
              captured.push(op);
            return { old_value: null, new_value: { dummy: true } };
          }
          return null;
        },
      ),
    } as unknown as Iii;
    const nonce = await acquireLease(iii, SCOPE, 's1', TTL);
    expect(captured).toHaveLength(1);
    expect(captured[0]).toMatchObject({
      type: 'set',
      path: '',
      value: { nonce, ts: expect.any(Number) },
    });
  });
});

describe('acquireLease TTL steal', () => {
  it('steals a lease whose ts is older than the TTL', async () => {
    const { iii, store } = makeStateIii();
    store.set(`${SCOPE}/s1`, { nonce: 'crashed', ts: Date.now() - TTL - 1_000 });
    await expect(acquireLease(iii, SCOPE, 's1', TTL)).resolves.toEqual(expect.any(String));
  });

  it('does not steal a lease still within its TTL', async () => {
    const { iii, store } = makeStateIii();
    store.set(`${SCOPE}/s1`, { nonce: 'fresh', ts: Date.now() });
    await expect(acquireLease(iii, SCOPE, 's1', TTL)).resolves.toBeNull();
  });
});

describe('acquireLease concurrency (the single-writer invariant)', () => {
  it('lets exactly one of two concurrent acquirers win, with write latency', async () => {
    const { iii } = makeStateIii(10);
    const results = await Promise.all([
      acquireLease(iii, SCOPE, 'race', TTL),
      acquireLease(iii, SCOPE, 'race', TTL),
    ]);
    expect(results.filter((r) => r !== null)).toHaveLength(1);
  });

  it('lets exactly one of eight concurrent acquirers win, with write latency', async () => {
    const { iii } = makeStateIii(10);
    const results = await Promise.all(
      Array.from({ length: 8 }, () => acquireLease(iii, SCOPE, 'race8', TTL)),
    );
    expect(results.filter((r) => r !== null)).toHaveLength(1);
  });

  it('never false-wins during a state-store outage (update returns null)', async () => {
    const { iii } = makeFailingUpdateIii();
    await expect(acquireLease(iii, SCOPE, 's1', TTL)).resolves.toBeNull();
  });

  it('returns null for every concurrent acquirer during an outage', async () => {
    const { iii } = makeFailingUpdateIii();
    const results = await Promise.all(
      Array.from({ length: 4 }, () => acquireLease(iii, SCOPE, 'out', TTL)),
    );
    expect(results.every((r) => r === null)).toBe(true);
  });

  it('keeps two independent (scope,key) leases from blocking each other', async () => {
    const { iii } = makeStateIii();
    const a = await acquireLease(iii, 'compaction_lease', 'sid', TTL);
    const b = await acquireLease(iii, 'prune_lease', 'sid', TTL);
    expect(a).not.toBeNull();
    expect(b).not.toBeNull();
    expect(a).not.toBe(b);
  });
});

describe('acquireLeaseWithWait', () => {
  it('acquires immediately when free', async () => {
    const { iii } = makeStateIii();
    await expect(acquireLeaseWithWait(iii, SCOPE, 's1', TTL, 200)).resolves.not.toBeNull();
  });

  it('returns null when the lease never frees within the timeout', async () => {
    const { iii } = makeStateIii();
    expect(await acquireLease(iii, SCOPE, 'busy', TTL)).not.toBeNull();
    await expect(acquireLeaseWithWait(iii, SCOPE, 'busy', TTL, 80)).resolves.toBeNull();
  });

  it('acquires once the holder releases mid-wait', async () => {
    const { iii } = makeStateIii();
    const held = (await acquireLease(iii, SCOPE, 'hand', TTL)) as string;
    setTimeout(() => {
      void releaseLease(iii, SCOPE, 'hand', held);
    }, 30);
    await expect(acquireLeaseWithWait(iii, SCOPE, 'hand', TTL, 1_000)).resolves.not.toBeNull();
  });
});
