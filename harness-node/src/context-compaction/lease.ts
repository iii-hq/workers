/**
 * Single-writer lease via nonce-and-readback. state::* has no CAS, so we
 * write a unique nonce and read it back — exactly one writer sees its own
 * nonce survive (last write wins).
 */

import { randomUUID } from 'node:crypto';
import type { ISdk } from '../runtime/iii.js';
import { stateGet, stateSet } from '../runtime/state.js';

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
  if (!v) return 0;
  if (typeof v === 'number') return Math.floor(v);
  if (typeof v === 'object') {
    const ts = (v as Record<string, unknown>).ts;
    if (typeof ts === 'number') return Math.floor(ts / 1000);
  }
  return 0;
}

export async function acquireLease(
  iii: ISdk,
  session_id: string,
  kind: LeaseKind = 'compaction',
): Promise<string | null> {
  const key = leaseKey(session_id, kind);
  const now_ms = Date.now();
  const now_secs = Math.floor(now_ms / 1000);

  const existing = await stateGet(iii, STATE_SCOPE, key);
  if (existing) {
    const ts_secs = readLeaseTimestampSecs(existing);
    if (ts_secs > 0 && now_secs - ts_secs < LEASE_TTL_SECS) return null;
  }

  const nonce = mintLeaseNonce();
  await stateSet(iii, STATE_SCOPE, key, { nonce, ts: now_ms });

  const stored = await stateGet(iii, STATE_SCOPE, key);
  const storedNonce =
    stored &&
    typeof stored === 'object' &&
    typeof (stored as Record<string, unknown>).nonce === 'string'
      ? ((stored as Record<string, unknown>).nonce as string)
      : null;
  return storedNonce === nonce ? nonce : null;
}

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
