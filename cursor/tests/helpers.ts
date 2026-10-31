import type { AddedHook, Call, IIIClient, MatchHook, Observer, RemovedHook } from 'iii-sdk';
import { isDeepStrictEqual } from 'node:util';
import type { z } from 'zod';
import type {
  BridgeClient,
  BridgeClientFactory,
  BridgeLaunchOptions,
  RpcOptions,
} from '../src/bridge.js';
import {
  createAgentFeeds,
  type FeedEmit,
  type FeedPayload,
  ON_AGENT_EVENT_FN,
  ON_RAW_EVENT_FN,
} from '../src/agent-feed.js';
import { defaultConfig, type Config } from '../src/config.js';
import type { RunStreamMessageWire } from '../src/types.js';

type Registration = {
  handler: (payload: unknown) => Promise<unknown> | unknown;
  options: Record<string, unknown>;
};

type Hooks = { added?: AddedHook; removed?: RemovedHook; match?: MatchHook };

export type ObserverSpec = {
  id: string;
  /** The observer's target. */
  target: string;
  args: Record<string, unknown>;
};

/** One call iii's delivery made to an observer's target. */
export type Delivery = {
  observer_id: string;
  function_id: string;
  target: string;
  payload: Record<string, unknown>;
};

export const AGENT = ON_AGENT_EVENT_FN;
export const RAW = ON_RAW_EVENT_FN;
export const consumerFor = (functionId: string) => `consumer::${functionId}`;

const asKeys = (key: string | readonly string[]) => (typeof key === 'string' ? [key] : [...key]);

export class MockIII {
  readonly state = new Map<string, unknown>();
  readonly functions = new Map<string, Registration>();
  /** What the worker observed: `iii.observe(source, target, options)`. */
  readonly observed: Array<{ source: unknown; target: unknown; options?: unknown }> = [];
  /** Every call the worker made, in order (its own feed calls included). */
  readonly calls: Array<Record<string, unknown>> = [];
  /** Every delivery iii made to an observer's target, in order. */
  readonly deliveries: Delivery[] = [];
  private readonly hooks = new Map<string, Hooks>();
  private readonly active = new Map<string, Observer[]>();
  configValue: unknown = defaultConfig();
  configFailures = 0;
  ensureError: unknown;

  registerFunction(
    id: string,
    handler: Registration['handler'],
    options: Record<string, unknown>,
  ): unknown {
    this.functions.set(id, { handler, options });
    const hooks = this.hooksOf(id);
    const ref = {
      id,
      onObserverAdded: (hook: AddedHook | null) => {
        hooks.added = hook ?? undefined;
        return ref;
      },
      onObserverRemoved: (hook: RemovedHook | null) => {
        hooks.removed = hook ?? undefined;
        return ref;
      },
      onObserverMatch: (hook: MatchHook | null) => {
        hooks.match = hook ?? undefined;
        return ref;
      },
      onObserverDeliver: () => ref,
      observers: () => this.observers(id),
      isObserved: (call?: Partial<Call>) => this.isObserved(id, call),
      unregister: () => {
        this.functions.delete(id);
      },
    };
    return ref;
  }

  observe(source: unknown, target: unknown, options?: unknown) {
    this.observed.push(options === undefined ? { source, target } : { source, target, options });
    return {
      id: `observer-${this.observed.length}`,
      state: 'pending',
      ready: async () => {},
      onStateChange: () => () => {},
      dispose: async () => {},
    };
  }

  observers(functionId: string): Observer[] {
    return this.active.get(functionId) ?? [];
  }

  isObserved(functionId: string, call?: Partial<Call>): boolean {
    return call === undefined
      ? this.observers(functionId).length > 0
      : this.matching(functionId, call).length > 0;
  }

  /** Add an observer of one of the worker's functions like the engine would; rejects with the hook's error. */
  async addObserver(functionId: string, sessionId: string, spec: Partial<ObserverSpec> = {}) {
    if (!this.functions.has(functionId)) throw new Error(`${functionId} is not registered`);
    const obs: Observer = {
      id: spec.id ?? `${functionId}:${sessionId}`,
      function_id: functionId,
      args: spec.args ?? { session_id: sessionId },
      filters: {},
      target: spec.target ?? consumerFor(functionId),
      on: 'success',
      capture: ['input', 'output', 'caller'],
      matches: () => true,
      accepted: async () => {},
      reject: () => {},
    };
    await this.hooks.get(functionId)?.added?.(obs);
    this.active.set(functionId, [...this.observers(functionId), obs]);
    return obs.id;
  }

  /** Remove an observer added with `addObserver`. */
  async removeObserver(id: string) {
    for (const [functionId, list] of this.active) {
      const obs = list.find((o) => o.id === id);
      if (!obs) continue;
      this.active.set(
        functionId,
        list.filter((o) => o.id !== id),
      );
      await this.hooks.get(functionId)?.removed?.(obs);
    }
  }

  /** Frames delivered to the default observer target of a feed (`consumer::<functionId>`). */
  feedPayloads(functionId: string): FeedPayload[] {
    return this.deliveries
      .filter((delivery) => delivery.target === consumerFor(functionId))
      .map((delivery) => delivery.payload as unknown as FeedPayload);
  }

  /** The `event` of every frame delivered to the default observer target of a feed. */
  feedEvents(functionId: string): Array<Record<string, unknown>> {
    return this.feedPayloads(functionId).map((payload) => payload.event as Record<string, unknown>);
  }

  private hooksOf(functionId: string): Hooks {
    const entry = this.hooks.get(functionId) ?? {};
    this.hooks.set(functionId, entry);
    return entry;
  }

  private matching(functionId: string, call: Partial<Call>): Observer[] {
    const match = this.hooks.get(functionId)?.match;
    const all = this.observers(functionId);
    if (!match) return all;
    if (typeof match === 'function') return all.filter((obs) => match(obs, call as Call));
    const keys = new Set(asKeys(match.call(call as Call)));
    return all.filter((obs) => asKeys(match.observer(obs)).some((key) => keys.has(key)));
  }

  async call(request: Record<string, unknown>): Promise<unknown> {
    this.calls.push(structuredClone(request));
    const functionId = request.function_id;
    const payload = (request.payload ?? {}) as Record<string, unknown>;
    const hosted = this.functions.get(String(functionId));
    if (hosted?.options.private) {
      // The worker's own private function: run it, then iii delivers its
      // return value to the observers that match the call.
      const input = structuredClone(payload);
      const output = (await hosted.handler(input)) as Record<string, unknown>;
      const call: Partial<Call> = { function_id: String(functionId), input, output };
      for (const obs of this.matching(String(functionId), call)) {
        this.deliveries.push({
          observer_id: obs.id,
          function_id: String(functionId),
          target: obs.target,
          payload: clone(output),
        });
      }
      return (request.action as { type?: string } | undefined)?.type === 'void'
        ? undefined
        : output;
    }
    if (functionId === 'state::get') return clone(this.state.get(String(payload.key)) ?? null);
    if (functionId === 'state::set') {
      this.state.set(String(payload.key), clone(payload.value));
      return null;
    }
    if (functionId === 'state::compare-and-set') {
      const key = String(payload.key);
      const exists = this.state.has(key);
      const current = exists ? this.state.get(key) : null;
      const hasExpected = Object.hasOwn(payload, 'expected');
      const matches = hasExpected
        ? isDeepStrictEqual(current, payload.expected)
        : !exists || current === null;
      if (!matches) return { swapped: false, current: clone(current) };
      this.state.set(key, clone(payload.value));
      return { swapped: true };
    }
    if (functionId === 'state::list') return [...this.state.values()].map(clone);
    if (functionId === 'configuration::register') {
      if (!this.ensureError) throw new Error('unexpected legacy registration');
      if (Object.hasOwn(payload, 'initial_value')) this.configValue = clone(payload.initial_value);
      return { ...payload, value: clone(this.configValue) };
    }
    if (functionId === 'configuration::ensure') {
      if (this.ensureError) throw this.ensureError;
      const empty = this.configValue == null;
      if (empty) this.configValue = clone(payload.initial_value);
      return {
        action: empty ? 'seeded' : 'preserved',
        entry: { ...payload, value: clone(this.configValue) },
      };
    }
    if (functionId === 'configuration::get') {
      if (this.configFailures > 0) {
        this.configFailures -= 1;
        throw new Error('configuration temporarily unavailable');
      }
      return { value: clone(this.configValue) };
    }
    throw new Error(`unexpected iii function ${String(functionId)}`);
  }

  asClient(): IIIClient {
    return this as unknown as IIIClient;
  }
}

export type BridgeCall = {
  kind: 'unary' | 'stream';
  service: string;
  method: string;
  request: Record<string, unknown>;
  options?: RpcOptions;
};

type UnaryHandler = (call: BridgeCall) => unknown | Promise<unknown>;
type StreamHandler = (call: BridgeCall) => AsyncIterable<unknown>;

export class FakeBridgeClient implements BridgeClient {
  readonly calls: BridgeCall[] = [];
  closes = 0;

  constructor(
    private readonly unaryHandler: UnaryHandler,
    private readonly streamHandler: StreamHandler,
  ) {}

  async unary<T>(
    service: string,
    method: string,
    request: Record<string, unknown>,
    responseSchema: z.ZodType<T>,
    options?: RpcOptions,
  ): Promise<T> {
    const call = { kind: 'unary' as const, service, method, request, options };
    this.calls.push(clone(call));
    return responseSchema.parse(await this.unaryHandler(call));
  }

  stream<T>(
    service: string,
    method: string,
    request: Record<string, unknown>,
    responseSchema: z.ZodType<T>,
    options?: RpcOptions,
  ): AsyncIterable<T> {
    const call = { kind: 'stream' as const, service, method, request, options };
    this.calls.push(clone(call));
    const source = this.streamHandler(call);
    return {
      async *[Symbol.asyncIterator]() {
        for await (const item of source) yield responseSchema.parse(item);
      },
    };
  }

  async close(): Promise<void> {
    this.closes += 1;
  }
}

export class FakeBridgeFactory implements BridgeClientFactory {
  readonly options: BridgeLaunchOptions[] = [];
  closeAllCalls = 0;
  forceCloseAllCalls = 0;

  constructor(readonly client: FakeBridgeClient) {}

  create(options: BridgeLaunchOptions): BridgeClient {
    this.options.push(clone(options));
    return this.client;
  }

  async closeAll(): Promise<void> {
    this.closeAllCalls += 1;
  }

  forceCloseAll(): void {
    this.forceCloseAllCalls += 1;
  }
}

/** Register the worker's two feeds on `iii` and add the default observers for `sessions`. */
export async function observedFeeds(
  iii: MockIII,
  ...sessions: string[]
): Promise<{ emit: FeedEmit; emitRaw: FeedEmit }> {
  const feeds = createAgentFeeds(iii.asClient());
  for (const session of sessions) {
    await iii.addObserver(AGENT, session);
    await iii.addObserver(RAW, session);
  }
  return feeds;
}

export function testConfig(overrides: Partial<Config> = {}): Config {
  return {
    ...defaultConfig(),
    local_backend: 'sdk-bridge',
    api_key: 'key_test_secret',
    bridge_binary: '/fake/bridge',
    ...overrides,
  };
}

export async function* frames(...items: RunStreamMessageWire[]): AsyncIterable<unknown> {
  yield* items;
}

export function terminalFrames(
  runId: string,
  text = 'done',
  status = 'RUN_LIFECYCLE_STATUS_FINISHED',
): RunStreamMessageWire[] {
  return [
    {
      interactionUpdate: { type: 'text-delta', update: { delta: text } },
      offset: 'send-1',
    },
    {
      result: {
        agentId: 'agent',
        runId,
        status,
        result: { agentId: 'agent', runId, status, result: text },
      },
    },
    { done: { agentId: 'agent', runId } },
  ];
}

export function clone<T>(value: T): T {
  return structuredClone(value);
}
