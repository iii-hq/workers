import { TriggerAction } from 'iii-sdk';
import { describe, expect, it, vi } from 'vitest';
import {
  AGENT_EVENT_TRIGGER_TYPE,
  createAgentFeeds,
  createFeed,
  MAX_BINDINGS,
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
