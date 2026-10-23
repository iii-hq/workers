import type { IIIClient } from 'iii-sdk';
import { isDeepStrictEqual } from 'node:util';
import type { z } from 'zod';
import type {
  BridgeClient,
  BridgeClientFactory,
  BridgeLaunchOptions,
  RpcOptions,
} from '../src/bridge.js';
import {
  AGENT_EVENT_TRIGGER_TYPE,
  createAgentFeeds,
  type FeedEmit,
  type FeedPayload,
  RAW_EVENT_TRIGGER_TYPE,
} from '../src/agent-feed.js';
import { defaultConfig, type Config } from '../src/config.js';
import type { RunStreamMessageWire } from '../src/types.js';

type Registration = {
  handler: (payload: unknown) => Promise<unknown> | unknown;
  options: Record<string, unknown>;
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

export const AGENT = AGENT_EVENT_TRIGGER_TYPE;
export const RAW = RAW_EVENT_TRIGGER_TYPE;
export const consumerFor = (typeId: string) => `consumer::${typeId}`;

export class MockIII {
  readonly state = new Map<string, unknown>();
  readonly functions = new Map<string, Registration>();
  readonly triggers: unknown[] = [];
  /** Owned trigger types the worker registered, with their handler. */
  readonly triggerTypes = new Map<string, TriggerTypeHandler>();
  readonly triggerCalls: Array<Record<string, unknown>> = [];
  configValue: unknown = defaultConfig();
  configFailures = 0;
  ensureError: unknown;

  registerFunction(
    id: string,
    handler: Registration['handler'],
    options: Record<string, unknown>,
  ): void {
    this.functions.set(id, { handler, options });
  }

  registerTrigger(trigger: unknown): void {
    this.triggers.push(trigger);
  }

  registerTriggerType(type: { id: string }, handler: TriggerTypeHandler): { id: string } {
    this.triggerTypes.set(type.id, handler);
    return { id: type.id };
  }

  /** Bind a consumer to an owned trigger type like the engine would. */
  async bind(typeId: string, sessionId: string, binding: Partial<TriggerBinding> = {}) {
    await this.handlerFor(typeId).registerTrigger({
      id: `${typeId}:${sessionId}`,
      function_id: consumerFor(typeId),
      config: { session_id: sessionId },
      ...binding,
    });
  }

  async unbind(typeId: string, id: string) {
    await this.handlerFor(typeId).unregisterTrigger({
      id,
      function_id: consumerFor(typeId),
      config: {},
    });
  }

  /** Payloads delivered to the default consumer of a feed (`consumer::<typeId>`). */
  feedPayloads(typeId: string): FeedPayload[] {
    return this.triggerCalls
      .filter((call) => call.function_id === consumerFor(typeId))
      .map((call) => call.payload as FeedPayload);
  }

  /** The `event` of every payload delivered to the default consumer of a feed. */
  feedEvents(typeId: string): Array<Record<string, unknown>> {
    return this.feedPayloads(typeId).map((payload) => payload.event as Record<string, unknown>);
  }

  private handlerFor(typeId: string): TriggerTypeHandler {
    const handler = this.triggerTypes.get(typeId);
    if (!handler) throw new Error(`trigger type ${typeId} is not registered`);
    return handler;
  }

  async trigger(request: Record<string, unknown>): Promise<unknown> {
    this.triggerCalls.push(structuredClone(request));
    const functionId = request.function_id;
    const payload = (request.payload ?? {}) as Record<string, unknown>;
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
    // Void feed deliveries to bound consumers: recorded above, nothing to return.
    if (request.action !== undefined) return undefined;
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

/** Register the worker's two feeds on `iii` and bind the default consumers for `sessions`. */
export async function boundFeeds(
  iii: MockIII,
  ...sessions: string[]
): Promise<{ emit: FeedEmit; emitRaw: FeedEmit }> {
  const feeds = createAgentFeeds(iii.asClient());
  for (const session of sessions) {
    await iii.bind(AGENT, session);
    await iii.bind(RAW, session);
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
