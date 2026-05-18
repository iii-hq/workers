import { describe, expect, it } from 'vitest';
import {
  LEASE_TTL_SECS,
  leaseKey,
  mintLeaseNonce,
  readLeaseTimestampSecs,
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
