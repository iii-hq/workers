import { describe, expect, it, vi } from 'vitest';
import {
  LEASE_TTL_SECS,
  leaseKey,
  mintLeaseNonce,
  readLeaseTimestampSecs,
  acquireLease,
  acquireLeaseWithWait,
} from '../../src/context-compaction/lease.js';

describe('lease helpers', () => {
  it('leaseKey namespaces by session', () => {
    const k = leaseKey('s9');
    expect(k).toContain('s9');
    expect(k).toContain('compaction_lease');
  });

  it('mintLeaseNonce produces unique values across rapid calls', () => {
    const seen = new Set<string>();
    for (let i = 0; i < 1000; i++) seen.add(mintLeaseNonce());
    expect(seen.size).toBe(1000);
  });

  it('readLeaseTimestampSecs reads new {nonce, ts} shape', () => {
    expect(readLeaseTimestampSecs({ nonce: 'a', ts: 1_700_000_000_000 })).toBe(1_700_000_000);
  });

  it('readLeaseTimestampSecs accepts legacy bare-int (seconds)', () => {
    expect(readLeaseTimestampSecs(1_700_000_000)).toBe(1_700_000_000);
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
// stateGet returns the raw trigger response (the value itself, not wrapped).
// stateSet stores payload.value at payload.key.
function makeStateIii() {
  const store = new Map<string, unknown>();
  const iii = {
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      const p = payload as Record<string, unknown>;
      if (function_id === 'state::get') {
        const v = store.get(p['key'] as string);
        return v !== undefined ? v : null;
      }
      if (function_id === 'state::set') {
        const v = p['value'];
        if (v === null || v === undefined) {
          store.delete(p['key'] as string);
        } else {
          store.set(p['key'] as string, v);
        }
        return { ok: true };
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

  it('leaseKey produces different keys for compaction vs prune', () => {
    const k1 = leaseKey('sess1', 'compaction');
    const k2 = leaseKey('sess1', 'prune');
    expect(k1).toContain('compaction_lease');
    expect(k2).toContain('prune_lease');
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
