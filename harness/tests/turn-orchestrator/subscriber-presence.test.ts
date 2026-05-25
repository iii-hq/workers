import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import {
  hasDurableSubscriber,
  resetSubscriberCache,
} from '../../src/turn-orchestrator/subscriber-presence.js';

function fakeIii(
  triggers: unknown[],
  opts?: { onList?: () => void; throwOnList?: boolean },
) {
  const trigger = vi.fn(async (req: { function_id: string }) => {
    if (req.function_id === 'engine::triggers::list') {
      opts?.onList?.();
      if (opts?.throwOnList) throw new Error('engine unreachable');
      return { triggers };
    }
    throw new Error(`unexpected function_id: ${req.function_id}`);
  });
  return { iii: { trigger } as unknown as ISdk, trigger };
}

describe('hasDurableSubscriber', () => {
  beforeEach(() => resetSubscriberCache());

  it('returns true when a durable:subscriber trigger is bound to the topic', async () => {
    const { iii } = fakeIii([
      { trigger_type: 'durable:subscriber', config: { topic: 'agent::after_function_call' } },
    ]);
    expect(await hasDurableSubscriber(iii, 'agent::after_function_call')).toBe(true);
  });

  it('returns false when no trigger subscribes to the topic', async () => {
    const { iii } = fakeIii([
      { trigger_type: 'stream', config: { stream_name: 'agent::hook_reply' } },
      { trigger_type: 'durable:subscriber', config: { topic: 'some::other::topic' } },
    ]);
    expect(await hasDurableSubscriber(iii, 'agent::after_function_call')).toBe(false);
  });

  it('caches the result within the TTL (one engine query for repeated calls)', async () => {
    let listCalls = 0;
    const { iii } = fakeIii([], {
      onList: () => {
        listCalls++;
      },
    });
    await hasDurableSubscriber(iii, 'agent::after_function_call', 1000);
    await hasDurableSubscriber(iii, 'agent::after_function_call', 2000);
    expect(listCalls).toBe(1);
  });

  it('re-queries after the TTL expires', async () => {
    let listCalls = 0;
    const { iii } = fakeIii([], {
      onList: () => {
        listCalls++;
      },
    });
    await hasDurableSubscriber(iii, 'agent::after_function_call', 1000);
    await hasDurableSubscriber(iii, 'agent::after_function_call', 1000 + 30_001);
    expect(listCalls).toBe(2);
  });

  it('fails safe (returns true) when the engine query throws', async () => {
    const { iii } = fakeIii([], { throwOnList: true });
    expect(await hasDurableSubscriber(iii, 'agent::after_function_call')).toBe(true);
  });
});
