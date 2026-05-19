/**
 * Tiny `state::*` wrappers. Mirrors
 * `turn-orchestrator/src/persistence.rs::state_get` / `state_set`.
 *
 * All helpers are tolerant: trigger errors degrade to `null` / `[]` and
 * are logged at warn level so a single failed read never aborts a turn.
 */

import type { ISdk } from 'iii-sdk';
import { logger } from './otel.js';

export type StateUpdateOp =
  | { type: 'set'; value: unknown }
  | { type: 'merge'; value: Record<string, unknown> }
  | { type: 'append'; value: unknown }
  | { type: 'increment'; value: number }
  | { type: 'delete' }
  | { type: string; [k: string]: unknown };

export async function stateGet(iii: ISdk, scope: string, key: string): Promise<unknown> {
  try {
    const v = await iii.trigger<unknown, unknown>({
      function_id: 'state::get',
      payload: { scope, key },
    });
    if (v === null || v === undefined) return null;
    return v;
  } catch (err) {
    logger.warn('state::get failed', { scope, key, err: String(err) });
    return null;
  }
}

export async function stateSet(
  iii: ISdk,
  scope: string,
  key: string,
  value: unknown,
): Promise<void> {
  try {
    await iii.trigger<unknown, unknown>({
      function_id: 'state::set',
      payload: { scope, key, value },
    });
  } catch (err) {
    logger.warn('state::set failed', { scope, key, err: String(err) });
  }
}

export async function stateDelete(iii: ISdk, scope: string, key: string): Promise<void> {
  try {
    await iii.trigger<unknown, unknown>({
      function_id: 'state::delete',
      payload: { scope, key },
    });
  } catch (err) {
    logger.warn('state::delete failed', { scope, key, err: String(err) });
  }
}

/**
 * `state::list` returns `{ items: [{ key, value, ... }] }` on the wire.
 * We surface the raw value array (with `value` unwrapped when present).
 */
export async function stateList(iii: ISdk, scope: string, prefix: string): Promise<unknown[]> {
  try {
    const resp = await iii.trigger<unknown, { items?: Array<Record<string, unknown>> }>({
      function_id: 'state::list',
      payload: { scope, prefix },
    });
    const items = resp?.items ?? [];
    return items.map((entry) => (entry?.value !== undefined ? entry.value : entry));
  } catch (err) {
    logger.warn('state::list failed', { scope, prefix, err: String(err) });
    return [];
  }
}

/**
 * `state::update` applies one or more atomic ops and returns the
 * `{ old_value, new_value }` envelope.
 */
export async function stateUpdate(
  iii: ISdk,
  scope: string,
  key: string,
  ops: StateUpdateOp[],
): Promise<{ old_value?: unknown; new_value?: unknown } | null> {
  try {
    const v = await iii.trigger<unknown, { old_value?: unknown; new_value?: unknown }>({
      function_id: 'state::update',
      payload: { scope, key, ops },
    });
    return v ?? null;
  } catch (err) {
    logger.warn('state::update failed', { scope, key, err: String(err) });
    return null;
  }
}
