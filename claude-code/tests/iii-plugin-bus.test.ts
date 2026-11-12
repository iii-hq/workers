/**
 * iii_call and iii_observe: the plugin's two primitives over the iii bus.
 *
 * Written against a fake bus with the iii-sdk 0.26 shapes the plugin uses
 * (`call`, `registerFunction`, `observe` returning an ObserverHandle), so every
 * behaviour is pinned without an engine.
 */

import { describe, expect, it } from 'vitest';
import { type Bus, callFunction, ObserverBuffer, sinkFunctionId } from '../src/iii-plugin/bus.js';
import type { Directory } from '../src/iii-plugin/directory.js';
import { createHandler } from '../src/iii-plugin/server.js';

type Handler = (payload: unknown) => unknown;

function fakeBus(
  options: {
    callResult?: unknown;
    callError?: Error;
    readyError?: { code: string; message: string };
  } = {},
) {
  const calls: Record<string, unknown>[] = [];
  const functions = new Map<string, { handler: Handler; options: unknown }>();
  const unregistered: string[] = [];
  const observes: { source: unknown; target: unknown; options: unknown; id: string }[] = [];
  const disposed: string[] = [];
  let next = 0;
  const bus: Bus = {
    call: async (req) => {
      calls.push(req as Record<string, unknown>);
      if (options.callError) throw options.callError;
      return options.callResult;
    },
    registerFunction: (id, handler, opts) => {
      functions.set(id, { handler: handler as Handler, options: opts });
      return { unregister: () => void unregistered.push(id) };
    },
    observe: (source, target, opts) => {
      const id = `obs-${++next}`;
      observes.push({ source, target, options: opts, id });
      return {
        id,
        state: options.readyError ? 'error' : 'active',
        ready: async () => {
          if (options.readyError)
            throw Object.assign(new Error(options.readyError.message), options.readyError);
        },
        dispose: async () => void disposed.push(id),
      };
    },
  };
  /** Deliver an observed invocation to the sink, as the engine would. */
  const deliver = async (observer_id: string, extra: Record<string, unknown> = {}) => {
    const sink = [...functions.values()][0];
    await sink.handler({
      observer_id,
      activation_id: `act-${Math.random()}`,
      function_id: 'orders::create',
      namespace: 'default',
      invocation_id: `inv-${Math.random()}`,
      status: 'success',
      started_at_ms: 1,
      duration_ms: 2,
      ...extra,
    });
  };
  return { bus, calls, functions, unregistered, observes, disposed, deliver };
}

describe('callFunction (iii_call)', () => {
  it('invokes the function with its payload and returns the result', async () => {
    const f = fakeBus({ callResult: { ok: true, n: 2 } });
    const r = await callFunction(f.bus, { function_id: 'math::add', payload: { a: 1, b: 1 } });
    expect(f.calls[0]).toMatchObject({ function_id: 'math::add', payload: { a: 1, b: 1 } });
    expect(r).toEqual({ result: { ok: true, n: 2 } });
  });

  it('defaults the payload to an empty object', async () => {
    const f = fakeBus({ callResult: null });
    await callFunction(f.bus, { function_id: 'engine::workers::list' });
    expect(f.calls[0].payload).toEqual({});
  });

  it('passes timeout_ms and namespace through', async () => {
    const f = fakeBus({ callResult: 1 });
    await callFunction(f.bus, { function_id: 'x::y', timeout_ms: 1234, namespace: 'other' });
    expect(f.calls[0]).toMatchObject({ timeoutMs: 1234, namespace: 'other' });
  });

  it('fires and forgets with void: true', async () => {
    const f = fakeBus({ callResult: undefined });
    const r = await callFunction(f.bus, { function_id: 'x::y', void: true });
    expect(f.calls[0].action).toBeDefined();
    expect(r).toEqual({ result: null, void: true });
  });

  it('truncates a result larger than max_chars and says so', async () => {
    const f = fakeBus({ callResult: { big: 'x'.repeat(5000) } });
    const r = await callFunction(f.bus, { function_id: 'x::y', max_chars: 100 });
    expect(r.truncated).toBe(true);
    expect(typeof r.result).toBe('string');
    expect(String(r.result).length).toBe(100);
    expect(r.total_chars).toBeGreaterThan(5000);
  });

  it('rejects a call without a function_id before touching the bus', async () => {
    const f = fakeBus();
    await expect(callFunction(f.bus, { function_id: '  ' })).rejects.toThrow(/function_id/);
    expect(f.calls).toEqual([]);
  });

  it('rejects a payload that is not an object', async () => {
    const f = fakeBus();
    await expect(
      callFunction(f.bus, { function_id: 'x::y', payload: 'nope' as never }),
    ).rejects.toThrow(/payload/);
  });

  it('surfaces the engine error', async () => {
    const f = fakeBus({ callError: new Error('function_not_found: x::y') });
    await expect(callFunction(f.bus, { function_id: 'x::y' })).rejects.toThrow(
      'function_not_found',
    );
  });
});

describe('sinkFunctionId', () => {
  it('gives every plugin process its own function namespace', () => {
    expect(sinkFunctionId('my.host', 42)).toBe('iii-plugin-my-host-42::observed');
    expect(sinkFunctionId('a', 1)).not.toBe(sinkFunctionId('a', 2));
  });
});

describe('ObserverBuffer (iii_observe / iii_events / iii_unobserve)', () => {
  const make = (f: ReturnType<typeof fakeBus>, opts: { maxBuffered?: number } = {}) =>
    new ObserverBuffer(async () => f.bus, { sinkId: 'iii-plugin-h-1::observed', ...opts });

  it('registers one sink lazily, on the first observe only', async () => {
    const f = fakeBus();
    const buf = make(f);
    expect(f.functions.size).toBe(0);
    await buf.observe({ function_id: 'a::b' });
    await buf.observe({ function_id: 'c::d' });
    expect([...f.functions.keys()]).toEqual(['iii-plugin-h-1::observed']);
    // The engine refuses a private function as an observer target
    // (FORBIDDEN), so the sink is public but kept out of listings and traces.
    const options = f.functions.get('iii-plugin-h-1::observed')?.options as Record<string, unknown>;
    expect(options.private).toBeUndefined();
    expect(options.metadata).toEqual({ internal: true, trace_hidden: true });
  });

  it('passes the source, filters, args, capture and lifetime to the engine', async () => {
    const f = fakeBus();
    const buf = make(f);
    const r = await buf.observe({
      function_id: 'orders::create',
      filters: { 'input.amount': { $gte: 500 } },
      args: { table: 'orders' },
      capture: ['input', 'error'],
      namespace: 'default',
      max_events: 3,
      expires_in_ms: 60000,
    });
    expect(f.observes[0].source).toEqual({
      function_id: 'orders::create',
      filters: { 'input.amount': { $gte: 500 } },
      args: { table: 'orders' },
      capture: ['input', 'error'],
      namespace: 'default',
    });
    expect(f.observes[0].target).toBe('iii-plugin-h-1::observed');
    expect(f.observes[0].options).toEqual({ max_fires: 3, expires_in_ms: 60000 });
    expect(r).toEqual({ observer_id: 'obs-1', function_id: 'orders::create', state: 'active' });
  });

  it('omits options that were not given', async () => {
    const f = fakeBus();
    await make(f).observe({ function_id: 'a::b' });
    expect(f.observes[0].source).toEqual({ function_id: 'a::b' });
    expect(f.observes[0].options).toEqual({});
  });

  it('rejects an observe without a function_id', async () => {
    const f = fakeBus();
    await expect(make(f).observe({ function_id: '' })).rejects.toThrow(/function_id/);
    expect(f.observes).toEqual([]);
  });

  it('disposes and reports an observer the engine refused', async () => {
    const f = fakeBus({ readyError: { code: 'INVALID_ARGS', message: 'bad filter' } });
    const buf = make(f);
    await expect(buf.observe({ function_id: 'a::b' })).rejects.toThrow(/INVALID_ARGS.*bad filter/);
    expect(f.disposed).toEqual(['obs-1']);
    expect(buf.list()).toEqual([]);
  });

  it('buffers events per observer and drains them in arrival order', async () => {
    const f = fakeBus();
    const buf = make(f);
    const a = await buf.observe({ function_id: 'a::b' });
    const b = await buf.observe({ function_id: 'c::d' });
    await f.deliver(a.observer_id, { input: 1 });
    await f.deliver(b.observer_id, { input: 'b' });
    await f.deliver(a.observer_id, { input: 2 });

    const first = buf.events({ observer_id: a.observer_id });
    expect(first.events.map((e) => e.input)).toEqual([1, 2]);
    expect(first.remaining).toBe(0);
    // Drained: a second read returns nothing new.
    expect(buf.events({ observer_id: a.observer_id }).events).toEqual([]);
    // The other observer is untouched.
    expect(buf.events({ observer_id: b.observer_id }).events.map((e) => e.input)).toEqual(['b']);
  });

  it('reads every observer at once, oldest first, when no id is given', async () => {
    const f = fakeBus();
    const buf = make(f);
    const a = await buf.observe({ function_id: 'a::b' });
    const b = await buf.observe({ function_id: 'c::d' });
    await f.deliver(a.observer_id, { input: 1 });
    await f.deliver(b.observer_id, { input: 2 });
    await f.deliver(a.observer_id, { input: 3 });
    expect(buf.events({}).events.map((e) => e.input)).toEqual([1, 2, 3]);
  });

  it('honours max and reports what is left', async () => {
    const f = fakeBus();
    const buf = make(f);
    const a = await buf.observe({ function_id: 'a::b' });
    for (let i = 0; i < 5; i++) await f.deliver(a.observer_id, { input: i });
    const r = buf.events({ observer_id: a.observer_id, max: 2 });
    expect(r.events.map((e) => e.input)).toEqual([0, 1]);
    expect(r.remaining).toBe(3);
  });

  it('keeps the newest events when the buffer is full and counts the dropped', async () => {
    const f = fakeBus();
    const buf = make(f, { maxBuffered: 3 });
    const a = await buf.observe({ function_id: 'a::b' });
    for (let i = 0; i < 5; i++) await f.deliver(a.observer_id, { input: i });
    const r = buf.events({ observer_id: a.observer_id });
    expect(r.events.map((e) => e.input)).toEqual([2, 3, 4]);
    expect(r.dropped).toBe(2);
    // The drop count is reported once, then reset.
    expect(buf.events({ observer_id: a.observer_id }).dropped).toBe(0);
  });

  it('errors on events for an unknown observer', async () => {
    const f = fakeBus();
    expect(() => make(f).events({ observer_id: 'nope' })).toThrow(/unknown observer/);
  });

  it('unobserve disposes the observer and forgets its buffer', async () => {
    const f = fakeBus();
    const buf = make(f);
    const a = await buf.observe({ function_id: 'a::b' });
    await f.deliver(a.observer_id);
    expect(await buf.unobserve(a.observer_id)).toEqual({
      observer_id: a.observer_id,
      removed: true,
      discarded: 1,
    });
    expect(f.disposed).toEqual([a.observer_id]);
    expect(buf.list()).toEqual([]);
    // A late delivery after unobserve is ignored, not resurrected.
    await f.deliver(a.observer_id);
    expect(buf.list()).toEqual([]);
  });

  it('errors on unobserve of an unknown observer', async () => {
    const f = fakeBus();
    await expect(make(f).unobserve('nope')).rejects.toThrow(/unknown observer/);
  });

  it('lists observers with their counts and state', async () => {
    const f = fakeBus();
    const buf = make(f);
    const a = await buf.observe({ function_id: 'a::b', filters: { 'input.x': { $eq: 1 } } });
    await f.deliver(a.observer_id);
    await f.deliver(a.observer_id);
    buf.events({ observer_id: a.observer_id, max: 1 });
    expect(buf.list()).toEqual([
      {
        observer_id: a.observer_id,
        function_id: 'a::b',
        filters: { 'input.x': { $eq: 1 } },
        state: 'active',
        buffered: 1,
        received: 2,
        dropped: 0,
      },
    ]);
  });

  it('close disposes every observer and unregisters the sink', async () => {
    const f = fakeBus();
    const buf = make(f);
    await buf.observe({ function_id: 'a::b' });
    await buf.observe({ function_id: 'c::d' });
    await buf.close();
    expect(f.disposed.sort()).toEqual(['obs-1', 'obs-2']);
    expect(f.unregistered).toEqual(['iii-plugin-h-1::observed']);
    expect(buf.list()).toEqual([]);
  });
});

describe('MCP tools for call and observe', () => {
  function server(f: ReturnType<typeof fakeBus>) {
    const dir = {
      bus: async () => f.bus,
      status: () => ({ engine: null, state: 'connected' }),
    } as unknown as Directory;
    const observers = new ObserverBuffer(async () => f.bus, { sinkId: 'iii-plugin-h-1::observed' });
    return createHandler(dir, observers);
  }
  const tool = async (handle: ReturnType<typeof createHandler>, name: string, args: unknown = {}) =>
    (await handle({
      jsonrpc: '2.0',
      id: 1,
      method: 'tools/call',
      params: { name, arguments: args },
    })) as {
      content: { text: string }[];
      isError?: boolean;
    };

  it('lists the call and observe tools with input schemas', async () => {
    const handle = server(fakeBus());
    const list = (await handle({ jsonrpc: '2.0', id: 1, method: 'tools/list' })) as {
      tools: { name: string; inputSchema: { required?: string[] } }[];
    };
    const byName = Object.fromEntries(list.tools.map((t) => [t.name, t]));
    for (const name of [
      'iii_call',
      'iii_observe',
      'iii_events',
      'iii_unobserve',
      'iii_observers',
    ]) {
      expect(byName[name], name).toBeDefined();
    }
    expect(byName.iii_call.inputSchema.required).toEqual(['function_id']);
    expect(byName.iii_observe.inputSchema.required).toEqual(['function_id']);
    expect(byName.iii_unobserve.inputSchema.required).toEqual(['observer_id']);
  });

  it('tells the agent, in its instructions, to use iii_call and iii_observe', async () => {
    const handle = server(fakeBus());
    const init = (await handle({ jsonrpc: '2.0', id: 1, method: 'initialize', params: {} })) as {
      instructions: string;
    };
    expect(init.instructions).toContain('iii_call');
    expect(init.instructions).toContain('iii_observe');
    expect(init.instructions).toContain('iii_events');
    expect(init.instructions).toMatch(/instead of .*iii call/i);
  });

  it('iii_call returns the result as JSON text', async () => {
    const f = fakeBus({ callResult: { workers: 3 } });
    const r = await tool(server(f), 'iii_call', { function_id: 'engine::workers::list' });
    expect(r.isError).toBeUndefined();
    expect(JSON.parse(r.content[0].text)).toEqual({ result: { workers: 3 } });
  });

  it('iii_call reports an engine failure as a tool error the model reads', async () => {
    const f = fakeBus({ callError: new Error('function_not_found: nope::x') });
    const r = await tool(server(f), 'iii_call', { function_id: 'nope::x' });
    expect(r.isError).toBe(true);
    expect(r.content[0].text).toContain('function_not_found');
  });

  it('observe, events, observers and unobserve work end to end', async () => {
    const f = fakeBus();
    const handle = server(f);
    const obs = JSON.parse(
      (await tool(handle, 'iii_observe', { function_id: 'orders::create' })).content[0].text,
    );
    expect(obs.observer_id).toBe('obs-1');
    await f.deliver('obs-1', { input: { id: 7 } });

    const listed = JSON.parse((await tool(handle, 'iii_observers')).content[0].text);
    expect(listed[0]).toMatchObject({ observer_id: 'obs-1', buffered: 1 });

    const events = JSON.parse(
      (await tool(handle, 'iii_events', { observer_id: 'obs-1' })).content[0].text,
    );
    expect(events.events[0]).toMatchObject({ observer_id: 'obs-1', input: { id: 7 } });

    const removed = JSON.parse(
      (await tool(handle, 'iii_unobserve', { observer_id: 'obs-1' })).content[0].text,
    );
    expect(removed.removed).toBe(true);
  });

  it('a refused observe is a tool error carrying the engine code', async () => {
    const f = fakeBus({ readyError: { code: 'INVALID_ARGS', message: 'bad filter' } });
    const r = await tool(server(f), 'iii_observe', { function_id: 'a::b' });
    expect(r.isError).toBe(true);
    expect(r.content[0].text).toContain('INVALID_ARGS');
  });
});
