import { CallAction } from 'iii-sdk';
import { describe, expect, it, vi } from 'vitest';
import {
  createAgentFeeds,
  createFeed,
  FEED_ARGS_FORMAT,
  MAX_OBSERVERS,
  ON_AGENT_EVENT_FN,
  ON_RAW_EVENT_FN,
} from '../src/agent-feed.js';
import { AGENT, consumerFor, fakeIii, RAW } from './_helpers/fake-iii.js';

const EVENT_ID = /^(.+)-([0-9a-f-]{36})-(\d{8})$/;

/** What the SDK does to a private function's payload under NODE_ENV=development. */
function deepFreeze<T>(value: T): T {
  if (value !== null && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const member of Object.values(value)) deepFreeze(member);
  }
  return value;
}

describe('agent feeds', () => {
  it('registers both feeds as private functions with session args', () => {
    const fake = fakeIii();
    createAgentFeeds(fake.iii);
    expect([...fake.functions.keys()].sort()).toEqual([
      'opencode::on-agent-event',
      'opencode::on-raw-event',
    ]);
    for (const id of [AGENT, RAW]) {
      const options = fake.functions.get(id)?.options;
      expect(options).toMatchObject({ private: true, args_format: FEED_ARGS_FORMAT });
      expect(options?.request_format).toEqual(options?.response_format);
    }
    expect(FEED_ARGS_FORMAT).toMatchObject({
      required: ['session_id'],
      additionalProperties: false,
      properties: { session_id: { type: 'string', minLength: 1, maxLength: 512 } },
    });
    expect(ON_AGENT_EVENT_FN).toBe(AGENT);
    expect(ON_RAW_EVENT_FN).toBe(RAW);
  });

  it('returns the frame it is called with', async () => {
    const fake = fakeIii();
    createAgentFeeds(fake.iii);
    const frame = {
      session_id: 's1',
      event_id: 'e',
      seq: 0,
      epoch: 'x',
      source: 'opencode',
      event: {},
    };
    expect(await fake.registered.get(AGENT)?.(frame)).toBe(frame);
  });

  it('calls itself with a Void action and iii delivers the frame to the observer', async () => {
    const fake = fakeIii();
    const { emit } = createAgentFeeds(fake.iii);
    await fake.observe(AGENT, 's1', { id: 'o1', target: 'acp::__on_event::c1' });
    await expect(emit('s1', { type: 'agent_end', messages: [] })).resolves.toBe(true);
    expect(fake.calls).toHaveLength(1);
    expect(fake.calls[0]).toMatchObject({ function_id: AGENT, action: CallAction.Void() });
    expect(fake.calls[0]).not.toHaveProperty('metadata');
    const delivery = fake.deliveries.find((d) => d.target === 'acp::__on_event::c1');
    expect(delivery?.payload).toMatchObject({
      session_id: 's1',
      seq: 0,
      source: 'opencode',
      event: { type: 'agent_end', messages: [] },
    });
    const payload = delivery?.payload as { event_id: string; epoch: string };
    expect(payload.event_id).toBe(`s1-${payload.epoch}-00000000`);
  });

  it('only delivers a session to observers of that session', async () => {
    const fake = fakeIii();
    const { emit, emitRaw } = createAgentFeeds(fake.iii);
    await fake.observe(AGENT, 'A', { target: 'f::a' });
    await fake.observe(RAW, 'A', { target: 'f::a-raw' });
    await expect(emit('B', { type: 'turn_end' })).resolves.toBe(false);
    await expect(emitRaw('B', { type: 'text' })).resolves.toBe(false);
    expect(fake.calls).toHaveLength(0);
    await emit('A', { type: 'turn_end' });
    expect(fake.deliveries.map((d) => d.target)).toEqual(['f::a']);
  });

  it('matches observers by session id keys', async () => {
    const fake = fakeIii();
    createAgentFeeds(fake.iii);
    expect(fake.iii.isObserved(AGENT, { input: { session_id: 's1' } })).toBe(false);
    await fake.observe(AGENT, 's1');
    expect(fake.iii.isObserved(AGENT, { input: { session_id: 's1' } })).toBe(true);
    expect(fake.iii.isObserved(AGENT, { input: { session_id: 's2' } })).toBe(false);
    expect(fake.iii.isObserved(RAW, { input: { session_id: 's1' } })).toBe(false);
  });

  it('makes no call when nothing observes the feed', async () => {
    const fake = fakeIii();
    const { emit } = createAgentFeeds(fake.iii);
    await expect(emit('s1', { type: 'turn_end' })).resolves.toBe(false);
    expect(fake.calls).toHaveLength(0);
  });

  it('stops delivering once the observer is removed', async () => {
    const fake = fakeIii();
    const { emit } = createAgentFeeds(fake.iii);
    await fake.observe(AGENT, 's1');
    await emit('s1', { n: 1 });
    await fake.unobserve(`${AGENT}:s1`);
    await emit('s1', { n: 2 });
    expect(fake.feedEvents(AGENT)).toEqual([{ n: 1 }]);
  });

  it('caps observers per feed with OBSERVER_LIMIT, and a removed observer frees its slot', async () => {
    const fake = fakeIii();
    createFeed(fake.iii, { id: 'cap::feed', source: 'opencode', description: 'd' });
    for (let i = 0; i < MAX_OBSERVERS; i++) {
      await fake.observe('cap::feed', 's', { id: `o${i}` });
    }
    expect(fake.observersOf('cap::feed')).toHaveLength(MAX_OBSERVERS);
    await expect(fake.observe('cap::feed', 's', { id: 'overflow' })).rejects.toMatchObject({
      code: 'OBSERVER_LIMIT',
    });
    expect(fake.observersOf('cap::feed')).toHaveLength(MAX_OBSERVERS);
    await fake.unobserve('o1');
    await expect(fake.observe('cap::feed', 's', { id: 'overflow' })).resolves.toBe('overflow');
  });

  it('numbers frames contiguously from 0 per feed and session, agent and raw independently', async () => {
    const fake = fakeIii();
    const { emit, emitRaw } = createAgentFeeds(fake.iii);
    for (const s of ['s1', 's2']) {
      await fake.observe(AGENT, s);
      await fake.observe(RAW, s);
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

  it('advances seq for frames nobody observes', async () => {
    const fake = fakeIii();
    const { emit } = createAgentFeeds(fake.iii);
    await emit('s1', { n: 0 });
    await fake.observe(AGENT, 's1');
    await emit('s1', { n: 1 });
    expect(fake.feedPayloads(AGENT).map((p) => p.seq)).toEqual([1]);
  });

  it.each([
    AGENT,
    RAW,
  ])('%s: sends a copy of the event, so the caller keeps mutating its own objects', async (id) => {
    const fake = fakeIii();
    // The SDK hands a private function's payload to its handler by reference
    // and, under NODE_ENV=development, deep-freezes it.
    const original = fake.iii.call.bind(fake.iii);
    (fake.iii as { call: typeof fake.iii.call }).call = (async (req: { payload: unknown }) =>
      original({ ...req, payload: deepFreeze(req.payload) } as never)) as typeof fake.iii.call;
    const feeds = createAgentFeeds(fake.iii);
    const emit = id === AGENT ? feeds.emit : feeds.emitRaw;
    await fake.observe(id, 's1');

    const part = { type: 'text', text: 'a' };
    const event = { type: 'text', part };
    await expect(emit('s1', event)).resolves.toBe(true);

    expect(Object.isFrozen(event)).toBe(false);
    expect(Object.isFrozen(part)).toBe(false);
    part.text += 'b';

    // The frame is the event as it was when emitted.
    expect(fake.feedEvents(id)).toEqual([{ type: 'text', part: { type: 'text', text: 'a' } }]);
  });

  it('sends an event structuredClone refuses as its JSON form', async () => {
    const fake = fakeIii();
    const { emitRaw } = createAgentFeeds(fake.iii);
    await fake.observe(RAW, 's1');
    await expect(emitRaw('s1', { type: 'step_start', abort: () => {} })).resolves.toBe(true);
    expect(fake.feedEvents(RAW)).toEqual([{ type: 'step_start' }]);
  });

  it('logs an event it cannot copy and resolves false', async () => {
    const fake = fakeIii();
    const { emitRaw } = createAgentFeeds(fake.iii);
    await fake.observe(RAW, 's1');
    const event: Record<string, unknown> = { type: 'cyclic', abort: () => {} };
    event.self = event;
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    await expect(emitRaw('s1', event)).resolves.toBe(false);
    expect(warn).toHaveBeenCalledWith(expect.stringContaining(RAW));
    expect(fake.calls).toHaveLength(0);
    warn.mockRestore();
  });

  it('logs a refused call and resolves false', async () => {
    const fake = fakeIii();
    const original = fake.iii.call.bind(fake.iii);
    (fake.iii as { call: typeof fake.iii.call }).call = (async (req: { function_id: string }) => {
      if (req.function_id === AGENT) throw new Error('OVERFLOW');
      return original(req as never);
    }) as typeof fake.iii.call;
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const { emit } = createAgentFeeds(fake.iii);
    await fake.observe(AGENT, 's1');
    await expect(emit('s1', { type: 'turn_end' })).resolves.toBe(false);
    expect(warn).toHaveBeenCalledWith(expect.stringContaining(AGENT));
    expect(consumerFor(AGENT)).toBe('consumer::opencode::on-agent-event');
    warn.mockRestore();
  });
});
