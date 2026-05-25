/**
 * `state::*` client aligned with `iii-sdk/state` (`IState`).
 *
 * Tolerant helpers (default) mirror turn-orchestrator persistence: trigger
 * errors degrade to `null` / `[]` and are logged at warn level so a single
 * failed read never aborts a turn. Use `createState(iii, { tolerant: false })`
 * when storage errors should propagate (session store, llm-budget).
 */

import type { ISdk } from 'iii-sdk';
import type { UpdateOp } from 'iii-sdk/stream';
import type {
  DeleteResult,
  IState,
  StateDeleteInput,
  StateGetInput,
  StateListInput,
  StateSetInput,
  StateSetResult,
  StateUpdateInput,
  StateUpdateResult,
} from 'iii-sdk/state';
import { logger } from './otel.js';

export type { UpdateOp } from 'iii-sdk/stream';
export type {
  DeleteResult,
  IState,
  StateDeleteInput,
  StateGetInput,
  StateListInput,
  StateSetInput,
  StateSetResult,
  StateUpdateInput,
  StateUpdateResult,
} from 'iii-sdk/state';

export type CreateStateOptions = {
  /** When true (default), log and return null/[] on trigger failure. */
  tolerant?: boolean;
};

type StateListGroupsResult = { groups: string[] };

function normalizeGetResult<T>(v: unknown): T | null {
  if (v === null || v === undefined) return null;
  return v as T;
}

/** Raw list rows before value unwrap; `null` when the response is not a list. */
export function stateListResponseRows(response: unknown): unknown[] | null {
  if (Array.isArray(response)) return response;
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
 * Official iii returns a flat `T[]`. Some bridge deployments wrap rows as
 * `{ value }`; we accept that shape for compatibility.
 */
export function parseStateListValues<T>(response: unknown): T[] {
  const arr = stateListResponseRows(response);
  if (!arr) return [];
  return arr.map((entry) => unwrapStateListEntry<T>(entry));
}

export function createState(iii: ISdk, opts: CreateStateOptions = {}): IState {
  const tolerant = opts.tolerant !== false;

  async function run<T>(
    op: string,
    context: Record<string, unknown>,
    fn: () => Promise<T>,
    fallback: T,
  ): Promise<T> {
    try {
      return await fn();
    } catch (err) {
      if (tolerant) {
        logger.warn(`${op} failed`, { ...context, err: String(err) });
        return fallback;
      }
      throw err;
    }
  }

  return {
    get: <TData>(input: StateGetInput): Promise<TData | null> =>
      run('state::get', { scope: input.scope, key: input.key }, async () => {
        const v = await iii.trigger<StateGetInput, TData>({
          function_id: 'state::get',
          payload: input,
        });
        return normalizeGetResult<TData>(v);
      }, null),

    set: <TData>(input: StateSetInput): Promise<StateSetResult<TData> | null> =>
      run('state::set', { scope: input.scope, key: input.key }, async () => {
        const result = await iii.trigger<StateSetInput, StateSetResult<TData>>({
          function_id: 'state::set',
          payload: input,
        });
        return result ?? null;
      }, null),

    delete: (input: StateDeleteInput): Promise<DeleteResult> =>
      run('state::delete', { scope: input.scope, key: input.key }, async () => {
        const result = await iii.trigger<StateDeleteInput, DeleteResult>({
          function_id: 'state::delete',
          payload: input,
        });
        return result ?? {};
      }, {}),

    list: <TData>(input: StateListInput): Promise<TData[]> =>
      run('state::list', { scope: input.scope }, async () => {
        const resp = await iii.trigger<StateListInput, unknown>({
          function_id: 'state::list',
          payload: input,
        });
        return parseStateListValues<TData>(resp);
      }, []),

    update: <TData>(input: StateUpdateInput): Promise<StateUpdateResult<TData> | null> =>
      run('state::update', { scope: input.scope, key: input.key }, async () => {
        const result = await iii.trigger<StateUpdateInput, StateUpdateResult<TData>>({
          function_id: 'state::update',
          payload: input,
        });
        return result ?? null;
      }, null),
  };
}

/** Lists all scope names that contain state data. */
export async function stateListGroups(
  iii: ISdk,
  opts: CreateStateOptions = {},
): Promise<string[]> {
  const tolerant = opts.tolerant !== false;
  try {
    const result = await iii.trigger<Record<string, never>, StateListGroupsResult | string[]>({
      function_id: 'state::list_groups',
      payload: {},
    });
    if (Array.isArray(result)) return result;
    return result?.groups ?? [];
  } catch (err) {
    if (tolerant) {
      logger.warn('state::list_groups failed', { err: String(err) });
      return [];
    }
    throw err;
  }
}

// --- Tolerant (scope, key) ergonomics for turn-orchestrator ---

const tolerantState = (iii: ISdk) => createState(iii, { tolerant: true });

export async function stateGet(iii: ISdk, scope: string, key: string): Promise<unknown> {
  return tolerantState(iii).get({ scope, key });
}

export async function stateSet(
  iii: ISdk,
  scope: string,
  key: string,
  value: unknown,
): Promise<StateSetResult<unknown> | null> {
  return tolerantState(iii).set({ scope, key, value });
}

export async function stateDelete(iii: ISdk, scope: string, key: string): Promise<void> {
  await tolerantState(iii).delete({ scope, key });
}

export async function stateListValues<T>(iii: ISdk, input: StateListInput): Promise<T[]> {
  return tolerantState(iii).list<T>(input);
}

export async function stateUpdate(
  iii: ISdk,
  scope: string,
  key: string,
  ops: UpdateOp[],
): Promise<StateUpdateResult<unknown> | null> {
  return tolerantState(iii).update({ scope, key, ops });
}
