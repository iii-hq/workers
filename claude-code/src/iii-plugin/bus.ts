/**
 * The plugin's two primitives over the iii bus: `call` and `observe`.
 *
 * `iii_call` invokes any function and returns its result. `iii_observe` makes
 * this plugin process observe a function: Claude Code cannot be woken by an MCP
 * server mid-turn, so every matching invocation is buffered here, per observer,
 * and read with `iii_events`; `iii_unobserve` removes one. All observers share
 * one private sink function, registered on first use under a namespace unique
 * to this process (two sessions must not fight over a function id).
 *
 * The `Bus` type is the slice of the iii-sdk 0.26 client this module uses, so
 * the behaviour is testable without an engine.
 */

import { hostname } from 'node:os';
import { CallAction, type IIIClient } from 'iii-sdk';

export type ObserveReason = { code: string; message: string };

export type ObserverHandleLike = {
  readonly id: string;
  readonly state: string;
  readonly reason?: ObserveReason;
  ready(): Promise<void>;
  dispose(): Promise<void>;
};

export type ObserveSourceLike = {
  function_id: string;
  namespace?: string;
  args?: Record<string, unknown>;
  filters?: Record<string, unknown>;
  capture?: string[];
};

export type Bus = {
  call(req: {
    function_id: string;
    payload: unknown;
    timeoutMs?: number;
    namespace?: string;
    action?: unknown;
  }): Promise<unknown>;
  registerFunction(
    id: string,
    handler: (payload: unknown) => unknown,
    options?: Record<string, unknown>,
  ): { unregister(): void };
  observe(
    source: ObserveSourceLike,
    target: string,
    options?: { max_fires?: number; expires_in_ms?: number },
  ): ObserverHandleLike;
};

/** The iii-sdk client as a `Bus`. */
export function clientBus(client: IIIClient): Bus {
  return {
    call: (req) => client.call(req as never),
    registerFunction: (id, handler, options) =>
      client.registerFunction(id, handler as never, options as never),
    observe: (source, target, options) =>
      client.observe(source as never, target, options) as unknown as ObserverHandleLike,
  };
}

const DEFAULT_MAX_CHARS = 100_000;

export type CallArgs = {
  function_id: string;
  payload?: Record<string, unknown>;
  timeout_ms?: number;
  namespace?: string;
  /** Fire and forget: do not wait for a result. */
  void?: boolean;
  /** Cap on the serialized result returned to the model. */
  max_chars?: number;
};

export type CallOutcome = {
  result: unknown;
  void?: true;
  truncated?: true;
  total_chars?: number;
};

export async function callFunction(bus: Bus, args: CallArgs): Promise<CallOutcome> {
  const function_id = typeof args.function_id === 'string' ? args.function_id.trim() : '';
  if (!function_id) throw new Error('function_id is required');
  const payload = args.payload ?? {};
  if (typeof payload !== 'object' || payload === null || Array.isArray(payload)) {
    throw new Error('payload must be a JSON object');
  }
  const req = {
    function_id,
    payload,
    ...(args.timeout_ms ? { timeoutMs: args.timeout_ms } : {}),
    ...(args.namespace ? { namespace: args.namespace } : {}),
    ...(args.void ? { action: CallAction.Void() } : {}),
  };
  const result = await bus.call(req);
  if (args.void) return { result: null, void: true };

  const max = args.max_chars && args.max_chars > 0 ? args.max_chars : DEFAULT_MAX_CHARS;
  const serialized = JSON.stringify(result) ?? 'null';
  if (serialized.length <= max) return { result: result ?? null };
  return { result: serialized.slice(0, max), truncated: true, total_chars: serialized.length };
}

/** `iii-plugin-<host>-<pid>::observed`: one sink per plugin process. */
export function sinkFunctionId(host: string = hostname(), pid: number = process.pid): string {
  const safe = host
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return `iii-plugin-${safe}-${pid}::observed`;
}

export type ObserveArgs = {
  function_id: string;
  namespace?: string;
  filters?: Record<string, unknown>;
  args?: Record<string, unknown>;
  capture?: string[];
  /** Stop after this many matching invocations. */
  max_events?: number;
  /** Remove the observer after this long. */
  expires_in_ms?: number;
};

export type ObservedEvent = { observer_id: string } & Record<string, unknown>;

type Entry = {
  observer_id: string;
  function_id: string;
  filters?: Record<string, unknown>;
  handle: ObserverHandleLike;
  events: { seq: number; event: ObservedEvent }[];
  received: number;
  dropped: number;
};

export type BufferOptions = {
  sinkId?: string;
  /** Per-observer cap; the oldest events are dropped beyond it. */
  maxBuffered?: number;
  /** How long `observe` waits for the engine to activate the observer. */
  readyTimeoutMs?: number;
};

export class ObserverBuffer {
  private readonly entries = new Map<string, Entry>();
  private sink: { unregister(): void } | null = null;
  private seq = 0;
  private readonly sinkId: string;
  private readonly maxBuffered: number;
  private readonly readyTimeoutMs: number;

  constructor(
    private readonly getBus: () => Promise<Bus>,
    options: BufferOptions = {},
  ) {
    this.sinkId = options.sinkId ?? sinkFunctionId();
    this.maxBuffered = options.maxBuffered ?? 500;
    this.readyTimeoutMs = options.readyTimeoutMs ?? 5_000;
  }

  private deliver(payload: unknown): void {
    const event = payload as ObservedEvent;
    const entry =
      event && typeof event === 'object' ? this.entries.get(event.observer_id) : undefined;
    if (!entry) return; // unobserved or never ours: dropped, not resurrected
    entry.received += 1;
    entry.events.push({ seq: ++this.seq, event });
    while (entry.events.length > this.maxBuffered) {
      entry.events.shift();
      entry.dropped += 1;
    }
  }

  async observe(
    req: ObserveArgs,
  ): Promise<{ observer_id: string; function_id: string; state: string }> {
    const function_id = typeof req.function_id === 'string' ? req.function_id.trim() : '';
    if (!function_id) throw new Error('function_id is required');
    const bus = await this.getBus();
    this.sink ??= bus.registerFunction(
      this.sinkId,
      (payload) => {
        this.deliver(payload);
        return {};
      },
      {
        // Not `private`: the engine refuses a private observer target. Any
        // caller can reach it, so `deliver` keeps only this session's own
        // observer ids and ignores everything else.
        description:
          'Receives invocations observed by an iii Claude Code plugin session (iii_observe).',
        metadata: { internal: true, trace_hidden: true },
      },
    );

    const source: ObserveSourceLike = { function_id };
    if (req.filters) source.filters = req.filters;
    if (req.args) source.args = req.args;
    if (req.capture) source.capture = req.capture;
    if (req.namespace) source.namespace = req.namespace;
    const options: { max_fires?: number; expires_in_ms?: number } = {};
    if (req.max_events) options.max_fires = req.max_events;
    if (req.expires_in_ms) options.expires_in_ms = req.expires_in_ms;

    const handle = bus.observe(source, this.sinkId, options);
    const entry: Entry = {
      observer_id: handle.id,
      function_id,
      ...(req.filters ? { filters: req.filters } : {}),
      handle,
      events: [],
      received: 0,
      dropped: 0,
    };
    this.entries.set(handle.id, entry);

    try {
      // An observer of a function nobody hosts yet stays `pending`: that is
      // valid, so a slow activation returns the state instead of failing.
      let timer: ReturnType<typeof setTimeout> | undefined;
      await Promise.race([
        handle.ready(),
        new Promise((r) => {
          timer = setTimeout(r, this.readyTimeoutMs);
        }),
      ]).finally(() => clearTimeout(timer));
    } catch (err) {
      this.entries.delete(handle.id);
      await handle.dispose().catch(() => {});
      const e = err as Partial<ObserveReason> & Error;
      const code = e.code ?? handle.reason?.code ?? 'OBSERVE_FAILED';
      const message = e.message ?? handle.reason?.message ?? String(err);
      throw new Error(`observe refused: ${code}: ${message}`);
    }
    return { observer_id: handle.id, function_id, state: handle.state };
  }

  events(req: { observer_id?: string; max?: number }): {
    events: ObservedEvent[];
    remaining: number;
    dropped: number;
  } {
    const max = req.max && req.max > 0 ? req.max : 50;
    let entries: Entry[];
    if (req.observer_id) {
      const entry = this.entries.get(req.observer_id);
      if (!entry) throw new Error(`unknown observer: ${req.observer_id}`);
      entries = [entry];
    } else {
      entries = [...this.entries.values()];
    }
    const queued = entries
      .flatMap((entry) => entry.events.map((item) => ({ entry, item })))
      .sort((a, b) => a.item.seq - b.item.seq);
    const taken = queued.slice(0, max);
    for (const { entry, item } of taken) entry.events.splice(entry.events.indexOf(item), 1);
    let dropped = 0;
    for (const entry of entries) {
      dropped += entry.dropped;
      entry.dropped = 0;
    }
    return {
      events: taken.map((t) => t.item.event),
      remaining: queued.length - taken.length,
      dropped,
    };
  }

  async unobserve(
    observer_id: string,
  ): Promise<{ observer_id: string; removed: true; discarded: number }> {
    const entry = this.entries.get(observer_id);
    if (!entry) throw new Error(`unknown observer: ${observer_id}`);
    this.entries.delete(observer_id);
    await entry.handle.dispose();
    return { observer_id, removed: true, discarded: entry.events.length };
  }

  list() {
    return [...this.entries.values()].map((e) => ({
      observer_id: e.observer_id,
      function_id: e.function_id,
      ...(e.filters ? { filters: e.filters } : {}),
      state: e.handle.state,
      buffered: e.events.length,
      received: e.received,
      dropped: e.dropped,
    }));
  }

  async close(): Promise<void> {
    const entries = [...this.entries.values()];
    this.entries.clear();
    await Promise.all(entries.map((e) => e.handle.dispose().catch(() => {})));
    this.sink?.unregister();
    this.sink = null;
  }
}
