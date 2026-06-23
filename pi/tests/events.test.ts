import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from 'iii-sdk';
import { makeEmitter } from '../src/events.js';
import { fakeIii } from './_helpers/fake-iii.js';

describe('makeEmitter', () => {
  it('writes stream::set frames with the stream name, group, and event', async () => {
    const fake = fakeIii();
    const emit = makeEmitter(fake.iii, 'agent::events');
    await emit('s1', { type: 'agent_end', messages: [] });
    const frames = fake.streamFrames('agent::events');
    expect(frames).toHaveLength(1);
    expect(frames[0]).toMatchObject({
      stream_name: 'agent::events',
      group_id: 's1',
      data: { type: 'agent_end', messages: [] },
    });
  });

  it('assigns unique, monotonically increasing item_ids per session', async () => {
    const fake = fakeIii();
    const emit = makeEmitter(fake.iii, 'agent::events');
    await emit('s-seq', { a: 1 });
    await emit('s-seq', { a: 2 });
    await emit('other', { a: 3 });
    const ids = fake.streamFrames('agent::events').map((f) => String(f.item_id));
    expect(new Set(ids).size).toBe(3);
    const [first, second] = ids;
    expect(first < second).toBe(true);
    expect(first.startsWith('s-seq-')).toBe(true);
  });

  it('swallows stream::set failures instead of failing the turn', async () => {
    const iii = {
      trigger: vi.fn(async () => {
        throw new Error('stream adapter missing');
      }),
    } as unknown as ISdk;
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const emit = makeEmitter(iii, 'agent::events');
    await expect(emit('s1', { type: 'turn_end' })).resolves.toBeUndefined();
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });
});
