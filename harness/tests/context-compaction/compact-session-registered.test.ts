/**
 * `context-compaction::compact_session` — the thin `/compact` wrapper over the
 * context-manager worker. Exercises the registered handler end to end:
 * session-manager + context-manager are faked, and we assert the wrapper maps
 * the `context::compact` union onto the console's `CompactResult` wire shape and
 * persists the round trip as an additive compaction bookkeeping entry (the
 * transcript is never rewritten).
 */

import { describe, expect, it, vi } from 'vitest';
import { register } from '../../src/context-compaction/register.js';
import type { ISdk } from '../../src/runtime/iii.js';
import type { AgentMessage } from '../../src/types/agent-message.js';
import { FakeContextManager, FakeSessionManager } from '../_helpers/fakeSessionManager.js';

type Handler = (payload: unknown) => Promise<unknown> | unknown;

function buildIii(): {
  iii: ISdk;
  handlers: Map<string, Handler>;
  sessions: FakeSessionManager;
  context: FakeContextManager;
} {
  const handlers = new Map<string, Handler>();
  const sessions = new FakeSessionManager();
  const context = new FakeContextManager();
  const iii = {
    trigger: vi.fn(async (req: { function_id: string; payload?: unknown }) => {
      const s = await sessions.handle(req.function_id, req.payload);
      if (s !== undefined) return s;
      const c = await context.handle(req.function_id, req.payload);
      if (c !== undefined) return c;
      return null;
    }),
    registerFunction: vi.fn((id: string, h: Handler) => {
      handlers.set(id, h);
    }),
    registerTrigger: vi.fn(),
    publish: vi.fn(),
    subscribe: vi.fn(),
    enqueue: vi.fn(),
  } as unknown as ISdk;
  return { iii, handlers, sessions, context };
}

function seed(sessions: FakeSessionManager): string[] {
  const msgs: AgentMessage[] = [
    { role: 'user', content: [{ type: 'text', text: 'q1' }], timestamp: 1 },
    {
      role: 'assistant',
      content: [{ type: 'text', text: 'a1' }],
      stop_reason: 'end',
      model: 'm',
      provider: 'p',
      timestamp: 2,
    },
    { role: 'user', content: [{ type: 'text', text: 'q2' }], timestamp: 3 },
  ];
  return sessions.seedMessages('s1', msgs, ['u1', 'a1', 'u2']);
}

describe('context-compaction::compact_session via registered handler', () => {
  it('maps context::compact ok onto CompactResult and persists the round trip', async () => {
    const { iii, handlers, sessions, context } = buildIii();
    seed(sessions);
    context.compact = () => ({
      status: 'ok',
      summary: 'SUMMARY',
      tail_start_index: 2,
      tokens_before: 123,
      tokens_after: 10,
      used_prior_summary: false,
    });

    await register(iii);
    const handler = handlers.get('context-compaction::compact_session');
    const result = (await handler?.({
      session_id: 's1',
      model: { id: 'm', providerID: 'p' },
    })) as Record<string, unknown>;

    // Console wire contract (unchanged): legacy `tail_start_id` key in the RPC.
    expect(result).toMatchObject({
      status: 'ok',
      tokens_before: 123,
      auto_continued: false,
      summary_text: 'SUMMARY',
      tail_start_id: 'u2',
    });
    expect(Object.keys(result).sort()).toEqual(
      ['auto_continued', 'status', 'summary_text', 'tail_start_id', 'tokens_before'].sort(),
    );

    // Persisted as an additive compaction custom entry using the spec field name.
    const appended = sessions
      .callsTo('session::append')
      .filter((p) => (p.custom as { custom_type?: string })?.custom_type === 'compaction');
    expect(appended).toHaveLength(1);
    const data = (appended[0]?.custom as { data: Record<string, unknown> }).data;
    expect(data.tail_start_entry_id).toBe('u2');
    expect(data.summary).toBe('SUMMARY');

    // The transcript is untouched: the 3 seeded messages are all still present.
    expect(sessions.messageEntries('s1')).toHaveLength(3);
  });

  it('anchors on the latest stored summary as previous_summary', async () => {
    const { iii, handlers, sessions, context } = buildIii();
    seed(sessions);
    sessions.seedCompaction('s1', { summary: 'PRIOR', tail_start_entry_id: 'u1' });
    context.compact = () => ({
      status: 'ok',
      summary: 'NEW',
      tail_start_index: null,
      tokens_before: 5,
      tokens_after: 5,
      used_prior_summary: true,
    });

    await register(iii);
    await handlers.get('context-compaction::compact_session')?.({
      session_id: 's1',
      model: { id: 'm', providerID: 'p' },
    });

    expect(context.callsTo('context::compact')[0]?.options).toMatchObject({
      lease_key: 's1',
      previous_summary: 'PRIOR',
    });
  });

  it('maps busy / empty / overflow statuses', async () => {
    for (const status of ['busy', 'empty'] as const) {
      const { iii, handlers, context } = buildIii();
      context.compact = () => ({ status });
      await register(iii);
      const r = await handlers.get('context-compaction::compact_session')?.({
        session_id: 's1',
        model: { id: 'm', providerID: 'p' },
      });
      expect(r).toEqual({ status });
    }

    const { iii, handlers, context } = buildIii();
    context.compact = () => ({ status: 'overflow' });
    await register(iii);
    const r = (await handlers.get('context-compaction::compact_session')?.({
      session_id: 's1',
      model: { id: 'm', providerID: 'p' },
    })) as Record<string, unknown>;
    expect(r.status).toBe('overflow');
    expect(typeof r.message).toBe('string');
  });

  it('returns an error result when no model is supplied', async () => {
    const { iii, handlers } = buildIii();
    await register(iii);
    const r = (await handlers.get('context-compaction::compact_session')?.({
      session_id: 's1',
    })) as Record<string, unknown>;
    expect(r.status).toBe('error');
  });

  it('throws when session_id is missing', async () => {
    const { iii, handlers } = buildIii();
    await register(iii);
    await expect(
      handlers.get('context-compaction::compact_session')?.({
        model: { id: 'm', providerID: 'p' },
      }),
    ).rejects.toThrow('session_id is required');
  });
});
