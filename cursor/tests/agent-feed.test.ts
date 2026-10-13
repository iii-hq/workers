import { TriggerAction } from 'iii-sdk';
import { describe, expect, it, vi } from 'vitest';
import {
  AGENT_EVENT_TRIGGER_TYPE,
  createAgentFeeds,
  createFeed,
  MAX_BINDINGS,
  RAW_EVENT_TRIGGER_TYPE,
  releaseEmitterSequence,
  validateFeedConfig,
} from '../src/agent-feed.js';
import { AGENT, MockIII, RAW } from './helpers.js';

const EVENT_ID = /^(.+)-([0-9a-f-]{36}-[0-9a-f-]{36})-(\d{8})$/;

describe('cursor agent feeds', () => {
  it('registers both owned trigger types', () => {
    const iii = new MockIII();
    createAgentFeeds(iii.asClient());
    expect([...iii.triggerTypes.keys()].sort()).toEqual([
      'cursor::agent-event',
      'cursor::raw-event',
    ]);
    expect(AGENT_EVENT_TRIGGER_TYPE).toBe(AGENT);
    expect(RAW_EVENT_TRIGGER_TYPE).toBe(RAW);
  });

  it('delivers a frame with namespace, config metadata and Void action', async () => {
    const iii = new MockIII();
    const { emit } = createAgentFeeds(iii.asClient());
    await iii.bind(AGENT, 'a-1', {
      id: 'b1',
      function_id: 'acp::__on_event::c1',
      namespace: 'tenant-a',
      metadata: { from: 'binding' },
      config: { session_id: 'a-1', metadata: { from: 'config' } },
    });
    await expect(emit('a-1', { type: 'turn_end' })).resolves.toBe(true);
    const call = iii.triggerCalls.find((c) => c.function_id === 'acp::__on_event::c1');
    expect(call).toMatchObject({
      namespace: 'tenant-a',
      metadata: { from: 'config' },
      action: TriggerAction.Void(),
      payload: { session_id: 'a-1', seq: 0, source: 'cursor', event: { type: 'turn_end' } },
    });
    const payload = call?.payload as { event_id: string; epoch: string };
    expect(payload.event_id).toBe(`a-1-${payload.epoch}-00000000`);
  });

  it('uses binding metadata as fallback and omits metadata when neither is set', async () => {
    const iii = new MockIII();
    const { emit } = createAgentFeeds(iii.asClient());
    await iii.bind(AGENT, 'm-1', { id: 'with', function_id: 'f::with', metadata: { m: 1 } });
    await iii.bind(AGENT, 'm-1', { id: 'without', function_id: 'f::without' });
    await emit('m-1', { type: 'turn_end' });
    expect(iii.triggerCalls.find((c) => c.function_id === 'f::with')?.metadata).toEqual({ m: 1 });
    expect(iii.triggerCalls.find((c) => c.function_id === 'f::without')).not.toHaveProperty(
      'metadata',
    );
  });

  it('filters by session and reports false when no consumer is bound', async () => {
    const iii = new MockIII();
    const { emit, emitRaw } = createAgentFeeds(iii.asClient());
    await iii.bind(AGENT, 'A', { id: 'a', function_id: 'f::a' });
    await iii.bind(RAW, 'A', { id: 'a-raw', function_id: 'f::a-raw' });
    await expect(emit('B', { type: 'turn_end' })).resolves.toBe(false);
    await expect(emitRaw('B', { type: 'frame' })).resolves.toBe(false);
    expect(iii.triggerCalls).toHaveLength(0);
    await expect(emit('A', { type: 'turn_end' })).resolves.toBe(true);
    expect(iii.triggerCalls.map((c) => c.function_id)).toEqual(['f::a']);
  });

  it('stops delivering after unbind', async () => {
    const iii = new MockIII();
    const { emit } = createAgentFeeds(iii.asClient());
    await iii.bind(AGENT, 'u-1');
    await emit('u-1', { n: 1 });
    await iii.unbind(AGENT, `${AGENT}:u-1`);
    await expect(emit('u-1', { n: 2 })).resolves.toBe(false);
    expect(iii.feedEvents(AGENT)).toEqual([{ n: 1 }]);
  });

  it('rejects invalid configs', async () => {
    const iii = new MockIII();
    createAgentFeeds(iii.asClient());
    const handler = iii.triggerTypes.get(RAW);
    const reg = (config: unknown) =>
      handler?.registerTrigger({ id: 'x', function_id: 'f::x', config });
    await expect(reg({})).rejects.toThrow(/session_id/);
    await expect(reg({ session_id: '' })).rejects.toThrow(/session_id/);
    await expect(reg({ session_id: 1 })).rejects.toThrow(/session_id/);
    await expect(reg({ session_id: 'x'.repeat(513) })).rejects.toThrow(/512/);
    await expect(reg({ session_id: 's', stream_name: 'agent::events' })).rejects.toThrow(
      /unknown config key/,
    );
    await expect(reg({ session_id: 's', metadata: 3 })).rejects.toThrow(/metadata/);
    await expect(reg({ session_id: 's', metadata: null })).rejects.toThrow(/metadata/);
    await expect(reg('s')).rejects.toThrow(/object/);
    expect(validateFeedConfig(RAW, { session_id: 's', metadata: { a: 1 } })).toEqual({
      session_id: 's',
      metadata: { a: 1 },
    });
  });

  it('caps bindings per trigger type, while re-registering an id replaces it', async () => {
    const iii = new MockIII();
    const feed = createFeed(iii.asClient(), {
      id: 'cap::feed',
      source: 'cursor',
      description: 'd',
    });
    const handler = iii.triggerTypes.get('cap::feed');
    for (let i = 0; i < MAX_BINDINGS; i++) {
      await handler?.registerTrigger({
        id: `b${i}`,
        function_id: 'f',
        config: { session_id: 's' },
      });
    }
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

  it('numbers agent and raw frames independently, contiguous from 0, and keeps stable ids', async () => {
    const iii = new MockIII();
    const { emit, emitRaw } = createAgentFeeds(iii.asClient());
    for (const session of ['q-1', 'q-2']) {
      await iii.bind(AGENT, session);
      await iii.bind(RAW, session);
    }
    await emitRaw('q-1', { r: 0 }, 'cursor-abc-raw');
    await emit('q-1', { a: 0 }, 'cursor-abc');
    await emitRaw('q-1', { r: 1 });
    await emit('q-2', { a: 0 });
    await emit('q-1', { a: 1 });
    const agent = iii.feedPayloads(AGENT);
    const raw = iii.feedPayloads(RAW);
    expect(agent.filter((p) => p.session_id === 'q-1').map((p) => p.seq)).toEqual([0, 1]);
    expect(agent.filter((p) => p.session_id === 'q-2').map((p) => p.seq)).toEqual([0]);
    expect(raw.map((p) => p.seq)).toEqual([0, 1]);
    expect(agent[0]?.event_id).toBe('cursor-abc');
    expect(raw[0]?.event_id).toBe('cursor-abc-raw');
    for (const p of [...agent.slice(1), raw[1]]) {
      const match = EVENT_ID.exec(p?.event_id ?? '');
      expect(match?.[1]).toBe(p?.session_id);
      expect(match?.[2]).toBe(p?.epoch);
      expect(Number(match?.[3])).toBe(p?.seq);
    }
    releaseEmitterSequence('q-1');
    await emit('q-1', { a: 2 });
    await emitRaw('q-1', { r: 2 });
    const afterAgent = iii.feedPayloads(AGENT).at(-1);
    const afterRaw = iii.feedPayloads(RAW).at(-1);
    expect(afterAgent?.seq).toBe(0);
    expect(afterRaw?.seq).toBe(0);
    expect(afterAgent?.epoch).not.toBe(agent[0]?.epoch);
    expect(afterRaw?.epoch).not.toBe(raw[0]?.epoch);
    expect(iii.feedPayloads(AGENT).filter((p) => p.session_id === 'q-2')).toHaveLength(1);
  });

  it('logs and swallows a failing delivery and still delivers to the other binding', async () => {
    const iii = new MockIII();
    const original = iii.trigger.bind(iii);
    iii.trigger = async (request: Record<string, unknown>) => {
      if (request.function_id === 'f::gone') throw new Error('function_not_found');
      return original(request);
    };
    const warning = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const { emit } = createAgentFeeds(iii.asClient());
    await iii.bind(AGENT, 'g-1', { id: 'gone', function_id: 'f::gone' });
    await iii.bind(AGENT, 'g-1');
    await expect(emit('g-1', { type: 'turn_end' })).resolves.toBe(true);
    expect(iii.feedEvents(AGENT)).toEqual([{ type: 'turn_end' }]);
    expect(warning).toHaveBeenCalledWith(expect.stringContaining('f::gone'));
    warning.mockRestore();
  });
});
