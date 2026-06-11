/**
 * Compaction/prune leases — thin (scope, ttl) adapters over the shared lease in
 * runtime/lease.ts. Maps a LeaseKind to a state scope and the 300s TTL.
 */

import type { ISdk } from '../runtime/iii.js';
import {
  acquireLease as acquireLeaseShared,
  acquireLeaseWithWait as acquireLeaseWithWaitShared,
  mintLeaseNonce,
  releaseLease as releaseLeaseShared,
} from '../runtime/lease.js';
import { stateSet } from '../runtime/state.js';

const COMPACTION_LEASE_SCOPE = 'compaction_lease';
const PRUNE_LEASE_SCOPE = 'prune_lease';
const LAST_COMPACTION_AT_SCOPE = 'last_compaction_at';

export type LeaseKind = 'compaction' | 'prune';

export const LEASE_TTL_SECS = 300;
const LEASE_TTL_MS = LEASE_TTL_SECS * 1000;

export { mintLeaseNonce };

function leaseScope(kind: LeaseKind): string {
  return kind === 'compaction' ? COMPACTION_LEASE_SCOPE : PRUNE_LEASE_SCOPE;
}

/** Epoch-secs of a `{nonce, ts}` lease claim; 0 for any other shape. */
export function readLeaseTimestampSecs(v: unknown): number {
  if (!v || typeof v !== 'object') return 0;
  const ts = (v as Record<string, unknown>).ts;
  return typeof ts === 'number' ? Math.floor(ts / 1000) : 0;
}

export async function acquireLease(
  iii: ISdk,
  session_id: string,
  kind: LeaseKind = 'compaction',
): Promise<string | null> {
  return acquireLeaseShared(iii, leaseScope(kind), session_id, LEASE_TTL_MS);
}

export async function releaseLease(
  iii: ISdk,
  session_id: string,
  ourNonce: string,
  kind: LeaseKind = 'compaction',
): Promise<void> {
  await releaseLeaseShared(iii, leaseScope(kind), session_id, ourNonce);
}

export async function acquireLeaseWithWait(
  iii: ISdk,
  session_id: string,
  kind: LeaseKind,
  totalTimeoutMs: number,
): Promise<string | null> {
  return acquireLeaseWithWaitShared(
    iii,
    leaseScope(kind),
    session_id,
    LEASE_TTL_MS,
    totalTimeoutMs,
  );
}

export async function stampLastCompaction(iii: ISdk, session_id: string): Promise<void> {
  await stateSet(iii, LAST_COMPACTION_AT_SCOPE, session_id, Date.now());
}
