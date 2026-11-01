import { vi } from 'vitest';
import type { AddedHook, Call, IIIClient, MatchHook, Observer, RemovedHook } from 'iii-sdk';

export type CallRecord = {
  function_id: string;
  namespace?: string;
  payload: Record<string, unknown>;
  action?: { type: string };
  metadata?: unknown;
};

/** One call iii's delivery made to an observer's target. */
export type Delivery = {
  observer_id: string;
  function_id: string;
  target: string;
  payload: Record<string, unknown>;
};

type Hooks = { added?: AddedHook; removed?: RemovedHook; match?: MatchHook };

type Hosted = {
  handler: (payload: unknown) => unknown;
  options: Record<string, unknown>;
};

export type ObserveOptions = {
  id?: string;
  /** The observer's target; defaults to `feedTarget(functionId)`. */
  target?: string;
  /** Raw args; defaults to `{ session_id }`. */
  args?: Record<string, unknown>;
};

export type FakeIii = {
  iii: IIIClient;
  /** Every call the worker made, in order (its own feed calls included). */
  calls: CallRecord[];
  state: Map<string, unknown>;
  registered: Map<string, (payload: unknown) => Promise<unknown>>;
  /** Functions the worker registered, with their options. */
  functions: Map<string, Hosted>;
  /** What the worker observed: `iii.observe(source, target, options)`. */
  observed: Array<{ source: unknown; target: unknown; options?: unknown }>;
  /** Add an observer of one of the worker's functions the way the engine would; rejects with the hook's error. */
  observeFeed: (functionId: string, session_id: string, opts?: ObserveOptions) => Promise<string>;
  /** Remove an observer the way `dispose()` would. */
  unobserve: (id: string) => Promise<void>;
  /** The active observers of one of the worker's functions. */
  observersOf: (functionId: string) => Observer[];
  /** The target `observeFeed` uses when none is given. */
  feedTarget: (functionId: string) => string;
  /** Every delivery iii made, in order. */
  deliveries: Delivery[];
  /** Frames one feed delivered to its default observer target, in order. */
  feedFrames: (functionId: string) => Array<Record<string, unknown>>;
};

/**
 * In-memory stand-in for the engine bus: `state::get/set/list` backed by a
 * Map keyed `${scope}/${key}`, every call recorded (with its action and
 * metadata), `registerFunction` captured so tests can invoke handlers at the
 * same unknown boundary the engine uses, and observers of the worker's own
 * functions modelled as iii does: `onObserverAdded` runs before an observer is
 * active, `onObserverMatch` picks the observers of each call, and iii's
 * delivery hands the handler's return value to each matched target
 * (`deliver: 'output'`).
 */
export function fakeIii(): FakeIii {
  const calls: CallRecord[] = [];
  const state = new Map<string, unknown>();
  const registered = new Map<string, (payload: unknown) => Promise<unknown>>();
  const functions = new Map<string, Hosted>();
  const hooks = new Map<string, Hooks>();
  const active = new Map<string, Observer[]>();
  const observed: FakeIii['observed'] = [];
  const deliveries: Delivery[] = [];
  let nextObserver = 0;

  const hooksOf = (functionId: string): Hooks => {
    let entry = hooks.get(functionId);
    if (!entry) {
      entry = {};
      hooks.set(functionId, entry);
    }
    return entry;
  };
  const observersOf = (functionId: string) => active.get(functionId) ?? [];
  const asKeys = (key: string | readonly string[]) => (typeof key === 'string' ? [key] : [...key]);

  const matching = (functionId: string, call: Partial<Call>): Observer[] => {
    const match = hooks.get(functionId)?.match;
    const all = observersOf(functionId);
    if (!match) return all;
    if (typeof match === 'function') return all.filter((obs) => match(obs, call as Call));
    const keys = new Set(asKeys(match.call(call as Call)));
    return all.filter((obs) => asKeys(match.observer(obs)).some((k) => keys.has(k)));
  };

  const ref = (functionId: string) => {
    const fn = {
      id: functionId,
      onObserverAdded: (hook: AddedHook | null) => {
        hooksOf(functionId).added = hook ?? undefined;
        return fn;
      },
      onObserverRemoved: (hook: RemovedHook | null) => {
        hooksOf(functionId).removed = hook ?? undefined;
        return fn;
      },
      onObserverMatch: (hook: MatchHook | null) => {
        hooksOf(functionId).match = hook ?? undefined;
        return fn;
      },
      onObserverDeliver: () => fn,
      observers: () => observersOf(functionId),
      isObserved: (call?: Partial<Call>) =>
        call === undefined
          ? observersOf(functionId).length > 0
          : matching(functionId, call).length > 0,
      unregister: () => {
        functions.delete(functionId);
      },
    };
    return fn;
  };

  const iii = {
    call: async (req: {
      function_id: string;
      namespace?: string;
      payload: Record<string, unknown>;
      action?: { type: string };
      metadata?: unknown;
    }) => {
      // Clone like the wire would: the live bus serializes payloads, so
      // later caller-side mutation must not rewrite recorded calls.
      const payload = structuredClone(req.payload);
      const call: CallRecord = { function_id: req.function_id, payload };
      if (req.namespace !== undefined) call.namespace = req.namespace;
      if (req.action !== undefined) call.action = req.action;
      if (req.metadata !== undefined) call.metadata = structuredClone(req.metadata);
      calls.push(call);
      const hosted = functions.get(req.function_id);
      if (hosted?.options.private) {
        // The worker's own private function: run it, then iii delivers its
        // return value to the observers that match the call.
        const output = (await hosted.handler(payload)) as Record<string, unknown>;
        const delivered: Partial<Call> = { function_id: req.function_id, input: payload, output };
        for (const obs of matching(req.function_id, delivered)) {
          deliveries.push({
            observer_id: obs.id,
            function_id: req.function_id,
            target: obs.target,
            payload: structuredClone(output),
          });
        }
        return req.action?.type === 'void' ? undefined : output;
      }
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
        return { name: payload.name, body: '# iii runtime\n\niii call engine::functions::list' };
      }
      if (req.function_id === 'directory::skills::index') {
        return { body: '# Skills index\n\n## shell\n\nRun commands.', workers_count: 1 };
      }
      if (req.function_id === 'state::list') {
        return [...state.entries()].filter(([k]) => k.startsWith(`${scope}/`)).map(([, v]) => v);
      }
      return null;
    },
    registerFunction: vi.fn(
      (
        fnId: string,
        handler: (payload: unknown) => Promise<unknown>,
        options: Record<string, unknown> = {},
      ) => {
        registered.set(fnId, handler);
        functions.set(fnId, { handler, options });
        return ref(fnId);
      },
    ),
    onObserverAdded: (functionId: string, hook: AddedHook | null) => {
      hooksOf(functionId).added = hook ?? undefined;
      return iii;
    },
    onObserverRemoved: (functionId: string, hook: RemovedHook | null) => {
      hooksOf(functionId).removed = hook ?? undefined;
      return iii;
    },
    onObserverMatch: (functionId: string, hook: MatchHook | null) => {
      hooksOf(functionId).match = hook ?? undefined;
      return iii;
    },
    observers: (functionId: string) => observersOf(functionId),
    isObserved: (functionId: string, call?: Partial<Call>) => ref(functionId).isObserved(call),
    observe: vi.fn((source: unknown, target: unknown, options?: unknown) => {
      observed.push(options === undefined ? { source, target } : { source, target, options });
      return {
        id: `observer-${observed.length}`,
        state: 'pending',
        ready: async () => {},
        onStateChange: () => () => {},
        dispose: async () => {},
      };
    }),
  } as unknown as IIIClient;

  const feedTarget = (functionId: string) => `test::on::${functionId}`;

  const observeFeed = async (functionId: string, session_id: string, opts: ObserveOptions = {}) => {
    if (!functions.has(functionId)) throw new Error(`${functionId} is not registered`);
    const id = opts.id ?? `observer-${++nextObserver}`;
    const obs: Observer = {
      id,
      function_id: functionId,
      args: opts.args ?? { session_id },
      filters: {},
      target: opts.target ?? feedTarget(functionId),
      on: 'success',
      capture: ['input', 'output', 'caller'],
      matches: () => true,
      accepted: async () => {},
      reject: () => {},
    };
    await hooks.get(functionId)?.added?.(obs);
    active.set(functionId, [...observersOf(functionId), obs]);
    return id;
  };

  const unobserve = async (id: string) => {
    for (const [functionId, list] of active) {
      const obs = list.find((o) => o.id === id);
      if (!obs) continue;
      active.set(
        functionId,
        list.filter((o) => o.id !== id),
      );
      await hooks.get(functionId)?.removed?.(obs);
    }
  };

  const feedFrames = (functionId: string) =>
    deliveries.filter((d) => d.target === feedTarget(functionId)).map((d) => d.payload);

  return {
    iii,
    calls,
    state,
    registered,
    functions,
    observed,
    observeFeed,
    unobserve,
    observersOf,
    feedTarget,
    deliveries,
    feedFrames,
  };
}
