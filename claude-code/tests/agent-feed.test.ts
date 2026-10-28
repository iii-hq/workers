import { describe, expect, it, vi } from 'vitest';
import { InvocationError } from 'iii-sdk';
import {
  AGENT_EVENT_OBSERVABLE,
  AGENT_EVENT_TRIGGER,
  MAX_BINDINGS,
  RAW_EVENT_OBSERVABLE,
  RAW_EVENT_TRIGGER,
  registerAgentFeeds,
  SOURCE,
  validateFeedConfig,
} from '../src/agent-feed.js';
import { fakeIii } from './_helpers/fake-iii.js';

const FEEDS = [AGENT_EVENT_TRIGGER, RAW_EVENT_TRIGGER] as const;

function setup() {
  const fake = fakeIii();
  const feeds = registerAgentFeeds(fake.iii);
  const feedOf = (typeId: string) => (typeId === AGENT_EVENT_TRIGGER ? feeds.agent : feeds.raw);
  return { fake, feeds, feedOf };
}

describe('agent feeds', () => {
  it('registers both owned trigger types and nothing on iii-stream', () => {
    const { fake } = setup();
    expect([...fake.triggerTypes.keys()].sort()).toEqual(
      [AGENT_EVENT_TRIGGER, RAW_EVENT_TRIGGER].sort(),
    );
    expect(fake.calls).toEqual([]);
  });

  it.each(
    FEEDS,
  )('%s: delivers to a bound consumer with function, namespace, metadata and Void action', async (typeId) => {
    const { fake, feedOf } = setup();
    await fake.bindFeed(typeId, 's1', {
      function_id: 'acp::__on_event::c1',
      namespace: 'team-a',
      metadata: { from: 'binding' },
    });
    const event = { type: 'message_complete', message: { role: 'assistant', content: [] } };
    await expect(feedOf(typeId).emit('s1', event)).resolves.toBe(1);

    expect(fake.calls).toHaveLength(1);
    const [call] = fake.calls;
    expect(call.function_id).toBe('acp::__on_event::c1');
    expect(call.namespace).toBe('team-a');
    expect(call.metadata).toEqual({ from: 'binding' });
    expect(call.action).toEqual({ type: 'void' });
    const payload = call.payload as Record<string, unknown>;
    expect(Object.keys(payload).sort()).toEqual(
      ['epoch', 'event', 'event_id', 'seq', 'session_id', 'source'].sort(),
    );
    expect(payload).toMatchObject({ session_id: 's1', seq: 0, source: SOURCE, event });
    expect(typeof payload.epoch).toBe('string');
  });

  it('config metadata wins over binding metadata; both absent omits metadata', async () => {
    const { fake, feeds } = setup();
    await fake.bindFeed(AGENT_EVENT_TRIGGER, 's1', {
      function_id: 'a::config',
      config: { session_id: 's1', metadata: { from: 'config' } },
      metadata: { from: 'binding' },
    });
    await fake.bindFeed(AGENT_EVENT_TRIGGER, 's1', { function_id: 'a::none' });
    await feeds.agent.emit('s1', { type: 'turn_end' });
    const byFn = new Map(fake.calls.map((c) => [c.function_id, c]));
    expect(byFn.get('a::config')?.metadata).toEqual({ from: 'config' });
    expect(byFn.get('a::none')).not.toHaveProperty('metadata');
    expect(byFn.get('a::none')).not.toHaveProperty('namespace');
  });

  it.each(FEEDS)('%s: a binding for session A never receives session B', async (typeId) => {
    const { fake, feedOf } = setup();
    await fake.bindFeed(typeId, 'A', { function_id: 'c::a' });
    await fake.bindFeed(typeId, 'B', { function_id: 'c::b' });
    await feedOf(typeId).emit('B', { n: 1 });
    await feedOf(typeId).emit('B', { n: 2 });
    await feedOf(typeId).emit('A', { n: 3 });
    expect(fake.calls.filter((c) => c.function_id === 'c::a').map((c) => c.payload.event)).toEqual([
      { n: 3 },
    ]);
    expect(fake.calls.filter((c) => c.function_id === 'c::b').map((c) => c.payload.event)).toEqual([
      { n: 1 },
      { n: 2 },
    ]);
  });

  it('makes no trigger call at all when nobody is bound to the session', async () => {
    const { fake, feeds } = setup();
    await fake.bindFeed(AGENT_EVENT_TRIGGER, 'other');
    await expect(feeds.agent.emit('s1', { type: 'turn_end' })).resolves.toBe(0);
    await expect(feeds.raw.emit('s1', { type: 'result' })).resolves.toBe(0);
    expect(fake.calls).toEqual([]);
  });

  it.each(FEEDS)('%s: stops delivering after unbind', async (typeId) => {
    const { fake, feedOf } = setup();
    const id = await fake.bindFeed(typeId, 's1');
    await feedOf(typeId).emit('s1', { n: 1 });
    await fake.unbindFeed(typeId, id);
    expect(feedOf(typeId).bindingCount()).toBe(0);
    await feedOf(typeId).emit('s1', { n: 2 });
    expect(fake.feedFrames(typeId).map((f) => f.event)).toEqual([{ n: 1 }]);
  });

  it.each([
    ['missing session_id', {}],
    ['empty session_id', { session_id: '' }],
    ['non-string session_id', { session_id: 7 }],
    ['over-long session_id', { session_id: 'x'.repeat(513) }],
    ['unknown key', { session_id: 's1', group_id: 's1' }],
    ['array metadata', { session_id: 's1', metadata: [] }],
    ['string metadata', { session_id: 's1', metadata: 'm' }],
    ['not an object', 's1'],
    ['null', null],
  ])('rejects an invalid binding config: %s', async (_label, config) => {
    const { fake, feeds } = setup();
    for (const typeId of FEEDS) {
      await expect(fake.bindFeed(typeId, 's1', { config })).rejects.toThrow(typeId);
    }
    expect(feeds.agent.bindingCount()).toBe(0);
    expect(feeds.raw.bindingCount()).toBe(0);
  });

  it('accepts a 512-char session id and an object metadata', () => {
    expect(
      validateFeedConfig(AGENT_EVENT_TRIGGER, { session_id: 'x'.repeat(512), metadata: { a: 1 } }),
    ).toEqual({ session_id: 'x'.repeat(512), metadata: { a: 1 } });
  });

  it('caps bindings per trigger type; re-registering an existing id replaces it', async () => {
    const { fake, feeds } = setup();
    for (let i = 0; i < MAX_BINDINGS; i++) {
      await fake.bindFeed(AGENT_EVENT_TRIGGER, `s${i}`, { id: `b${i}` });
    }
    expect(feeds.agent.bindingCount()).toBe(MAX_BINDINGS);
    await expect(fake.bindFeed(AGENT_EVENT_TRIGGER, 'late', { id: 'b-new' })).rejects.toThrow(
      /binding limit/,
    );
    // Replacing an existing id is not a new binding.
    await fake.bindFeed(AGENT_EVENT_TRIGGER, 'moved', { id: 'b0', function_id: 'c::moved' });
    expect(feeds.agent.bindingCount()).toBe(MAX_BINDINGS);
    await feeds.agent.emit('moved', { n: 1 });
    await feeds.agent.emit('s0', { n: 2 });
    expect(fake.calls.map((c) => c.function_id)).toEqual(['c::moved']);
    // The cap is per trigger type: the raw feed still accepts bindings.
    await expect(fake.bindFeed(RAW_EVENT_TRIGGER, 'late')).resolves.toBeTypeOf('string');
  });

  it('numbers seq from 0 per (feed, session) with independent agent and raw counters', async () => {
    const { fake, feeds } = setup();
    for (const typeId of FEEDS) {
      await fake.bindFeed(typeId, 's1');
      await fake.bindFeed(typeId, 's2');
    }
    // Interleave the feeds the way a turn does (raw, then translated).
    await feeds.raw.emit('s1', { r: 0 });
    await feeds.agent.emit('s1', { a: 0 });
    await feeds.raw.emit('s1', { r: 1 });
    await feeds.raw.emit('s2', { r: 0 });
    await feeds.agent.emit('s1', { a: 1 });
    await feeds.agent.emit('s2', { a: 0 });
    await feeds.agent.emit('s1', { a: 2 });

    const seqs = (typeId: string, sid: string) =>
      fake
        .feedFrames(typeId)
        .filter((f) => f.session_id === sid)
        .map((f) => f.seq);
    expect(seqs(AGENT_EVENT_TRIGGER, 's1')).toEqual([0, 1, 2]);
    expect(seqs(AGENT_EVENT_TRIGGER, 's2')).toEqual([0]);
    expect(seqs(RAW_EVENT_TRIGGER, 's1')).toEqual([0, 1]);
    expect(seqs(RAW_EVENT_TRIGGER, 's2')).toEqual([0]);

    for (const typeId of FEEDS) {
      const frames = fake.feedFrames(typeId);
      const epochs = new Set(frames.map((f) => f.epoch));
      expect(epochs.size).toBe(1);
      for (const f of frames) {
        expect(f.event_id).toBe(`${f.session_id}-${f.epoch}-${String(f.seq).padStart(8, '0')}`);
        expect(String(f.event_id)).toMatch(/-\d{8}$/);
      }
    }
  });

  it('uses a stable event id when one is given', async () => {
    const { fake, feeds } = setup();
    await fake.bindFeed(AGENT_EVENT_TRIGGER, 's1');
    await feeds.agent.emit('s1', { n: 1 }, 'stable-1');
    expect(fake.feedFrames(AGENT_EVENT_TRIGGER)[0]).toMatchObject({ event_id: 'stable-1', seq: 0 });
  });

  it('swallows a failing delivery and keeps delivering to the other bindings', async () => {
    const { fake, feeds } = setup();
    await fake.bindFeed(AGENT_EVENT_TRIGGER, 's1', { function_id: 'c::gone' });
    await fake.bindFeed(AGENT_EVENT_TRIGGER, 's1', { function_id: 'c::ok' });
    const real = fake.iii.trigger.bind(fake.iii);
    (fake.iii as { trigger: unknown }).trigger = async (req: { function_id: string }) => {
      if (req.function_id === 'c::gone') throw new Error('function_not_found');
      return real(req as never);
    };
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    await expect(feeds.agent.emit('s1', { type: 'turn_end' })).resolves.toBe(1);
    expect(fake.calls.map((c) => c.function_id)).toEqual(['c::ok']);
    expect(warn).toHaveBeenCalledWith(expect.stringContaining('c::gone'));
    warn.mockRestore();
  });
});

const OBSERVABLES = [
  [AGENT_EVENT_TRIGGER, AGENT_EVENT_OBSERVABLE],
  [RAW_EVENT_TRIGGER, RAW_EVENT_OBSERVABLE],
] as const;

describe('observable feed functions (iii 0.25)', () => {
  it('registers both on-* functions owner-delivered with FeedConfig args', () => {
    const { fake } = setup();
    expect(AGENT_EVENT_OBSERVABLE).toBe('claude::on-agent-event');
    expect(RAW_EVENT_OBSERVABLE).toBe('claude::on-raw-event');
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
    expect([...fake.triggerTypes.keys()].sort()).toEqual(
      [AGENT_EVENT_TRIGGER, RAW_EVENT_TRIGGER].sort(),
    );
  });

  it.each(OBSERVABLES)('%s: direct invocation of %s is refused with FORBIDDEN', async (_, fnId) => {
    const { fake } = setup();
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
  )('%s: an observer receives the frame a binding receives', async (typeId, fnId) => {
    const { fake, feedOf } = setup();
    await fake.bindFeed(typeId, 's1');
    const observer = await fake.observe(fnId, { session_id: 's1' });
    const elsewhere = await fake.observe(fnId, { session_id: 's2' });

    await expect(feedOf(typeId).emit('s1', { type: 'turn_end' })).resolves.toBe(2);

    const bound = fake.feedFrames(typeId);
    expect(bound).toHaveLength(1);
    expect(observer.received).toEqual(bound);
    expect(bound[0]).toMatchObject({
      session_id: 's1',
      seq: 0,
      source: SOURCE,
      event: { type: 'turn_end' },
    });
    expect(elsewhere.call).not.toHaveBeenCalled();
  });

  it('an observer alone gets frames with the shared seq, and none after removal', async () => {
    const { fake, feeds } = setup();
    const observer = await fake.observe(RAW_EVENT_OBSERVABLE, { session_id: 's1' });
    await feeds.raw.emit('s1', { n: 0 });
    await feeds.raw.emit('s1', { n: 1 });
    expect(observer.received.map((p) => (p as { seq: number }).seq)).toEqual([0, 1]);
    // The agent feed is its own: the raw observer sees none of it.
    await feeds.agent.emit('s1', { n: 9 });
    expect(observer.received).toHaveLength(2);
    expect(fake.calls).toEqual([]);

    await fake.unobserve(RAW_EVENT_OBSERVABLE, observer);
    await expect(feeds.raw.emit('s1', { n: 2 })).resolves.toBe(0);
    expect(observer.received).toHaveLength(2);
  });

  it('rejects observer args the trigger config would refuse', async () => {
    const { fake, feeds } = setup();
    for (const args of [{}, { session_id: '' }, { session_id: 's1', bogus: 1 }, null]) {
      await expect(fake.observe(AGENT_EVENT_OBSERVABLE, args)).rejects.toMatchObject({
        code: 'INVALID_ARGS',
      });
    }
    expect(feeds.agent.bindingCount()).toBe(0);
  });

  it('counts observers against the binding cap', async () => {
    const { fake, feeds } = setup();
    for (let i = 0; i < MAX_BINDINGS; i++) await fake.bindFeed(AGENT_EVENT_TRIGGER, 's1');
    await expect(fake.observe(AGENT_EVENT_OBSERVABLE, { session_id: 's1' })).rejects.toMatchObject({
      code: 'BINDING_LIMIT',
    });
    expect(feeds.agent.bindingCount()).toBe(MAX_BINDINGS);
  });

  it('swallows an observer that skips or fails and keeps delivering', async () => {
    const { fake, feeds } = setup();
    const skipped = await fake.observe(AGENT_EVENT_OBSERVABLE, { session_id: 's1' });
    skipped.call.mockRejectedValueOnce(
      Object.assign(new Error('skip'), { code: 'FILTERS_NOT_MATCHED' }),
    );
    const broken = await fake.observe(AGENT_EVENT_OBSERVABLE, { session_id: 's1' });
    broken.call.mockRejectedValueOnce(new Error('target gone'));
    const ok = await fake.observe(AGENT_EVENT_OBSERVABLE, { session_id: 's1' });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    await expect(feeds.agent.emit('s1', { type: 'turn_end' })).resolves.toBe(1);
    expect(ok.received).toHaveLength(1);
    expect(warn).toHaveBeenCalledTimes(1);
    expect(warn).toHaveBeenCalledWith(expect.stringContaining(broken.target));
    warn.mockRestore();
  });
});
