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

function normalizeGetResult<T>(v: unknown): T | null {
  if (v === null || v === undefined) return null;
  return v as T;
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
      run(
        'state::get',
        { scope: input.scope, key: input.key },
        async () => {
          const v = await iii.trigger<StateGetInput, TData>({
            function_id: 'state::get',
            payload: input,
          });
          return normalizeGetResult<TData>(v);
        },
        null,
      ),

    set: <TData>(input: StateSetInput): Promise<StateSetResult<TData> | null> =>
      run(
        'state::set',
        { scope: input.scope, key: input.key },
        async () => {
          const result = await iii.trigger<StateSetInput, StateSetResult<TData>>({
            function_id: 'state::set',
            payload: input,
          });
          return result ?? null;
        },
        null,
      ),

    delete: (input: StateDeleteInput): Promise<DeleteResult> =>
      run(
        'state::delete',
        { scope: input.scope, key: input.key },
        async () => {
          const result = await iii.trigger<StateDeleteInput, DeleteResult>({
            function_id: 'state::delete',
            payload: input,
          });
          return result ?? {};
        },
        {},
      ),

    list: <TData>(input: StateListInput): Promise<TData[]> =>
      run(
        'state::list',
        { scope: input.scope },
        async () => {
          // state::list returns a flat array of stored values.
          const resp = await iii.trigger<StateListInput, unknown>({
            function_id: 'state::list',
            payload: input,
          });
          return Array.isArray(resp) ? (resp as TData[]) : [];
        },
        [],
      ),

    update: <TData>(input: StateUpdateInput): Promise<StateUpdateResult<TData> | null> =>
      run(
        'state::update',
        { scope: input.scope, key: input.key },
        async () => {
          const result = await iii.trigger<StateUpdateInput, StateUpdateResult<TData>>({
            function_id: 'state::update',
            payload: input,
          });
          return result ?? null;
        },
        null,
      ),
  };
}

// --- Tolerant (scope, key) ergonomics for turn-orchestrator ---

const tolerantState = (iii: ISdk) => createState(iii, { tolerant: true });

export async function stateGet<T = unknown>(
  iii: ISdk,
  scope: string,
  key: string,
): Promise<T | null> {
  return tolerantState(iii).get<T>({ scope, key });
}

export async function stateSet<T = unknown>(
  iii: ISdk,
  scope: string,
  key: string,
  value: T,
): Promise<StateSetResult<T> | null> {
  return tolerantState(iii).set<T>({ scope, key, value });
}

export async function stateDelete(iii: ISdk, scope: string, key: string): Promise<void> {
  await tolerantState(iii).delete({ scope, key });
}

export async function stateListValues<T>(iii: ISdk, input: StateListInput): Promise<T[]> {
  return tolerantState(iii).list<T>(input);
}

export async function stateUpdate<T = unknown>(
  iii: ISdk,
  scope: string,
  key: string,
  ops: UpdateOp[],
): Promise<StateUpdateResult<T> | null> {
  return tolerantState(iii).update<T>({ scope, key, ops });
}
