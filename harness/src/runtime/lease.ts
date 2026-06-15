/**
 * Single-writer lease over `state::update` — the only atomic read-modify-write
 * iii exposes (there's no native lock). Acquire writes a `{nonce, ts}` claim and
 * reads the prior value in one atomic op, so exactly one concurrent acquirer
 * sees a free/expired prior and wins; a claim older than `ttlMs` reads inactive,
 * folding crash recovery into the same op. session-lease is a thin (scope, ttl)
 * adapter over this.
 */

import { randomUUID } from 'node:crypto';
import type { ISdk } from './iii.js';
import { stateGet, stateSet, stateUpdate } from './state.js';

export type LeaseRecord = { nonce: string; ts: number };

let counter = 0;

export function mintLeaseNonce(): string {
  return `${process.pid}-${process.hrtime.bigint()}-${counter++}-${randomUUID().slice(0, 8)}`;
}

function isLeaseActive(rec: LeaseRecord | null | undefined, nowMs: number, ttlMs: number): boolean {
  return rec != null && nowMs - rec.ts < ttlMs;
}

/** Returns the nonce to release with, or null if another holder owns a valid lease. */
export async function acquireLease(
  iii: ISdk,
  scope: string,
  key: string,
  ttlMs: number,
): Promise<string | null> {
  const now = Date.now();
  if (isLeaseActive(await stateGet<LeaseRecord>(iii, scope, key), now, ttlMs)) return null;

  const nonce = mintLeaseNonce();
  const result = await stateUpdate<LeaseRecord>(iii, scope, key, [
    { type: 'set', path: '', value: { nonce, ts: now } },
  ]);
  // A null envelope means the atomic write failed — never treat that as a win,
  // or an outage lets every contender through.
  if (!result) return null;

  // We clobbered a still-valid claim (always someone else's — our nonce is
  // fresh): restore it and bow out.
  if (isLeaseActive(result.old_value, now, ttlMs)) {
    await stateSet(iii, scope, key, result.old_value);
    return null;
  }
  return nonce;
}

export async function releaseLease(
  iii: ISdk,
  scope: string,
  key: string,
  nonce: string,
): Promise<void> {
  const stored = await stateGet<LeaseRecord>(iii, scope, key);
  if (stored?.nonce === nonce) await stateSet(iii, scope, key, null);
}

/** Poll {@link acquireLease} with exponential backoff until acquired or timed out. */
export async function acquireLeaseWithWait(
  iii: ISdk,
  scope: string,
  key: string,
  ttlMs: number,
  totalTimeoutMs: number,
): Promise<string | null> {
  const deadline = Date.now() + totalTimeoutMs;
  let backoff = 50;
  while (true) {
    const nonce = await acquireLease(iii, scope, key, ttlMs);
    if (nonce) return nonce;
    if (Date.now() + backoff > deadline) return null;
    await new Promise((r) => setTimeout(r, backoff));
    backoff = Math.min(backoff * 2, 500);
  }
}
