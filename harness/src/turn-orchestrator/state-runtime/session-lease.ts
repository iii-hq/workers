/**
 * Per-session mutual-exclusion lease for turn FSM transitions.
 *
 * The `turn-step` durable queue has no per-session ordering — `Enqueue` takes
 * only a queue name (see iii-sdk `TriggerAction.Enqueue`). So two
 * `turn::function_awaiting_approval` wakes for one session (one per
 * `approval::resolve` write, fanned out by the `turn::on_approval` state
 * trigger) can run concurrently. Without serialization both load the same
 * parked `turn_state`, execute every call, and finalize — duplicating side
 * effects (the function runs twice) and emitting duplicate
 * `function_execution_end` / `turn_end` frames, which wedges the turn.
 *
 * The only atomic primitive the state worker exposes is `state::update` with
 * `increment` — a locked read-modify-write that returns the prior value (the
 * kv adapter holds the store write-lock for the whole op). Acquire increments
 * a per-session holder counter; the caller that observes prior `0` (or a
 * missing key) won and may proceed. Release resets the counter to `0`.
 *
 * Crash recovery: a holder that dies mid-transition never resets the counter,
 * which would wedge the session forever. A contender whose acquire fails
 * therefore steals a lease whose recorded acquire time is older than
 * {@link LEASE_TTL_MS}. The steal is best-effort (a post-crash window can let
 * two contenders through), but that degrades to the pre-fix behavior only
 * briefly after a crash — far better than a permanent deadlock.
 */

import type { ISdk } from '../../runtime/iii.js';
import { stateGet, stateSet, stateUpdate } from '../../runtime/state.js';

export const LEASE_SCOPE = 'turn_lease';
export const LEASE_AT_SCOPE = 'turn_lease_at';
/** A holder older than this is assumed crashed and may be stolen. */
export const LEASE_TTL_MS = 30_000;

/** Atomically bump the holder counter; returns the prior count (0 when free/missing). */
async function bumpHolders(iii: ISdk, session_id: string): Promise<number> {
  const res = await stateUpdate(iii, LEASE_SCOPE, session_id, [
    { type: 'increment', path: '', by: 1 },
  ]);
  const prior = (res as { old_value?: unknown } | null)?.old_value;
  return typeof prior === 'number' ? prior : 0;
}

/**
 * Try to acquire the session lease. Returns `true` when this caller holds it
 * and must call {@link releaseSessionLease}; `false` when another transition
 * holds it (the caller should back off / retry).
 */
export async function acquireSessionLease(iii: ISdk, session_id: string): Promise<boolean> {
  if ((await bumpHolders(iii, session_id)) === 0) {
    await stateSet(iii, LEASE_AT_SCOPE, session_id, Date.now());
    return true;
  }
  // Contended — recover a lease abandoned by a crashed holder.
  const acquiredAt = await stateGet(iii, LEASE_AT_SCOPE, session_id);
  if (typeof acquiredAt === 'number' && Date.now() - acquiredAt > LEASE_TTL_MS) {
    await stateSet(iii, LEASE_SCOPE, session_id, 0);
    if ((await bumpHolders(iii, session_id)) === 0) {
      await stateSet(iii, LEASE_AT_SCOPE, session_id, Date.now());
      return true;
    }
  }
  return false;
}

/** Release the session lease. Safe to call only from the holder. */
export async function releaseSessionLease(iii: ISdk, session_id: string): Promise<void> {
  await stateSet(iii, LEASE_SCOPE, session_id, 0);
}
