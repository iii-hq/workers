/**
 * Per-session mutual-exclusion lease for turn FSM transitions — a thin
 * (scope, ttl) adapter over the shared lease in runtime/lease.ts.
 *
 * The `turn-step` durable queue has no per-session ordering, so two
 * `turn::function_awaiting_approval` wakes (one per `approval::resolve`, fanned
 * out by `turn::on_approval`) can run concurrently and double-execute the parked
 * turn. The lease lets one through; the loser throws TransientError and retries
 * via the queue, by which point the state has usually advanced and it stale-skips.
 */

import type { ISdk } from '../../runtime/iii.js';
import { acquireLease, releaseLease } from '../../runtime/lease.js';

export const LEASE_SCOPE = 'turn_lease';
/** A holder older than this is assumed crashed and may be stolen. */
export const LEASE_TTL_MS = 30_000;

export async function acquireSessionLease(iii: ISdk, session_id: string): Promise<string | null> {
  return acquireLease(iii, LEASE_SCOPE, session_id, LEASE_TTL_MS);
}

export async function releaseSessionLease(
  iii: ISdk,
  session_id: string,
  nonce: string,
): Promise<void> {
  await releaseLease(iii, LEASE_SCOPE, session_id, nonce);
}
