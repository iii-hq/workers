/**
 * Single-writer lease via nonce-and-readback. Mirrors
 * `context-compaction/src/lib.rs::{acquire_lease, release_lease,
 * mint_lease_nonce, read_lease_timestamp_secs}`.
 *
 * Why not CAS? The engine's `state::*` ops have no CAS primitive, so a
 * naive check-then-write races. Stamping a unique nonce and reading it
 * back lets exactly one writer see its own nonce survive (last write
 * wins).
 */

import { randomUUID } from 'node:crypto';
import type { ISdk } from '../runtime/iii.js';
import { stateGet, stateSet } from '../runtime/state.js';

export const LEASE_TTL_SECS = 300;
const STATE_SCOPE = 'agent';

let counter = 0;

export function leaseKey(session_id: string): string {
  return `session/${session_id}/compaction_lease`;
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

export async function acquireLease(iii: ISdk, session_id: string): Promise<string | null> {
  const key = leaseKey(session_id);
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

export async function releaseLease(iii: ISdk, session_id: string, ourNonce: string): Promise<void> {
  const key = leaseKey(session_id);
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
