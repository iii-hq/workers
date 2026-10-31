import { vi } from 'vitest';
import type { AddedHook, Call, IIIClient, MatchHook, Observer, RemovedHook } from 'iii-sdk';
import {
  createAgentFeeds,
  type Emit,
  type FeedPayload,
  ON_AGENT_EVENT_FN,
  ON_RAW_EVENT_FN,
} from '../../src/agent-feed.js';

export type CallRecord = {
  function_id: string;
  namespace?: string;
  payload: Record<string, unknown>;
  action?: unknown;
  metadata?: unknown;
};

/** One call iii's delivery made to an observer's target. */
export type Delivery = {
  observer_id: string;
  function_id: string;
  target: string;
  payload: Record<string, unknown>;
};

export type ObserverSpec = {
  id: string;
  /** The observer's target. */
  target: string;
  args: Record<string, unknown>;
};

type Hooks = { added?: AddedHook; removed?: RemovedHook; match?: MatchHook };

type Hosted = { handler: (payload: unknown) => unknown; options: Record<string, unknown> };

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
  /** Add an observer of one of the worker's functions like the engine would; rejects with the hook's error. */
  observe: (
    functionId: string,
    session_id: string,
    spec?: Partial<ObserverSpec>,
  ) => Promise<string>;
  /** Remove an observer added with `observe` (default id `${functionId}:${session_id}`). */
  unobserve: (id: string) => Promise<void>;
  /** The active observers of one of the worker's functions. */
  observersOf: (functionId: string) => Observer[];
  /** Every delivery iii made, in order. */
  deliveries: Delivery[];
  /** Frames delivered to the default observer target of a feed (`consumer::<functionId>`). */
  feedPayloads: (functionId: string) => FeedPayload[];
  /** The `event` of every frame delivered to the default observer target of a feed. */
  feedEvents: (functionId: string) => Array<Record<string, unknown>>;
};

export const consumerFor = (functionId: string) => `consumer::${functionId}`;

/**
 * In-memory stand-in for the engine bus: `state::get/set/list` backed by a
 * Map keyed `${scope}/${key}`, `registerFunction` captured so tests can invoke
 * handlers at the same unknown boundary the engine uses, and observers of the
 * worker's own functions modelled as iii does: `onObserverAdded` runs before an
 * observer is active, `onObserverMatch` picks the observers of each call, and
 * iii's delivery hands the handler's return value to each matched target
 * (`deliver: 'output'`). Every call (incl. Void feed calls) is recorded.
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

  const hooksOf = (functionId: string): Hooks => {
    const entry = hooks.get(functionId) ?? {};
    hooks.set(functionId, entry);
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

  const isObserved = (functionId: string, call?: Partial<Call>) =>
    call === undefined ? observersOf(functionId).length > 0 : matching(functionId, call).length > 0;

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
      isObserved: (call?: Partial<Call>) => isObserved(functionId, call),
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
      calls.push({
        function_id: req.function_id,
        namespace: req.namespace,
        payload,
        ...(req.action === undefined ? {} : { action: req.action }),
        ...(req.metadata === undefined ? {} : { metadata: structuredClone(req.metadata) }),
      });
      const hosted = functions.get(req.function_id);
      if (hosted?.options.private) {
        // The worker's own private function: run it, then iii delivers its
        // return value to the observers that match the call.
        const output = (await hosted.handler(payload)) as Record<string, unknown>;
        const call: Partial<Call> = { function_id: req.function_id, input: payload, output };
        for (const obs of matching(req.function_id, call)) {
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
    observers: (functionId: string) => observersOf(functionId),
    isObserved,
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

  const observe = async (
    functionId: string,
    session_id: string,
    spec: Partial<ObserverSpec> = {},
  ) => {
    if (!functions.has(functionId)) throw new Error(`${functionId} is not registered`);
    const obs: Observer = {
      id: spec.id ?? `${functionId}:${session_id}`,
      function_id: functionId,
      args: spec.args ?? { session_id },
      filters: {},
      target: spec.target ?? consumerFor(functionId),
      on: 'success',
      capture: ['input', 'output', 'caller'],
      matches: () => true,
      accepted: async () => {},
      reject: () => {},
    };
    await hooks.get(functionId)?.added?.(obs);
    active.set(functionId, [...observersOf(functionId), obs]);
    return obs.id;
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

  const feedPayloads = (functionId: string) =>
    deliveries
      .filter((d) => d.target === consumerFor(functionId))
      .map((d) => d.payload as unknown as FeedPayload);

  const feedEvents = (functionId: string) =>
    feedPayloads(functionId).map((p) => p.event as Record<string, unknown>);

  return {
    iii,
    calls,
    state,
    registered,
    functions,
    observed,
    observe,
    unobserve,
    observersOf,
    deliveries,
    feedPayloads,
    feedEvents,
  };
}

export const AGENT = ON_AGENT_EVENT_FN;
export const RAW = ON_RAW_EVENT_FN;

/** Register the worker's two feeds on `fake` and add the default observers for `sessions`. */
export async function observedFeeds(
  fake: FakeIii,
  ...sessions: string[]
): Promise<{ emit: Emit; emitRaw: Emit }> {
  const feeds = createAgentFeeds(fake.iii);
  for (const session of sessions) {
    await fake.observe(AGENT, session);
    await fake.observe(RAW, session);
  }
  return feeds;
}
