import { describe, expect, it } from 'vitest';
import {
  DEFAULT_CONFIG,
  buildPublishEnvelope,
  buildResponse,
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

  it('includes publish:{ok:true} and no publish_failed when publish succeeds', async () => {
    let listCount = 0;
    const fakeSdk = {
      trigger: async (req: { function_id: string }) => {
        if (req.function_id === 'iii::durable::publish') return null;
        if (req.function_id === 'stream::list') {
          listCount++;
          return listCount < 2 ? { items: [] } : { items: [{ data: { block: false } }] };
        }
        return null;
      },
    } as unknown as Parameters<typeof execute>[0];

    const out = (await execute(
      fakeSdk,
      { ...DEFAULT_CONFIG, poll_interval_ms: 1 },
      {
        topic: 'agent::before_function_call',
        payload: {},
        merge_rule: 'first_block_wins',
        timeout_ms: 200,
        quiescence_ms: 50,
      },
    )) as Record<string, unknown>;

    expect(out.publish).toEqual({ ok: true });
    expect(out).not.toHaveProperty('publish_failed');
  });

  it('includes publish:{ok:false,error} and publish_failed:true when publish throws', async () => {
    const fakeSdk = {
      trigger: async (req: { function_id: string }) => {
        if (req.function_id === 'iii::durable::publish') throw new Error('ws closed');
        if (req.function_id === 'stream::list') return { items: [] };
        return null;
      },
    } as unknown as Parameters<typeof execute>[0];

    const out = (await execute(
      fakeSdk,
      { ...DEFAULT_CONFIG, poll_interval_ms: 1 },
      {
        topic: 'agent::before_function_call',
        payload: {},
        merge_rule: 'first_block_wins',
        timeout_ms: 200,
        quiescence_ms: 50,
      },
    )) as Record<string, unknown>;

    const publish = out.publish as Record<string, unknown>;
    expect(publish.ok).toBe(false);
    expect(publish.error).toBe('ws closed');
    expect(out.publish_failed).toBe(true);
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
