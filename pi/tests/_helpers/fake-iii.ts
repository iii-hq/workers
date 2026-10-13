import { vi } from 'vitest';
import type { ISdk } from 'iii-sdk';

export type TriggerCall = {
  function_id: string;
  namespace?: string;
  payload: Record<string, unknown>;
  action?: { type: string };
  metadata?: unknown;
};

type TriggerHandler = {
  registerTrigger(b: {
    id: string;
    function_id: string;
    config: unknown;
    metadata?: Record<string, unknown>;
    namespace?: string;
  }): Promise<void>;
  unregisterTrigger(b: { id: string; function_id: string; config: unknown }): Promise<void>;
};

export type BindOptions = {
  id?: string;
  function_id?: string;
  namespace?: string;
  /** Binding metadata (the engine-level `metadata` of the registration). */
  metadata?: Record<string, unknown>;
  /** Raw config; defaults to `{ session_id }`. */
  config?: unknown;
};

export type FakeIii = {
  iii: ISdk;
  calls: TriggerCall[];
  state: Map<string, unknown>;
  registered: Map<string, (payload: unknown) => Promise<unknown>>;
  /** Trigger types the worker registered, keyed by id, with their handler. */
  triggerTypes: Map<string, TriggerHandler>;
  /** Bind a consumer to an owned trigger type the way the engine would. */
  bindFeed: (typeId: string, session_id: string, opts?: BindOptions) => Promise<string>;
  unbindFeed: (typeId: string, id: string) => Promise<void>;
  /** The function id `bindFeed` uses when none is given. */
  feedFunction: (typeId: string) => string;
  /** Delivery payloads one feed sent to its default bound function, in order. */
  feedFrames: (typeId: string) => Array<Record<string, unknown>>;
};

/**
 * In-memory stand-in for the engine bus: `state::get/set/list` backed by a
 * Map keyed `${scope}/${key}`, every trigger recorded as a plain call (with
 * its action and metadata), `registerFunction` captured so tests can invoke
 * handlers at the same unknown boundary the engine uses, and
 * `registerTriggerType` captured so tests can bind consumers to the worker's
 * owned event feeds.
 */
export function fakeIii(): FakeIii {
  const calls: TriggerCall[] = [];
  const state = new Map<string, unknown>();
  const registered = new Map<string, (payload: unknown) => Promise<unknown>>();
  const triggerTypes = new Map<string, TriggerHandler>();
  let nextBinding = 0;

  const iii = {
    trigger: async (req: {
      function_id: string;
      namespace?: string;
      payload: Record<string, unknown>;
      action?: { type: string };
      metadata?: unknown;
    }) => {
      // Clone like the wire would: the live bus serializes payloads, so
      // later caller-side mutation must not rewrite recorded calls.
      const payload = structuredClone(req.payload);
      const call: TriggerCall = { function_id: req.function_id, payload };
      if (req.namespace !== undefined) call.namespace = req.namespace;
      if (req.action !== undefined) call.action = req.action;
      if (req.metadata !== undefined) call.metadata = structuredClone(req.metadata);
      calls.push(call);
      const { scope, key, value } = payload as { scope?: string; key?: string; value?: unknown };
      if (req.function_id === 'configuration::ensure') {
        const entryKey = `configuration/${String(payload.id)}`;
        const prior = state.get(entryKey);
        const next = prior == null ? structuredClone(payload.initial_value) : prior;
        state.set(entryKey, next);
        return {
          action: prior == null ? 'seeded' : 'preserved',
          entry: { ...payload, value: next },
        };
      }
      if (req.function_id === 'configuration::get')
        return { value: state.get(`configuration/${String(payload.id)}`) ?? null };
      if (req.function_id === 'configuration::register')
        throw new Error('unexpected legacy configuration registration');
      if (req.function_id === 'state::set') {
        state.set(`${scope}/${key}`, value);
        return null;
      }
      if (req.function_id === 'state::get') return state.get(`${scope}/${key}`) ?? null;
      // The iii context lives in the `iii-directory` worker, so a turn that
      // wants it asks the bus for it — these are the two calls it makes.
      if (req.function_id === 'directory::system-prompts::get') {
        return { name: payload.name, body: '# iii runtime\n\niii trigger engine::functions::list' };
      }
      if (req.function_id === 'directory::skills::index') {
        return { body: '# Skills index\n\n## shell\n\nRun commands.', workers_count: 1 };
      }
      if (req.function_id === 'state::list') {
        return [...state.entries()].filter(([k]) => k.startsWith(`${scope}/`)).map(([, v]) => v);
      }
      return null;
    },
    registerFunction: vi.fn((fnId: string, handler: (payload: unknown) => Promise<unknown>) => {
      registered.set(fnId, handler);
    }),
    registerTriggerType: vi.fn((type: { id: string }, handler: TriggerHandler) => {
      triggerTypes.set(type.id, handler);
      return { id: type.id };
    }),
  } as unknown as ISdk;

  const feedFunction = (typeId: string) => `test::on::${typeId}`;

  const handlerOf = (typeId: string) => {
    const handler = triggerTypes.get(typeId);
    if (!handler) throw new Error(`trigger type ${typeId} is not registered`);
    return handler;
  };

  const bindFeed = async (typeId: string, session_id: string, opts: BindOptions = {}) => {
    const id = opts.id ?? `binding-${++nextBinding}`;
    await handlerOf(typeId).registerTrigger({
      id,
      function_id: opts.function_id ?? feedFunction(typeId),
      config: 'config' in opts ? opts.config : { session_id },
      ...(opts.metadata === undefined ? {} : { metadata: opts.metadata }),
      ...(opts.namespace === undefined ? {} : { namespace: opts.namespace }),
    });
    return id;
  };

  const unbindFeed = async (typeId: string, id: string) =>
    handlerOf(typeId).unregisterTrigger({ id, function_id: '', config: undefined });

  const feedFrames = (typeId: string) =>
    calls.filter((c) => c.function_id === feedFunction(typeId)).map((c) => c.payload);

  return {
    iii,
    calls,
    state,
    registered,
    triggerTypes,
    bindFeed,
    unbindFeed,
    feedFunction,
    feedFrames,
  };
}
