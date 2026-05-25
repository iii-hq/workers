// Single-writer lease via atomic state::update. The set-and-return-old
// semantics close the read-modify-write race: a concurrent writer sees
// our nonce in old_value and restores its predecessor.

import { randomUUID } from 'node:crypto';
import type { ISdk } from '../runtime/iii.js';
import { stateGet, stateSet, stateUpdate } from '../runtime/state.js';

export type LeaseKind = 'compaction' | 'prune';

export const LEASE_TTL_SECS = 300;
const STATE_SCOPE = 'agent';

let counter = 0;

export function leaseKey(session_id: string, kind: LeaseKind = 'compaction'): string {
  return `session/${session_id}/${kind}_lease`;
}

export function mintLeaseNonce(): string {
  const pid = process.pid;
  const nanos = process.hrtime.bigint().toString();
  const seq = counter++;
  return `${pid}-${nanos}-${seq}-${randomUUID().slice(0, 8)}`;
}

export function readLeaseTimestampSecs(v: unknown): number {
  if (!v || typeof v !== 'object') return 0;
  const ts = (v as Record<string, unknown>).ts;
  if (typeof ts === 'number') return Math.floor(ts / 1000);
  return 0;
}

function isLeaseActive(v: unknown, now_secs: number): boolean {
  const ts_secs = readLeaseTimestampSecs(v);
  return ts_secs > 0 && now_secs - ts_secs < LEASE_TTL_SECS;
}

export async function acquireLease(
  iii: ISdk,
  session_id: string,
  kind: LeaseKind = 'compaction',
): Promise<string | null> {
  const key = leaseKey(session_id, kind);
  const now_ms = Date.now();
  const now_secs = Math.floor(now_ms / 1000);

  // Fast path: skip the atomic set when a valid lease is clearly held.
  const existing = await stateGet(iii, STATE_SCOPE, key);
  if (existing && isLeaseActive(existing, now_secs)) return null;

  const nonce = mintLeaseNonce();
  // path: '' targets FieldPath::root in the engine — set the whole value
  // atomically. Without `path`, the engine fails to deserialize the op and
  // stateUpdate falls into its catch + returns null.
  const result = await stateUpdate(iii, STATE_SCOPE, key, [
    { type: 'set', path: '', value: { nonce, ts: now_ms } },
  ]);
  // stateUpdate swallows backend errors and returns null. Treat a null
  // envelope as "we did NOT acquire the lease" — otherwise a transient
  // state-store failure would let every concurrent caller declare itself
  // the winner, defeating the whole point of the lease.
  if (!result) return null;
  const oldValue = result.old_value;

  // If we just overwrote a concurrent acquirer's active lease, restore it
  // and bow out. stateUpdate is atomic, so only one caller can see
  // old_value == null (or expired) — exactly one winner.
  if (oldValue && isLeaseActive(oldValue, now_secs)) {
    await stateSet(iii, STATE_SCOPE, key, oldValue);
    return null;
  }
  return nonce;
}

// Known race: between the stateGet and the stateSet a new acquirer could
// claim the lease, and our stateSet(null) would clear it. Closing this
// needs a CAS-like primitive in iii-database (delete-if-matches). In
// practice this is bounded by LEASE_TTL_SECS (300s) >> the summariser's
// own timeout (120s), so a duplicate claim resolves itself via TTL.
export async function releaseLease(
  iii: ISdk,
  session_id: string,
  ourNonce: string,
  kind: LeaseKind = 'compaction',
): Promise<void> {
  const key = leaseKey(session_id, kind);
  const stored = await stateGet(iii, STATE_SCOPE, key);
  const storedNonce =
    stored &&
    typeof stored === 'object' &&
    typeof (stored as Record<string, unknown>).nonce === 'string'
      ? ((stored as Record<string, unknown>).nonce as string)
      : null;
  if (storedNonce === ourNonce) {
    await stateSet(iii, STATE_SCOPE, key, null);
  }
}

export async function stampLastCompaction(iii: ISdk, session_id: string): Promise<void> {
  await stateSet(iii, STATE_SCOPE, `session/${session_id}/last_compaction_at`, Date.now());
}

export async function acquireLeaseWithWait(
  iii: ISdk,
  session_id: string,
  kind: LeaseKind,
  totalTimeoutMs: number,
): Promise<string | null> {
  const deadline = Date.now() + totalTimeoutMs;
  let backoff = 50;
  while (true) {
    const got = await acquireLease(iii, session_id, kind);
    if (got) return got;
    if (Date.now() + backoff > deadline) return null;
    await new Promise((r) => setTimeout(r, backoff));
    backoff = Math.min(backoff * 2, 500);
  }
}
