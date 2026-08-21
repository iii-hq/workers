import type { IIIClient } from 'iii-sdk';
import { isDeepStrictEqual } from 'node:util';
import type { z } from 'zod';
import type {
  BridgeClient,
  BridgeClientFactory,
  BridgeLaunchOptions,
  RpcOptions,
} from '../src/bridge.js';
import { defaultConfig, type Config } from '../src/config.js';
import type { RunStreamMessageWire } from '../src/types.js';

type Registration = {
  handler: (payload: unknown) => Promise<unknown> | unknown;
  options: Record<string, unknown>;
};

export class MockIII {
  readonly state = new Map<string, unknown>();
  readonly functions = new Map<string, Registration>();
  readonly triggers: unknown[] = [];
  readonly streamItems: unknown[] = [];
  readonly triggerCalls: Array<Record<string, unknown>> = [];
  configValue: unknown = defaultConfig();
  configFailures = 0;

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
    if (functionId === 'stream::set') {
      const current = this.streamItems.findIndex((item) => {
        const stored = item as Record<string, unknown>;
        return (
          stored.stream_name === payload.stream_name &&
          stored.group_id === payload.group_id &&
          stored.item_id === payload.item_id
        );
      });
      if (current >= 0) this.streamItems[current] = clone(payload);
      else this.streamItems.push(clone(payload));
      return null;
    }
    if (functionId === 'configuration::register') return { ok: true };
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

export function testConfig(overrides: Partial<Config> = {}): Config {
  return {
    ...defaultConfig(),
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
