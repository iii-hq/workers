import { describe, expect, it, vi } from 'vitest';
import {
  FEED_ARGS_FORMAT,
  MAX_OBSERVERS,
  ON_AGENT_EVENT_FN,
  ON_RAW_EVENT_FN,
  registerAgentFeeds,
  SOURCE,
} from '../src/agent-feed.js';
import { fakeIii } from './_helpers/fake-iii.js';

const FEEDS = [ON_AGENT_EVENT_FN, ON_RAW_EVENT_FN] as const;

/** What the SDK does to a private function's payload under NODE_ENV=development. */
function deepFreeze<T>(value: T): T {
  if (value !== null && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const member of Object.values(value)) deepFreeze(member);
  }
  return value;
}

function setup() {
  const fake = fakeIii();
  const feeds = registerAgentFeeds(fake.iii);
  const feedOf = (functionId: string) =>
    functionId === ON_AGENT_EVENT_FN ? feeds.agent : feeds.raw;
  return { fake, feeds, feedOf };
}

describe('agent feeds', () => {
  it('registers both feeds as private functions with session args, and nothing on iii-stream', () => {
    const { fake } = setup();
    for (const functionId of FEEDS) {
      const options = fake.functions.get(functionId)?.options;
      expect(options).toMatchObject({ private: true, args_format: FEED_ARGS_FORMAT });
      expect(options?.request_format).toEqual(options?.response_format);
    }
    expect(FEED_ARGS_FORMAT).toMatchObject({
      required: ['session_id'],
      additionalProperties: false,
      properties: { session_id: { type: 'string', minLength: 1, maxLength: 512 } },
    });
    expect(fake.calls).toEqual([]);
  });

  it.each(FEEDS)('%s: the handler returns the frame it is called with', async (functionId) => {
    const { fake } = setup();
    const frame = {
      session_id: 's1',
      event_id: 'e',
      seq: 0,
      epoch: 'x',
      source: SOURCE,
      event: {},
    };
    expect(await fake.registered.get(functionId)?.(frame)).toBe(frame);
  });

  it.each(
    FEEDS,
  )('%s: calls itself fire-and-forget and iii delivers the frame', async (functionId) => {
    const { fake, feedOf } = setup();
    await fake.observeFeed(functionId, 's1', { target: 'acp::__on_event::c1' });
    const event = { type: 'message_complete', message: { role: 'assistant', content: [] } };
    await expect(feedOf(functionId).emit('s1', event)).resolves.toBe(true);

    expect(fake.calls).toHaveLength(1);
    const [call] = fake.calls;
    expect(call.function_id).toBe(functionId);
    expect(call.action).toEqual({ type: 'void' });
    expect(call).not.toHaveProperty('metadata');

    expect(fake.deliveries).toHaveLength(1);
    const [delivery] = fake.deliveries;
    expect(delivery.target).toBe('acp::__on_event::c1');
    expect(Object.keys(delivery.payload).sort()).toEqual(
      ['epoch', 'event', 'event_id', 'seq', 'session_id', 'source'].sort(),
    );
    expect(delivery.payload).toMatchObject({ session_id: 's1', seq: 0, source: SOURCE, event });
    expect(typeof delivery.payload.epoch).toBe('string');
  });

  it.each(FEEDS)('%s: an observer of session A never receives session B', async (functionId) => {
    const { fake, feedOf } = setup();
    await fake.observeFeed(functionId, 'A', { target: 'c::a' });
    await fake.observeFeed(functionId, 'B', { target: 'c::b' });
    await feedOf(functionId).emit('B', { n: 1 });
    await feedOf(functionId).emit('B', { n: 2 });
    await feedOf(functionId).emit('A', { n: 3 });
    const events = (target: string) =>
      fake.deliveries.filter((d) => d.target === target).map((d) => d.payload.event);
    expect(events('c::a')).toEqual([{ n: 3 }]);
    expect(events('c::b')).toEqual([{ n: 1 }, { n: 2 }]);
  });

  it.each(FEEDS)('%s: matches observers by session id keys', async (functionId) => {
    const { fake } = setup();
    expect(fake.iii.isObserved(functionId, { input: { session_id: 's1' } })).toBe(false);
    await fake.observeFeed(functionId, 's1');
    expect(fake.iii.isObserved(functionId, { input: { session_id: 's1' } })).toBe(true);
    expect(fake.iii.isObserved(functionId, { input: { session_id: 's2' } })).toBe(false);
  });

  it('makes no call at all when nobody observes the session', async () => {
    const { fake, feeds } = setup();
    await fake.observeFeed(ON_AGENT_EVENT_FN, 'other');
    await expect(feeds.agent.emit('s1', { type: 'turn_end' })).resolves.toBe(false);
    await expect(feeds.raw.emit('s1', { type: 'result' })).resolves.toBe(false);
    expect(fake.calls).toEqual([]);
  });

  it.each(FEEDS)('%s: stops delivering once the observer is removed', async (functionId) => {
    const { fake, feedOf } = setup();
    const id = await fake.observeFeed(functionId, 's1');
    await feedOf(functionId).emit('s1', { n: 1 });
    await fake.unobserve(id);
    expect(fake.observersOf(functionId)).toHaveLength(0);
    await expect(feedOf(functionId).emit('s1', { n: 2 })).resolves.toBe(false);
    expect(fake.feedFrames(functionId).map((f) => f.event)).toEqual([{ n: 1 }]);
  });

  it('caps observers per feed with OBSERVER_LIMIT', async () => {
    const { fake } = setup();
    for (let i = 0; i < MAX_OBSERVERS; i++) {
      await fake.observeFeed(ON_AGENT_EVENT_FN, `s${i}`, { id: `o${i}` });
    }
    expect(fake.observersOf(ON_AGENT_EVENT_FN)).toHaveLength(MAX_OBSERVERS);
    await expect(
      fake.observeFeed(ON_AGENT_EVENT_FN, 'late', { id: 'o-new' }),
    ).rejects.toMatchObject({
      code: 'OBSERVER_LIMIT',
    });
    expect(fake.observersOf(ON_AGENT_EVENT_FN)).toHaveLength(MAX_OBSERVERS);
    // The cap is per feed: the raw feed still accepts observers.
    await expect(fake.observeFeed(ON_RAW_EVENT_FN, 'late')).resolves.toBeTypeOf('string');
    // A removed observer frees its slot.
    await fake.unobserve('o0');
    await expect(fake.observeFeed(ON_AGENT_EVENT_FN, 'late', { id: 'o-new' })).resolves.toBe(
      'o-new',
    );
  });

  it('numbers seq from 0 per (feed, session) with independent agent and raw counters', async () => {
    const { fake, feeds } = setup();
    for (const functionId of FEEDS) {
      await fake.observeFeed(functionId, 's1');
      await fake.observeFeed(functionId, 's2');
    }
    // Interleave the feeds the way a turn does (raw, then translated).
    await feeds.raw.emit('s1', { r: 0 });
    await feeds.agent.emit('s1', { a: 0 });
    await feeds.raw.emit('s1', { r: 1 });
    await feeds.raw.emit('s2', { r: 0 });
    await feeds.agent.emit('s1', { a: 1 });
    await feeds.agent.emit('s2', { a: 0 });
    await feeds.agent.emit('s1', { a: 2 });

    const seqs = (functionId: string, sid: string) =>
      fake
        .feedFrames(functionId)
        .filter((f) => f.session_id === sid)
        .map((f) => f.seq);
    expect(seqs(ON_AGENT_EVENT_FN, 's1')).toEqual([0, 1, 2]);
    expect(seqs(ON_AGENT_EVENT_FN, 's2')).toEqual([0]);
    expect(seqs(ON_RAW_EVENT_FN, 's1')).toEqual([0, 1]);
    expect(seqs(ON_RAW_EVENT_FN, 's2')).toEqual([0]);

    for (const functionId of FEEDS) {
      const frames = fake.feedFrames(functionId);
      const epochs = new Set(frames.map((f) => f.epoch));
      expect(epochs.size).toBe(1);
      for (const f of frames) {
        expect(f.event_id).toBe(`${f.session_id}-${f.epoch}-${String(f.seq).padStart(8, '0')}`);
        expect(String(f.event_id)).toMatch(/-\d{8}$/);
      }
    }
  });

  it('advances seq for frames nobody observes', async () => {
    const { fake, feeds } = setup();
    await feeds.agent.emit('s1', { n: 0 });
    await feeds.agent.emit('s1', { n: 1 });
    await fake.observeFeed(ON_AGENT_EVENT_FN, 's1');
    await feeds.agent.emit('s1', { n: 2 });
    expect(fake.feedFrames(ON_AGENT_EVENT_FN)).toEqual([
      expect.objectContaining({ seq: 2, event: { n: 2 } }),
    ]);
  });

  it('uses a stable event id when one is given', async () => {
    const { fake, feeds } = setup();
    await fake.observeFeed(ON_AGENT_EVENT_FN, 's1');
    await feeds.agent.emit('s1', { n: 1 }, 'stable-1');
    expect(fake.feedFrames(ON_AGENT_EVENT_FN)[0]).toMatchObject({ event_id: 'stable-1', seq: 0 });
  });

  it.each(
    FEEDS,
  )('%s: sends a copy of the event, so the caller keeps mutating its own objects', async (functionId) => {
    const { fake, feedOf } = setup();
    await fake.observeFeed(functionId, 's1');
    // The SDK hands a private function's payload to its handler by reference
    // and, under NODE_ENV=development, deep-freezes it.
    const real = fake.iii.call.bind(fake.iii);
    (fake.iii as { call: unknown }).call = async (req: { payload: unknown }) =>
      real({ ...req, payload: deepFreeze(req.payload) } as never);

    // pi-agent-core's message_update: the provider's live partial, and a
    // shallow copy of it that shares its content array.
    const output = { role: 'assistant', content: [{ type: 'text', text: 'a' }] };
    const event = {
      type: 'message_update',
      assistantMessageEvent: { type: 'text_delta', partial: output },
      message: { ...output },
    };
    await expect(feedOf(functionId).emit('s1', event)).resolves.toBe(true);

    // pi-ai's next deltas mutate the same objects.
    expect(Object.isFrozen(output)).toBe(false);
    expect(Object.isFrozen(output.content)).toBe(false);
    output.content[0].text += 'b';
    output.content.push({ type: 'text', text: 'c' });
    expect(event.message.content).toEqual([
      { type: 'text', text: 'ab' },
      { type: 'text', text: 'c' },
    ]);

    // The frame is the event as it was when emitted.
    expect(fake.feedFrames(functionId).map((f) => f.event)).toEqual([
      {
        type: 'message_update',
        assistantMessageEvent: {
          type: 'text_delta',
          partial: { role: 'assistant', content: [{ type: 'text', text: 'a' }] },
        },
        message: { role: 'assistant', content: [{ type: 'text', text: 'a' }] },
      },
    ]);
  });

  it('sends an event structuredClone refuses as its JSON form', async () => {
    const { fake, feeds } = setup();
    await fake.observeFeed(ON_RAW_EVENT_FN, 's1');
    const event = { type: 'tool_execution_start', args: { path: 'a' }, abort: () => {} };
    await expect(feeds.raw.emit('s1', event)).resolves.toBe(true);
    expect(fake.feedFrames(ON_RAW_EVENT_FN)[0].event).toEqual({
      type: 'tool_execution_start',
      args: { path: 'a' },
    });
  });

  it('logs an event it cannot copy and resolves false without throwing', async () => {
    const { fake, feeds } = setup();
    await fake.observeFeed(ON_RAW_EVENT_FN, 's1');
    const event: Record<string, unknown> = { type: 'cyclic', abort: () => {} };
    event.self = event;
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    await expect(feeds.raw.emit('s1', event)).resolves.toBe(false);
    expect(warn).toHaveBeenCalledWith(expect.stringContaining(ON_RAW_EVENT_FN));
    expect(fake.calls).toEqual([]);
    warn.mockRestore();
  });

  it('logs a refused call and resolves false without throwing', async () => {
    const { fake, feeds } = setup();
    await fake.observeFeed(ON_AGENT_EVENT_FN, 's1');
    const real = fake.iii.call.bind(fake.iii);
    (fake.iii as { call: unknown }).call = async (req: { function_id: string }) => {
      if (req.function_id === ON_AGENT_EVENT_FN) throw new Error('OVERFLOW');
      return real(req as never);
    };
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    await expect(feeds.agent.emit('s1', { type: 'turn_end' })).resolves.toBe(false);
    expect(warn).toHaveBeenCalledWith(expect.stringContaining(ON_AGENT_EVENT_FN));
    warn.mockRestore();
  });
});
