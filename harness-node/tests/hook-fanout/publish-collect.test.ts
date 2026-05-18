import { describe, expect, it } from 'vitest';
import {
  DEFAULT_CONFIG,
  buildPublishEnvelope,
  execute,
} from '../../src/hook-fanout/publish-collect.js';
import { HOOK_REPLY_STREAM } from '../../src/hook-fanout/types.js';

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
  it('returns merged first_block_wins reply when one subscriber blocks', async () => {
    let publishedCount = 0;
    let listCount = 0;
    const fakeSdk = {
      trigger: async (req: { function_id: string }) => {
        if (req.function_id === 'iii::durable::publish') {
          publishedCount++;
          return null;
        }
        if (req.function_id === 'stream::list') {
          listCount++;
          // First poll is empty, second returns a denial.
          if (listCount < 2) return { items: [] };
          return {
            items: [
              {
                data: { block: true, reason: 'no', denial: { status: 'denied' } },
              },
            ],
          };
        }
        return null;
      },
    } as unknown as Parameters<typeof execute>[0];

    const out = await execute(
      fakeSdk,
      { ...DEFAULT_CONFIG, poll_interval_ms: 1 },
      {
        topic: 'agent::before_function_call',
        payload: {},
        merge_rule: 'first_block_wins',
        timeout_ms: 1000,
        quiescence_ms: 50,
      },
    );

    expect(publishedCount).toBe(1);
    expect(out.replies.length).toBe(1);
    expect(out.merged).toMatchObject({ block: true });
  });

  it('throws on unknown merge_rule', async () => {
    const noop = {
      trigger: async () => null,
    } as unknown as Parameters<typeof execute>[0];
    await expect(
      execute(noop, DEFAULT_CONFIG, {
        topic: 't',
        payload: {},
        merge_rule: 'weird',
      }),
    ).rejects.toThrow(/unknown merge_rule/);
  });

  it('throws on missing topic', async () => {
    const noop = {
      trigger: async () => null,
    } as unknown as Parameters<typeof execute>[0];
    await expect(
      execute(noop, DEFAULT_CONFIG, { payload: {}, merge_rule: 'first_block_wins' }),
    ).rejects.toThrow(/topic/);
  });
});
