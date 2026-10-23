import { vi } from 'vitest';
import type { ISdk } from 'iii-sdk';
import {
  AGENT_EVENT_TRIGGER_TYPE,
  createAgentFeeds,
  type Emit,
  type FeedPayload,
  RAW_EVENT_TRIGGER_TYPE,
} from '../../src/agent-feed.js';

export type TriggerCall = {
  function_id: string;
  namespace?: string;
  payload: Record<string, unknown>;
  action?: unknown;
  metadata?: unknown;
};

export type TriggerBinding = {
  id: string;
  function_id: string;
  config: unknown;
  namespace?: string;
  metadata?: Record<string, unknown>;
};

export type TriggerTypeHandler = {
  registerTrigger: (binding: TriggerBinding) => Promise<void>;
  unregisterTrigger: (binding: TriggerBinding) => Promise<void>;
};

export type FakeIii = {
  iii: ISdk;
  calls: TriggerCall[];
  state: Map<string, unknown>;
  registered: Map<string, (payload: unknown) => Promise<unknown>>;
  /** Trigger types the worker registered, keyed by id, with their handler. */
  triggerTypes: Map<string, TriggerTypeHandler>;
  /** Bind a consumer to an owned trigger type like the engine would. */
  bind: (typeId: string, session_id: string, binding?: Partial<TriggerBinding>) => Promise<void>;
  /** Unbind a binding created with `bind` (default id `${typeId}:${session_id}`). */
  unbind: (typeId: string, id: string) => Promise<void>;
  /** Payloads delivered to the default consumer of a feed (`consumer::<typeId>`). */
  feedPayloads: (typeId: string) => FeedPayload[];
  /** The `event` of every payload delivered to the default consumer of a feed. */
  feedEvents: (typeId: string) => Array<Record<string, unknown>>;
};

export const consumerFor = (typeId: string) => `consumer::${typeId}`;

/**
 * In-memory stand-in for the engine bus: `state::get/set/list` backed by a
 * Map keyed `${scope}/${key}`, `registerFunction` captured so tests can invoke
 * handlers at the same unknown boundary the engine uses, and
 * `registerTriggerType` captured so tests bind consumers to the worker's
 * owned feeds. Every trigger call (incl. Void feed deliveries) is recorded.
 */
export function fakeIii(): FakeIii {
  const calls: TriggerCall[] = [];
  const state = new Map<string, unknown>();
  const registered = new Map<string, (payload: unknown) => Promise<unknown>>();
  const triggerTypes = new Map<string, TriggerTypeHandler>();

  const iii = {
    trigger: async (req: {
      function_id: string;
      namespace?: string;
      payload: Record<string, unknown>;
      action?: unknown;
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
    registerFunction: vi.fn((fnId: string, handler: (payload: unknown) => Promise<unknown>) => {
      registered.set(fnId, handler);
    }),
    registerTrigger: vi.fn(),
    registerTriggerType: vi.fn((type: { id: string }, handler: TriggerTypeHandler) => {
      triggerTypes.set(type.id, handler);
      return { id: type.id };
    }),
  } as unknown as ISdk;

  const handlerFor = (typeId: string) => {
    const handler = triggerTypes.get(typeId);
    if (!handler) throw new Error(`trigger type ${typeId} is not registered`);
    return handler;
  };

  const bind = async (typeId: string, session_id: string, binding: Partial<TriggerBinding> = {}) =>
    handlerFor(typeId).registerTrigger({
      id: `${typeId}:${session_id}`,
      function_id: consumerFor(typeId),
      config: { session_id },
      ...binding,
    });

  const unbind = async (typeId: string, id: string) =>
    handlerFor(typeId).unregisterTrigger({ id, function_id: consumerFor(typeId), config: {} });

  const feedPayloads = (typeId: string) =>
    calls
      .filter((c) => c.function_id === consumerFor(typeId))
      .map((c) => c.payload as unknown as FeedPayload);

  const feedEvents = (typeId: string) =>
    feedPayloads(typeId).map((p) => p.event as Record<string, unknown>);

  return { iii, calls, state, registered, triggerTypes, bind, unbind, feedPayloads, feedEvents };
}

export const AGENT = AGENT_EVENT_TRIGGER_TYPE;
export const RAW = RAW_EVENT_TRIGGER_TYPE;

/** Register the worker's two feeds on `fake` and bind the default consumers for `sessions`. */
export async function boundFeeds(
  fake: FakeIii,
  ...sessions: string[]
): Promise<{ emit: Emit; emitRaw: Emit }> {
  const feeds = createAgentFeeds(fake.iii);
  for (const session of sessions) {
    await fake.bind(AGENT, session);
    await fake.bind(RAW, session);
  }
  return feeds;
}
