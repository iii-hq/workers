import { describe, expect, it } from 'vitest';
import {
  DEFAULT_CONFIG,
  buildPublishEnvelope,
  buildResponse,
  execute,
  handleStreamReply,
} from '../../src/hook-fanout/publish-collect.js';
import { HOOK_REPLY_STREAM } from '../../src/hook-fanout/types.js';

type FakeSdk = {
  iii: Parameters<typeof execute>[0];
  triggerCalls: Array<{ function_id: string; payload: unknown }>;
};

function makeFakeSdk(opts: {
  onPublish?: (envelope: unknown) => void | Promise<void>;
  /** Auto-deliver these replies as soon as the publish trigger is invoked. */
  autoReplies?: unknown[];
}): FakeSdk {
  const triggerCalls: Array<{ function_id: string; payload: unknown }> = [];
  const iii = {
    trigger: async (req: { function_id: string; payload: unknown }) => {
      triggerCalls.push(req);
      if (req.function_id === 'iii::durable::publish') {
        const envelope = req.payload as Record<string, unknown>;
        if (opts.onPublish) await opts.onPublish(envelope);
        if (opts.autoReplies) {
          const data = envelope.data as Record<string, unknown>;
          const event_id = data.event_id as string;
          for (const reply of opts.autoReplies) {
            // Simulate the stream trigger firing for each subscriber's
            // `stream::set` write on `agent::hook_reply`.
            await handleStreamReply({ group_id: event_id, data: reply });
          }
        }
        return null;
      }
      return null;
    },
  } as unknown as Parameters<typeof execute>[0];
  return { iii, triggerCalls };
}

describe('buildPublishEnvelope', () => {
  it('embeds event_id and reply_stream alongside the payload', () => {
    const env = buildPublishEnvelope('agent::before_function_call', 'evt-1', {
      function_call: { id: 'tc-1' },
    });
    expect(env.topic).toBe('agent::before_function_call');
    const data = env.data as Record<string, unknown>;
    expect(data.event_id).toBe('evt-1');
    expect(data.reply_stream).toBe(HOOK_REPLY_STREAM);
    const payload = data.payload as Record<string, unknown>;
    const fc = payload.function_call as Record<string, unknown>;
    expect(fc.id).toBe('tc-1');
  });
});

describe('publish-collect execute', () => {
  it('returns merged first_block_wins reply when a subscriber delivers a denial', async () => {
    const denial = { block: true, reason: 'no', denial: { status: 'denied' } };
    const fake = makeFakeSdk({ autoReplies: [denial] });

    const out = (await execute(
      fake.iii,
      { ...DEFAULT_CONFIG, quiescence_ms: 10 },
      {
        topic: 'agent::before_function_call',
        payload: {},
        merge_rule: 'first_block_wins',
        timeout_ms: 1_000,
        quiescence_ms: 10,
      },
    )) as Record<string, unknown>;

    const publishCount = fake.triggerCalls.filter(
      (c) => c.function_id === 'iii::durable::publish',
    ).length;
    expect(publishCount).toBe(1);
    expect((out.replies as unknown[]).length).toBe(1);
    expect(out.merged).toMatchObject({ block: true });
  });

  it('exits on expected_replies threshold without waiting for quiescence', async () => {
    const fake = makeFakeSdk({ autoReplies: [{ block: false }, { block: false }] });

    const started = Date.now();
    const out = (await execute(
      fake.iii,
      { ...DEFAULT_CONFIG, quiescence_ms: 10_000 },
      {
        topic: 'agent::before_function_call',
        payload: {},
        merge_rule: 'first_block_wins',
        timeout_ms: 10_000,
        expected_replies: 2,
        quiescence_ms: 10_000,
      },
    )) as Record<string, unknown>;
    const elapsed = Date.now() - started;

    expect((out.replies as unknown[]).length).toBe(2);
    // Should be near-instant: no polling, no quiescence wait.
    expect(elapsed).toBeLessThan(500);
  });

  it('ignores stream frames for unknown event_ids', async () => {
    // Pre-publish: the global handler should silently drop frames whose
    // group_id matches no pending Collector.
    await handleStreamReply({ group_id: 'nope', data: { block: true } });

    const fake = makeFakeSdk({ autoReplies: [{ block: false }] });
    const out = (await execute(
      fake.iii,
      { ...DEFAULT_CONFIG, quiescence_ms: 10 },
      {
        topic: 'agent::before_function_call',
        payload: {},
        merge_rule: 'first_block_wins',
        timeout_ms: 200,
        quiescence_ms: 10,
      },
    )) as Record<string, unknown>;

    // Only the intended reply lands.
    expect((out.replies as unknown[]).length).toBe(1);
    expect((out.replies as Array<Record<string, unknown>>)[0].block).toBe(false);
  });

  it('throws on unknown merge_rule', async () => {
    const fake = makeFakeSdk({});
    await expect(
      execute(fake.iii, DEFAULT_CONFIG, {
        topic: 't',
        payload: {},
        merge_rule: 'weird',
      }),
    ).rejects.toThrow(/unknown merge_rule/);
  });

  it('throws on missing topic', async () => {
    const fake = makeFakeSdk({});
    await expect(
      execute(fake.iii, DEFAULT_CONFIG, { payload: {}, merge_rule: 'first_block_wins' }),
    ).rejects.toThrow(/topic/);
  });

  it('includes publish:{ok:true} and no publish_failed when publish succeeds', async () => {
    const fake = makeFakeSdk({ autoReplies: [{ block: false }] });

    const out = (await execute(
      fake.iii,
      { ...DEFAULT_CONFIG, quiescence_ms: 10 },
      {
        topic: 'agent::before_function_call',
        payload: {},
        merge_rule: 'first_block_wins',
        timeout_ms: 200,
        quiescence_ms: 10,
      },
    )) as Record<string, unknown>;

    expect(out.publish).toEqual({ ok: true });
    expect(out).not.toHaveProperty('publish_failed');
  });

  it('includes publish:{ok:false,error} and publish_failed:true when publish throws', async () => {
    const fake = makeFakeSdk({
      onPublish: () => {
        throw new Error('ws closed');
      },
    });

    const out = (await execute(
      fake.iii,
      { ...DEFAULT_CONFIG, min_timeout_ms: 10 },
      {
        topic: 'agent::before_function_call',
        payload: {},
        merge_rule: 'first_block_wins',
        timeout_ms: 50,
        quiescence_ms: 10,
      },
    )) as Record<string, unknown>;

    const publish = out.publish as Record<string, unknown>;
    expect(publish.ok).toBe(false);
    expect(publish.error).toBe('ws closed');
    expect(out.publish_failed).toBe(true);
  });
});

describe('handleStreamReply frame shapes', () => {
  it('accepts both groupId and group_id', async () => {
    const fakeCamel = makeFakeSdk({});
    // Manually publish and deliver via camelCase
    const publishCalls: unknown[] = [];
    const iiiCamel = {
      trigger: async (req: { function_id: string; payload: unknown }) => {
        if (req.function_id === 'iii::durable::publish') {
          publishCalls.push(req.payload);
          const env = req.payload as Record<string, unknown>;
          const data = env.data as Record<string, unknown>;
          await handleStreamReply({ groupId: data.event_id, data: { block: false } });
        }
        return null;
      },
    } as unknown as Parameters<typeof execute>[0];
    void fakeCamel;

    const out = (await execute(
      iiiCamel,
      { ...DEFAULT_CONFIG, quiescence_ms: 10 },
      {
        topic: 't',
        payload: {},
        merge_rule: 'first_block_wins',
        timeout_ms: 200,
        quiescence_ms: 10,
      },
    )) as Record<string, unknown>;
    expect((out.replies as unknown[]).length).toBe(1);
  });

  it('unwraps event.data when present', async () => {
    const iiiSdk = {
      trigger: async (req: { function_id: string; payload: unknown }) => {
        if (req.function_id === 'iii::durable::publish') {
          const env = req.payload as Record<string, unknown>;
          const data = env.data as Record<string, unknown>;
          await handleStreamReply({
            group_id: data.event_id,
            event: { data: { block: true, reason: 'wrapped' } },
          });
        }
        return null;
      },
    } as unknown as Parameters<typeof execute>[0];

    const out = (await execute(
      iiiSdk,
      { ...DEFAULT_CONFIG, quiescence_ms: 10 },
      {
        topic: 't',
        payload: {},
        merge_rule: 'first_block_wins',
        timeout_ms: 200,
        quiescence_ms: 10,
      },
    )) as Record<string, unknown>;
    const replies = out.replies as Array<Record<string, unknown>>;
    expect(replies[0].block).toBe(true);
    expect(replies[0].reason).toBe('wrapped');
  });
});

describe('buildResponse', () => {
  it('marks publish ok on success and omits publish_failed', () => {
    const out = buildResponse('evt-1', [{ block: false }], { block: false }, false);
    expect(out.event_id).toBe('evt-1');
    expect(out.publish).toEqual({ ok: true });
    expect(out).not.toHaveProperty('publish_failed');
  });

  it('marks publish failed with error text and sets publish_failed:true', () => {
    const out = buildResponse('evt-2', [], { block: false }, true, 'ws closed');
    expect(out.publish).toEqual({ ok: false, error: 'ws closed' });
    expect(out.publish_failed).toBe(true);
  });

  it('marks publish failed without error text when none provided', () => {
    const out = buildResponse('evt-3', [], { block: false }, true);
    expect(out.publish).toEqual({ ok: false });
    expect(out.publish_failed).toBe(true);
  });
});
