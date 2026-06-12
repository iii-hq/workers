import { describe, expect, it, vi } from 'vitest';
import { payloadStoreKey, stateStoreKey } from '../_helpers/stateStoreKey.js';
import {
  LEASE_TTL_SECS,
  mintLeaseNonce,
  readLeaseTimestampSecs,
  acquireLease,
  acquireLeaseWithWait,
} from '../../src/context-compaction/lease.js';

describe('lease helpers', () => {
  it('stateStoreKey namespaces compaction lease by session', () => {
    const k = stateStoreKey('compaction_lease', 's9');
    expect(k).toBe('compaction_lease/s9');
  });

  it('mintLeaseNonce produces unique values across rapid calls', () => {
    const seen = new Set<string>();
    for (let i = 0; i < 1000; i++) seen.add(mintLeaseNonce());
    expect(seen.size).toBe(1000);
  });

  it('readLeaseTimestampSecs reads new {nonce, ts} shape', () => {
    expect(readLeaseTimestampSecs({ nonce: 'a', ts: 1_700_000_000_000 })).toBe(1_700_000_000);
  });

  it('readLeaseTimestampSecs treats bare-int values as inactive', () => {
    expect(readLeaseTimestampSecs(1_700_000_000)).toBe(0);
  });

  it('readLeaseTimestampSecs returns 0 for garbage', () => {
    expect(readLeaseTimestampSecs(null)).toBe(0);
    expect(readLeaseTimestampSecs('abc')).toBe(0);
    expect(readLeaseTimestampSecs({ unrelated: true })).toBe(0);
  });

  it('LEASE_TTL_SECS exceeds the summariser timeout (120s)', () => {
    expect(LEASE_TTL_SECS).toBeGreaterThan(120);
  });
});

// Minimal in-memory ISdk stub for lease integration tests.
// state::update is atomic by construction (Map ops are synchronous), which
// matches what we need from iii-database in production.
function makeStateIii() {
  const store = new Map<string, unknown>();
  const iii = {
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      const p = payload as Record<string, unknown>;
      if (function_id === 'state::get') {
        const v = store.get(payloadStoreKey(p as { scope?: string; key?: string }));
        return v !== undefined ? v : null;
      }
      if (function_id === 'state::set') {
        const v = p.value;
        const key = payloadStoreKey(p as { scope?: string; key?: string });
        if (v === null || v === undefined) {
          store.delete(key);
        } else {
          store.set(key, v);
        }
        return { ok: true };
      }
      if (function_id === 'state::update') {
        const key = payloadStoreKey(p as { scope?: string; key?: string });
        const ops = (p.ops ?? []) as Array<{ type: string; value?: unknown }>;
        const oldValue = store.has(key) ? store.get(key) : null;
        let newValue: unknown = oldValue;
        for (const op of ops) {
          if (op.type === 'set') newValue = op.value;
        }
        if (newValue === null || newValue === undefined) {
          store.delete(key);
        } else {
          store.set(key, newValue);
        }
        return { old_value: oldValue ?? null, new_value: newValue ?? null };
      }
      return null;
    }),
  };
  return { iii: iii as unknown as Parameters<typeof acquireLease>[0], store };
}

describe('lease kinds', () => {
  it('compaction and prune leases are independent for the same session', async () => {
    const { iii } = makeStateIii();
    const nonce1 = await acquireLease(iii, 'sid', 'compaction');
    const nonce2 = await acquireLease(iii, 'sid', 'prune');
    // Both should succeed — they use different state keys.
    expect(nonce1).not.toBeNull();
    expect(nonce2).not.toBeNull();
    // And they must be distinct nonces.
    expect(nonce1).not.toBe(nonce2);
  });

  it('stateStoreKey produces different keys for compaction vs prune', () => {
    const k1 = stateStoreKey('compaction_lease', 'sess1');
    const k2 = stateStoreKey('prune_lease', 'sess1');
    expect(k1).toBe('compaction_lease/sess1');
    expect(k2).toBe('prune_lease/sess1');
    expect(k1).not.toBe(k2);
  });
});

describe('acquireLeaseWithWait', () => {
  it('returns null when lease never frees up within timeout', async () => {
    const { iii } = makeStateIii();
    // Pre-occupy the compaction lease so subsequent acquires fail.
    const first = await acquireLease(iii, 'sid2', 'compaction');
    expect(first).not.toBeNull();

    // Now acquireLeaseWithWait should give up quickly (50 ms timeout).
    const result = await acquireLeaseWithWait(iii, 'sid2', 'compaction', 50);
    expect(result).toBeNull();
  });
});

// Adds artificial latency to BOTH state::set and state::update, mimicking
// a real state store (iii-database over IPC, remote Redis). The key
// invariant is that state::update remains atomic — read+write inside a
// single async function tick — which is what iii-database guarantees.
function makeRacyStateIii(writeLatencyMs: number) {
  const store = new Map<string, unknown>();
  const iii = {
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      const p = payload as Record<string, unknown>;
      if (function_id === 'state::get') {
        const v = store.get(payloadStoreKey(p as { scope?: string; key?: string }));
        return v !== undefined ? v : null;
      }
      if (function_id === 'state::set') {
        await new Promise((r) => setTimeout(r, writeLatencyMs));
        const v = p.value;
        const key = payloadStoreKey(p as { scope?: string; key?: string });
        if (v === null || v === undefined) {
          store.delete(key);
        } else {
          store.set(key, v);
        }
        return { ok: true };
      }
      if (function_id === 'state::update') {
        await new Promise((r) => setTimeout(r, writeLatencyMs));
        const key = payloadStoreKey(p as { scope?: string; key?: string });
        const ops = (p.ops ?? []) as Array<{ type: string; value?: unknown }>;
        const oldValue = store.has(key) ? store.get(key) : null;
        let newValue: unknown = oldValue;
        for (const op of ops) {
          if (op.type === 'set') newValue = op.value;
        }
        if (newValue === null || newValue === undefined) {
          store.delete(key);
        } else {
          store.set(key, newValue);
        }
        return { old_value: oldValue ?? null, new_value: newValue ?? null };
      }
      return null;
    }),
  };
  return { iii: iii as unknown as Parameters<typeof acquireLease>[0], store };
}

// Simulates a transient state-store outage: state::update fails (returns
// null) while state::get still works. acquireLease must refuse to claim
// the lease in this case — otherwise every concurrent caller becomes a
// "winner" during the outage and the single-writer invariant breaks.
function makeFailingUpdateIii() {
  const store = new Map<string, unknown>();
  const iii = {
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      const p = payload as Record<string, unknown>;
      if (function_id === 'state::get') {
        const v = store.get(payloadStoreKey(p as { scope?: string; key?: string }));
        return v !== undefined ? v : null;
      }
      if (function_id === 'state::set') {
        const v = p.value;
        const key = payloadStoreKey(p as { scope?: string; key?: string });
        if (v === null || v === undefined) {
          store.delete(key);
        } else {
          store.set(key, v);
        }
        return { ok: true };
      }
      if (function_id === 'state::update') {
        // Simulate the stateUpdate wrapper's error path: backend failed,
        // wrapper swallowed and returned null.
        return null;
      }
      return null;
    }),
  };
  return { iii: iii as unknown as Parameters<typeof acquireLease>[0], store };
}

describe('acquireLease wire shape', () => {
  it('sends state::update with path: "" (root) on the set op', async () => {
    // Regression: the engine UpdateOp::Set requires a path field
    // (FieldPath; "" = root). A previous version of the lease shipped
    // ops without `path`, the engine failed to deserialize, the wrapper
    // returned null, and every /compact bailed in production.
    const captured: Array<{ type: string; path?: unknown; value?: unknown }> = [];
    const iii = {
      trigger: vi.fn(
        async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
          const p = payload as Record<string, unknown>;
          if (function_id === 'state::get') return null;
          if (function_id === 'state::update') {
            const ops = (p.ops ?? []) as Array<{
              type: string;
              path?: unknown;
              value?: unknown;
            }>;
            for (const op of ops) captured.push(op);
            return { old_value: null, new_value: { dummy: true } };
          }
          return null;
        },
      ),
    };
    const nonce = await acquireLease(
      iii as unknown as Parameters<typeof acquireLease>[0],
      'wireshape-sid',
      'compaction',
    );
    expect(nonce).not.toBeNull();
    expect(captured).toHaveLength(1);
    expect(captured[0]?.type).toBe('set');
    expect(captured[0]?.path).toBe('');
    expect(captured[0]?.value).toMatchObject({ nonce, ts: expect.any(Number) });
  });
});

describe('acquireLease state-store failures', () => {
  it('returns null when state::update fails (does not falsely claim the lease)', async () => {
    const { iii } = makeFailingUpdateIii();
    const got = await acquireLease(iii, 'failsid', 'compaction');
    expect(got).toBeNull();
  });

  it('still returns null for every concurrent caller during an outage', async () => {
    const { iii } = makeFailingUpdateIii();
    const results = await Promise.all(
      Array.from({ length: 4 }, () => acquireLease(iii, 'failsid-many', 'compaction')),
    );
    expect(results.every((r) => r === null)).toBe(true);
  });
});

describe('acquireLease concurrency', () => {
  it('lets exactly one of two concurrent acquirers win even with write latency', async () => {
    const { iii } = makeRacyStateIii(10);
    const results = await Promise.all([
      acquireLease(iii, 'race-sid', 'compaction'),
      acquireLease(iii, 'race-sid', 'compaction'),
    ]);
    const winners = results.filter((r) => r !== null);
    expect(winners.length).toBe(1);
  });

  it('lets exactly one of many concurrent acquirers win', async () => {
    const { iii } = makeRacyStateIii(10);
    const results = await Promise.all(
      Array.from({ length: 8 }, () => acquireLease(iii, 'race-sid-multi', 'compaction')),
    );
    const winners = results.filter((r) => r !== null);
    expect(winners.length).toBe(1);
  });
});
