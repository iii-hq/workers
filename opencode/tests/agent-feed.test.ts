import { InvocationError, TriggerAction } from 'iii-sdk';
import { describe, expect, it, vi } from 'vitest';
import {
  AGENT_EVENT_OBSERVABLE,
  AGENT_EVENT_TRIGGER_TYPE,
  createAgentFeeds,
  createFeed,
  MAX_BINDINGS,
  RAW_EVENT_OBSERVABLE,
  RAW_EVENT_TRIGGER_TYPE,
  validateFeedConfig,
} from '../src/agent-feed.js';
import { AGENT, consumerFor, fakeIii, RAW } from './_helpers/fake-iii.js';

const EVENT_ID = /^(.+)-([0-9a-f-]{36})-(\d{8})$/;

describe('agent feeds', () => {
  it('registers both owned trigger types', () => {
    const fake = fakeIii();
    createAgentFeeds(fake.iii);
    expect([...fake.triggerTypes.keys()].sort()).toEqual([
      'opencode::agent-event',
      'opencode::raw-event',
    ]);
    expect(AGENT_EVENT_TRIGGER_TYPE).toBe(AGENT);
    expect(RAW_EVENT_TRIGGER_TYPE).toBe(RAW);
  });

  it('delivers a frame to a bound consumer with namespace, metadata and Void action', async () => {
    const fake = fakeIii();
    const { emit } = createAgentFeeds(fake.iii);
    await fake.bind(AGENT, 's1', {
      id: 'b1',
      function_id: 'acp::__on_event::c1',
      namespace: 'tenant-a',
      metadata: { from: 'binding' },
      config: { session_id: 's1', metadata: { from: 'config' } },
    });
    await expect(emit('s1', { type: 'agent_end', messages: [] })).resolves.toBe(1);
    const call = fake.calls.find((c) => c.function_id === 'acp::__on_event::c1');
    expect(call).toBeDefined();
    expect(call?.namespace).toBe('tenant-a');
    expect(call?.metadata).toEqual({ from: 'config' });
    expect(call?.action).toEqual(TriggerAction.Void());
    expect(call?.payload).toMatchObject({
      session_id: 's1',
      seq: 0,
      source: 'opencode',
      event: { type: 'agent_end', messages: [] },
    });
    const payload = call?.payload as { event_id: string; epoch: string };
    expect(payload.event_id).toBe(`s1-${payload.epoch}-00000000`);
  });

  it('falls back to binding metadata and omits metadata when neither is set', async () => {
    const fake = fakeIii();
    const { emit } = createAgentFeeds(fake.iii);
    await fake.bind(AGENT, 's1', { id: 'with', function_id: 'f::with', metadata: { m: 1 } });
    await fake.bind(AGENT, 's1', { id: 'without', function_id: 'f::without' });
    await emit('s1', { type: 'turn_end' });
    expect(fake.calls.find((c) => c.function_id === 'f::with')?.metadata).toEqual({ m: 1 });
    expect(fake.calls.find((c) => c.function_id === 'f::without')).not.toHaveProperty('metadata');
  });

  it('only delivers a session to bindings for that session', async () => {
    const fake = fakeIii();
    const { emit, emitRaw } = createAgentFeeds(fake.iii);
    await fake.bind(AGENT, 'A', { id: 'a', function_id: 'f::a' });
    await fake.bind(RAW, 'A', { id: 'a-raw', function_id: 'f::a-raw' });
    await emit('B', { type: 'turn_end' });
    await emitRaw('B', { type: 'text' });
    expect(fake.calls).toHaveLength(0);
    await emit('A', { type: 'turn_end' });
    expect(fake.calls.map((c) => c.function_id)).toEqual(['f::a']);
  });

  it('makes no trigger call when nothing is bound', async () => {
    const fake = fakeIii();
    const { emit } = createAgentFeeds(fake.iii);
    await expect(emit('s1', { type: 'turn_end' })).resolves.toBe(0);
    expect(fake.calls).toHaveLength(0);
  });

  it('stops delivering after unbind', async () => {
    const fake = fakeIii();
    const { emit } = createAgentFeeds(fake.iii);
    await fake.bind(AGENT, 's1');
    await emit('s1', { n: 1 });
    await fake.unbind(AGENT, `${AGENT}:s1`);
    await emit('s1', { n: 2 });
    expect(fake.feedEvents(AGENT)).toEqual([{ n: 1 }]);
  });

  it('rejects invalid configs', async () => {
    const fake = fakeIii();
    createAgentFeeds(fake.iii);
    const handler = fake.triggerTypes.get(AGENT);
    const reg = (config: unknown) =>
      handler?.registerTrigger({ id: 'x', function_id: 'f::x', config });
    await expect(reg({})).rejects.toThrow(/session_id/);
    await expect(reg({ session_id: '' })).rejects.toThrow(/session_id/);
    await expect(reg({ session_id: 7 })).rejects.toThrow(/session_id/);
    await expect(reg({ session_id: 'x'.repeat(513) })).rejects.toThrow(/512/);
    await expect(reg({ session_id: 's', group_id: 's' })).rejects.toThrow(/unknown config key/);
    await expect(reg({ session_id: 's', metadata: 'nope' })).rejects.toThrow(/metadata/);
    await expect(reg({ session_id: 's', metadata: [1] })).rejects.toThrow(/metadata/);
    await expect(reg(null)).rejects.toThrow(/object/);
    expect(validateFeedConfig(AGENT, { session_id: 'x'.repeat(512) }).session_id).toHaveLength(512);
  });

  it('caps bindings per trigger type, while re-registering an id replaces it', async () => {
    const fake = fakeIii();
    const feed = createFeed(fake.iii, { id: 'cap::feed', source: 'opencode', description: 'd' });
    const handler = fake.triggerTypes.get('cap::feed');
    for (let i = 0; i < MAX_BINDINGS; i++) {
      await handler?.registerTrigger({
        id: `b${i}`,
        function_id: 'f',
        config: { session_id: 's' },
      });
    }
    expect(feed.bindingCount()).toBe(MAX_BINDINGS);
    await expect(
      handler?.registerTrigger({ id: 'b0', function_id: 'g', config: { session_id: 's' } }),
    ).resolves.toBeUndefined();
    await expect(
      handler?.registerTrigger({ id: 'overflow', function_id: 'f', config: { session_id: 's' } }),
    ).rejects.toThrow(/binding limit/);
    expect(feed.bindingCount()).toBe(MAX_BINDINGS);
    await handler?.unregisterTrigger({ id: 'b1', function_id: 'f', config: {} });
    await expect(
      handler?.registerTrigger({ id: 'overflow', function_id: 'f', config: { session_id: 's' } }),
    ).resolves.toBeUndefined();
  });

  it('numbers frames contiguously from 0 per feed and session, agent and raw independently', async () => {
    const fake = fakeIii();
    const { emit, emitRaw } = createAgentFeeds(fake.iii);
    for (const s of ['s1', 's2']) {
      await fake.bind(AGENT, s);
      await fake.bind(RAW, s);
    }
    await emitRaw('s1', { r: 0 });
    await emit('s1', { a: 0 });
    await emitRaw('s1', { r: 1 });
    await emit('s2', { a: 0 });
    await emit('s1', { a: 1 });
    await emitRaw('s1', { r: 2 });
    const agent = fake.feedPayloads(AGENT);
    const raw = fake.feedPayloads(RAW);
    expect(agent.filter((p) => p.session_id === 's1').map((p) => p.seq)).toEqual([0, 1]);
    expect(agent.filter((p) => p.session_id === 's2').map((p) => p.seq)).toEqual([0]);
    expect(raw.map((p) => p.seq)).toEqual([0, 1, 2]);
    for (const p of [...agent, ...raw]) {
      const match = EVENT_ID.exec(p.event_id);
      expect(match?.[1]).toBe(p.session_id);
      expect(match?.[2]).toBe(p.epoch);
      expect(Number(match?.[3])).toBe(p.seq);
      expect(p.source).toBe('opencode');
    }
    expect(new Set(agent.map((p) => p.epoch)).size).toBe(1);
  });

  it('logs and swallows a failing delivery, then keeps delivering to other bindings', async () => {
    const fake = fakeIii();
    const original = fake.iii.trigger.bind(fake.iii);
    (fake.iii as { trigger: typeof fake.iii.trigger }).trigger = (async (req: {
      function_id: string;
    }) => {
      if (req.function_id === 'f::gone') throw new Error('function_not_found');
      return original(req as never);
    }) as typeof fake.iii.trigger;
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const { emit } = createAgentFeeds(fake.iii);
    await fake.bind(AGENT, 's1', { id: 'gone', function_id: 'f::gone' });
    await fake.bind(AGENT, 's1');
    await expect(emit('s1', { type: 'turn_end' })).resolves.toBe(1);
    expect(fake.feedEvents(AGENT)).toEqual([{ type: 'turn_end' }]);
    expect(warn).toHaveBeenCalledWith(expect.stringContaining('f::gone'));
    expect(consumerFor(AGENT)).toBe('consumer::opencode::agent-event');
    warn.mockRestore();
  });
});

const OBSERVABLES = [
  [AGENT, AGENT_EVENT_OBSERVABLE, 'emit'],
  [RAW, RAW_EVENT_OBSERVABLE, 'emitRaw'],
] as const;

describe('observable feed functions (iii 0.25)', () => {
  it('registers both on-* functions owner-delivered with FeedConfig args, next to the trigger types', () => {
    const fake = fakeIii();
    createAgentFeeds(fake.iii);
    expect(AGENT_EVENT_OBSERVABLE).toBe('opencode::on-agent-event');
    expect(RAW_EVENT_OBSERVABLE).toBe('opencode::on-raw-event');
    expect([...fake.observeHooks.keys()].sort()).toEqual(
      [AGENT_EVENT_OBSERVABLE, RAW_EVENT_OBSERVABLE].sort(),
    );
    for (const [, fnId] of OBSERVABLES) {
      const hooks = fake.observeHooks.get(fnId);
      expect(hooks?.delivery).toBe('owner');
      expect(hooks?.args_format).toMatchObject({
        type: 'object',
        required: ['session_id'],
        additionalProperties: false,
      });
    }
    // The trigger types stay registered next to them.
    expect([...fake.triggerTypes.keys()].sort()).toEqual([AGENT, RAW].sort());
  });

  it('createFeed without an observableId registers no function', () => {
    const fake = fakeIii();
    createFeed(fake.iii, { id: 'plain::feed', source: 'opencode', description: 'd' });
    expect(fake.registered.size).toBe(0);
    expect(fake.triggerTypes.has('plain::feed')).toBe(true);
  });

  it.each(OBSERVABLES)('%s: direct invocation of %s is refused with FORBIDDEN', async (_, fnId) => {
    const fake = fakeIii();
    createAgentFeeds(fake.iii);
    const handler = fake.registered.get(fnId);
    expect(handler).toBeDefined();
    for (const payload of [
      {},
      { session_id: 's1' },
      { session_id: 's1', event: { forged: true } },
    ]) {
      const err = await handler?.(payload).then(
        () => undefined,
        (e: unknown) => e,
      );
      expect(err).toBeInstanceOf(InvocationError);
      expect(err).toMatchObject({ code: 'FORBIDDEN' });
    }
    // Nothing was delivered by the refused calls.
    expect(fake.calls).toEqual([]);
  });

  it.each(
    OBSERVABLES,
  )('%s: an observer receives the frame a binding receives', async (typeId, fnId, emitter) => {
    const fake = fakeIii();
    const feeds = createAgentFeeds(fake.iii);
    await fake.bind(typeId, 's1');
    const observer = await fake.observe(fnId, { session_id: 's1' });
    const elsewhere = await fake.observe(fnId, { session_id: 's2' });

    await expect(feeds[emitter]('s1', { type: 'turn_end' })).resolves.toBe(2);

    const bound = fake.feedPayloads(typeId);
    expect(bound).toHaveLength(1);
    expect(observer.received).toEqual(bound);
    expect(bound[0]).toMatchObject({
      session_id: 's1',
      seq: 0,
      source: 'opencode',
      event: { type: 'turn_end' },
    });
    expect(elsewhere.call).not.toHaveBeenCalled();
  });

  it('an observer alone gets frames with the shared seq, and none after removal', async () => {
    const fake = fakeIii();
    const { emit, emitRaw } = createAgentFeeds(fake.iii);
    const observer = await fake.observe(RAW_EVENT_OBSERVABLE, { session_id: 's1' });
    await emitRaw('s1', { n: 0 });
    await emitRaw('s1', { n: 1 });
    expect(observer.received.map((p) => (p as { seq: number }).seq)).toEqual([0, 1]);
    // The agent feed is its own: the raw observer sees none of it.
    await emit('s1', { n: 9 });
    expect(observer.received).toHaveLength(2);
    expect(fake.calls).toEqual([]);

    await fake.unobserve(RAW_EVENT_OBSERVABLE, observer);
    await expect(emitRaw('s1', { n: 2 })).resolves.toBe(0);
    expect(observer.received).toHaveLength(2);
  });

  it('rejects observer args the trigger config would refuse', async () => {
    const fake = fakeIii();
    const { emit } = createAgentFeeds(fake.iii);
    for (const args of [{}, { session_id: '' }, { session_id: 's1', bogus: 1 }, null]) {
      await expect(fake.observe(AGENT_EVENT_OBSERVABLE, args)).rejects.toMatchObject({
        code: 'INVALID_ARGS',
      });
    }
    await expect(emit('s1', { type: 'turn_end' })).resolves.toBe(0);
  });

  it('counts observers against the binding cap', async () => {
    const fake = fakeIii();
    const feed = createFeed(fake.iii, {
      id: 'cap::feed',
      observableId: 'cap::on-feed',
      source: 'opencode',
      description: 'd',
    });
    for (let i = 0; i < MAX_BINDINGS; i++) await fake.bind('cap::feed', 's1', { id: `b${i}` });
    await expect(fake.observe('cap::on-feed', { session_id: 's1' })).rejects.toMatchObject({
      code: 'BINDING_LIMIT',
    });
    expect(feed.bindingCount()).toBe(MAX_BINDINGS);
  });

  it('swallows an observer that skips or fails and keeps delivering', async () => {
    const fake = fakeIii();
    const { emit } = createAgentFeeds(fake.iii);
    const skipped = await fake.observe(AGENT_EVENT_OBSERVABLE, { session_id: 's1' });
    skipped.call.mockRejectedValueOnce(
      Object.assign(new Error('skip'), { code: 'FILTERS_NOT_MATCHED' }),
    );
    const broken = await fake.observe(AGENT_EVENT_OBSERVABLE, { session_id: 's1' });
    broken.call.mockRejectedValueOnce(new Error('target gone'));
    const ok = await fake.observe(AGENT_EVENT_OBSERVABLE, { session_id: 's1' });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    await expect(emit('s1', { type: 'turn_end' })).resolves.toBe(1);
    expect(ok.received).toHaveLength(1);
    expect(warn).toHaveBeenCalledTimes(1);
    expect(warn).toHaveBeenCalledWith(expect.stringContaining(broken.target));
    warn.mockRestore();
  });
});
