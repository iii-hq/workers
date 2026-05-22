/**
 * Tiny `state::*` wrappers. Mirrors
 * `turn-orchestrator/src/persistence.rs::state_get` / `state_set`.
 *
 * All helpers are tolerant: trigger errors degrade to `null` / `[]` and
 * are logged at warn level so a single failed read never aborts a turn.
 */

import type { ISdk } from 'iii-sdk';
import type { StateListInput } from 'iii-sdk/state';
import { logger } from './otel.js';

export type { StateListInput } from 'iii-sdk/state';

// Mirrors engine `UpdateOp` (sdk/packages/rust/iii/src/types.rs). Variants
// that target a JSON field (`set`, `increment`, `decrement`, `remove`) take
// a required `path` string — `""` (FieldPath::root) means "the whole value".
// `merge`/`append` accept an optional MergePath (string or array of strings).
export type StateUpdateOp =
  | { type: 'set'; path: string; value: unknown }
  | { type: 'merge'; path?: string | string[]; value: Record<string, unknown> }
  | { type: 'append'; path?: string | string[]; value: unknown }
  | { type: 'increment'; path: string; by: number }
  | { type: 'decrement'; path: string; by: number }
  | { type: 'remove'; path: string }
  | { type: string; [k: string]: unknown };

/** One row from a keyed `state::list` envelope (`{ items: [...] }`), when present. */
export type StateListKeyedEntry = {
  key?: string;
  value?: unknown;
};

/** Raw list rows before value unwrap; `null` when the response is not a list. */
export function stateListResponseRows(response: unknown): unknown[] | null {
  if (Array.isArray(response)) return response;
  if (response && typeof response === 'object') {
    const items = (response as Record<string, unknown>).items;
    if (Array.isArray(items)) return items;
  }
  return null;
}

function unwrapStateListEntry<T>(entry: unknown): T {
  if (entry && typeof entry === 'object' && 'value' in (entry as Record<string, unknown>)) {
    return (entry as Record<string, unknown>).value as T;
  }
  return entry as T;
}

/**
 * Normalizes a `state::list` trigger result to stored values.
 *
 * Official iii returns a flat `T[]` ({@link StateListInput} only). Some
 * deployments also wrap rows as `{ value }` or `{ items: [{ key, value }] }`;
 * we accept those shapes so harness workers stay compatible.
 */
export function parseStateListValues<T>(response: unknown): T[] {
  const arr = stateListResponseRows(response);
  if (!arr) return [];
  return arr.map((entry) => unwrapStateListEntry<T>(entry));
}

/** Keyed rows when the list response includes `key` (not returned by stock iii). */
export function parseStateListKeyedEntries(response: unknown): StateListKeyedEntry[] {
  const arr = stateListResponseRows(response);
  if (!arr) return [];
  return arr.map((entry) => {
    if (entry && typeof entry === 'object') {
      const row = entry as Record<string, unknown>;
      return {
        key: typeof row.key === 'string' ? row.key : undefined,
        value: row.value !== undefined ? row.value : entry,
      };
    }
    return { value: entry };
  });
}

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
 * Lists all values in a scope using the iii SDK contract (`StateListInput`).
 */
export async function stateListValues<T>(iii: ISdk, input: StateListInput): Promise<T[]> {
  try {
    const resp = await iii.trigger<StateListInput, unknown>({
      function_id: 'state::list',
      payload: input,
    });
    return parseStateListValues<T>(resp);
  } catch (err) {
    logger.warn('state::list failed', { scope: input.scope, err: String(err) });
    return [];
  }
}

/**
 * @deprecated Third argument `prefix` is not sent to iii (engine lists the
 * whole scope). Kept for call-site stability; filter returned values locally
 * if you need key-prefix semantics.
 */
export async function stateList(iii: ISdk, scope: string, _prefix?: string): Promise<unknown[]> {
  return stateListValues(iii, { scope });
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
