import { CallAction } from 'iii-sdk';
import { describe, expect, it, vi } from 'vitest';
import {
  createAgentFeeds,
  createFeed,
  FEED_ARGS_FORMAT,
  MAX_OBSERVERS,
  ON_AGENT_EVENT_FN,
  ON_RAW_EVENT_FN,
  releaseEmitterSequence,
} from '../src/agent-feed.js';
import { AGENT, MockIII, RAW } from './helpers.js';

const EVENT_ID = /^(.+)-([0-9a-f-]{36}-[0-9a-f-]{36})-(\d{8})$/;

/** What the SDK does to a private function's payload under NODE_ENV=development. */
function deepFreeze<T>(value: T): T {
  if (value !== null && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const member of Object.values(value)) deepFreeze(member);
  }
  return value;
}

describe('cursor agent feeds', () => {
  it('registers both feeds as private functions with session args', () => {
    const iii = new MockIII();
    createAgentFeeds(iii.asClient());
    expect([...iii.functions.keys()].sort()).toEqual([
      'cursor::on-agent-event',
      'cursor::on-raw-event',
    ]);
    for (const id of [AGENT, RAW]) {
      const options = iii.functions.get(id)?.options;
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
    const iii = new MockIII();
    createAgentFeeds(iii.asClient());
    const frame = {
      session_id: 's',
      event_id: 'e',
      seq: 0,
      epoch: 'x',
      source: 'cursor',
      event: {},
    };
    expect(await iii.functions.get(AGENT)?.handler(frame)).toBe(frame);
  });

  it('calls itself with a Void action and iii delivers the frame to the observer', async () => {
    const iii = new MockIII();
    const { emit } = createAgentFeeds(iii.asClient());
    await iii.addObserver(AGENT, 'a-1', { id: 'o1', target: 'acp::__on_event::c1' });
    await expect(emit('a-1', { type: 'turn_end' })).resolves.toBe(true);
    expect(iii.calls).toHaveLength(1);
    expect(iii.calls[0]).toMatchObject({ function_id: AGENT, action: CallAction.Void() });
    expect(iii.calls[0]).not.toHaveProperty('metadata');
    const delivery = iii.deliveries.find((d) => d.target === 'acp::__on_event::c1');
    expect(delivery?.payload).toMatchObject({
      session_id: 'a-1',
      seq: 0,
      source: 'cursor',
      event: { type: 'turn_end' },
    });
    const payload = delivery?.payload as { event_id: string; epoch: string };
    expect(payload.event_id).toBe(`a-1-${payload.epoch}-00000000`);
  });

  it('delivers by session and reports false when nobody observes the session', async () => {
    const iii = new MockIII();
    const { emit, emitRaw } = createAgentFeeds(iii.asClient());
    await iii.addObserver(AGENT, 'A', { target: 'f::a' });
    await iii.addObserver(RAW, 'A', { target: 'f::a-raw' });
    await expect(emit('B', { type: 'turn_end' })).resolves.toBe(false);
    await expect(emitRaw('B', { type: 'frame' })).resolves.toBe(false);
    expect(iii.calls).toHaveLength(0);
    await expect(emit('A', { type: 'turn_end' })).resolves.toBe(true);
    expect(iii.deliveries.map((d) => d.target)).toEqual(['f::a']);
  });

  it('matches observers by session id keys', async () => {
    const iii = new MockIII();
    createAgentFeeds(iii.asClient());
    expect(iii.isObserved(AGENT, { input: { session_id: 's' } })).toBe(false);
    await iii.addObserver(AGENT, 's');
    expect(iii.isObserved(AGENT, { input: { session_id: 's' } })).toBe(true);
    expect(iii.isObserved(AGENT, { input: { session_id: 't' } })).toBe(false);
    expect(iii.isObserved(RAW, { input: { session_id: 's' } })).toBe(false);
  });

  it('stops delivering once the observer is removed', async () => {
    const iii = new MockIII();
    const { emit } = createAgentFeeds(iii.asClient());
    await iii.addObserver(AGENT, 'u-1');
    await emit('u-1', { n: 1 });
    await iii.removeObserver(`${AGENT}:u-1`);
    await expect(emit('u-1', { n: 2 })).resolves.toBe(false);
    expect(iii.feedEvents(AGENT)).toEqual([{ n: 1 }]);
  });

  it('caps observers per feed with OBSERVER_LIMIT, and a removed observer frees its slot', async () => {
    const iii = new MockIII();
    createFeed(iii.asClient(), { id: 'cap::feed', source: 'cursor', description: 'd' });
    for (let i = 0; i < MAX_OBSERVERS; i++) {
      await iii.addObserver('cap::feed', 's', { id: `o${i}` });
    }
    await expect(iii.addObserver('cap::feed', 's', { id: 'overflow' })).rejects.toMatchObject({
      code: 'OBSERVER_LIMIT',
    });
    expect(iii.observers('cap::feed')).toHaveLength(MAX_OBSERVERS);
    await iii.removeObserver('o1');
    await expect(iii.addObserver('cap::feed', 's', { id: 'overflow' })).resolves.toBe('overflow');
  });

  it('numbers agent and raw frames independently, contiguous from 0, and keeps stable ids', async () => {
    const iii = new MockIII();
    const { emit, emitRaw } = createAgentFeeds(iii.asClient());
    for (const session of ['q-1', 'q-2']) {
      await iii.addObserver(AGENT, session);
      await iii.addObserver(RAW, session);
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

  it('advances seq for frames nobody observes', async () => {
    const iii = new MockIII();
    const { emit } = createAgentFeeds(iii.asClient());
    await emit('late', { n: 0 });
    await iii.addObserver(AGENT, 'late');
    await emit('late', { n: 1 });
    expect(iii.feedPayloads(AGENT).map((p) => p.seq)).toEqual([1]);
  });

  it.each([
    AGENT,
    RAW,
  ])('%s: sends a copy of the event, so the caller keeps mutating its own objects', async (id) => {
    const iii = new MockIII();
    // The SDK hands a private function's payload to its handler by reference
    // and, under NODE_ENV=development, deep-freezes it.
    const original = iii.call.bind(iii);
    iii.call = async (request: Record<string, unknown>) =>
      original({ ...request, payload: deepFreeze(request.payload) });
    const feeds = createAgentFeeds(iii.asClient());
    const emit = id === AGENT ? feeds.emit : feeds.emitRaw;
    await iii.addObserver(id, 'm-1');

    const content = { type: 'text', text: 'a' };
    const event = {
      method: 'session/update',
      params: { update: { sessionUpdate: 'agent_message_chunk', content } },
    };
    await expect(emit('m-1', event)).resolves.toBe(true);

    expect(Object.isFrozen(event)).toBe(false);
    expect(Object.isFrozen(content)).toBe(false);
    content.text += 'b';

    // The frame is the event as it was when emitted.
    expect(iii.feedEvents(id)).toEqual([
      {
        method: 'session/update',
        params: {
          update: { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'a' } },
        },
      },
    ]);
  });

  it('sends an event structuredClone refuses as its JSON form', async () => {
    const iii = new MockIII();
    const { emitRaw } = createAgentFeeds(iii.asClient());
    await iii.addObserver(RAW, 'j-1');
    await expect(emitRaw('j-1', { type: 'status', cancel: () => undefined })).resolves.toBe(true);
    expect(iii.feedEvents(RAW)).toEqual([{ type: 'status' }]);
  });

  it('logs an event it cannot copy and resolves false', async () => {
    const iii = new MockIII();
    const { emitRaw } = createAgentFeeds(iii.asClient());
    await iii.addObserver(RAW, 'c-1');
    const event: Record<string, unknown> = { type: 'cyclic', cancel: () => undefined };
    event.self = event;
    const warning = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    await expect(emitRaw('c-1', event)).resolves.toBe(false);
    expect(warning).toHaveBeenCalledWith(expect.stringContaining(RAW));
    expect(iii.calls).toEqual([]);
    warning.mockRestore();
  });

  it('logs a refused call and resolves false', async () => {
    const iii = new MockIII();
    const original = iii.call.bind(iii);
    iii.call = async (request: Record<string, unknown>) => {
      if (request.function_id === AGENT) throw new Error('OVERFLOW');
      return original(request);
    };
    const warning = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const { emit } = createAgentFeeds(iii.asClient());
    await iii.addObserver(AGENT, 'g-1');
    await expect(emit('g-1', { type: 'turn_end' })).resolves.toBe(false);
    expect(iii.feedEvents(AGENT)).toEqual([]);
    expect(warning).toHaveBeenCalledWith(expect.stringContaining(AGENT));
    warning.mockRestore();
  });
});
