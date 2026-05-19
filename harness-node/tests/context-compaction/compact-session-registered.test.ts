/**
 * End-to-end test for `context-compaction::compact_session` invoked via the
 * REGISTERED handler (not handleSync directly). Mirrors the UI `/compact`
 * slash-command path: provider/model in payload, session-tree mock that
 * returns a realistic transcript.
 *
 * The bug we're chasing: the UI reports `compacted — 0 tokens summarised
 * (continued)`. `(continued)` proves a replay happened, so summarize was
 * NOT in the 'empty' branch. So tokens_before should be > 0. This test
 * asserts that.
 */

import { describe, expect, it, vi } from 'vitest';
import { register } from '../../src/context-compaction/register.js';
import type { ISdk } from '../../src/runtime/iii.js';
import type { AgentMessage } from '../../src/types/agent-message.js';

type Handler = (payload: unknown) => Promise<unknown>;

function userEntry(id: string, text: string): { entry_id: string; message: AgentMessage } {
  return {
    entry_id: id,
    message: { role: 'user', content: [{ type: 'text', text }], timestamp: 0 },
  };
}

function asstEntry(id: string, text: string): { entry_id: string; message: AgentMessage } {
  return {
    entry_id: id,
    message: {
      role: 'assistant',
      content: [{ type: 'text', text }],
      stop_reason: 'end',
      error_message: null,
      error_kind: null,
      usage: null,
      model: 'fake-model',
      provider: 'anthropic',
      timestamp: 0,
    },
  };
}

function buildSdk(opts: {
  sessionEntries: Array<{ entry_id: string; message: AgentMessage }>;
  summaryText?: string;
}): {
  iii: ISdk;
  handlers: Map<string, Handler>;
  stateStore: Map<string, unknown>;
} {
  const handlers = new Map<string, Handler>();
  const stateStore = new Map<string, unknown>();

  let channelCb: ((raw: string) => void) | null = null;
  const channel = {
    reader: {
      onMessage(cb: (raw: string) => void) {
        channelCb = cb;
      },
      stream: { resume: () => {} },
    },
    writerRef: 'mock-writer-ref',
  };

  const trigger = vi.fn(async (req: { function_id: string; payload?: unknown }) => {
    const fn = req.function_id;
    const payload = req.payload;
    const p = (payload ?? {}) as Record<string, unknown>;

    if (fn === 'state::get') {
      const v = stateStore.get(p.key as string);
      return v !== undefined ? v : null;
    }
    if (fn === 'state::set') {
      if (p.value === null || p.value === undefined) stateStore.delete(p.key as string);
      else stateStore.set(p.key as string, p.value);
      return { ok: true };
    }
    if (fn === 'session-tree::messages') {
      return { messages: opts.sessionEntries };
    }
    if (fn === 'session-tree::compactions') {
      return { entries: [] };
    }
    if (fn === 'session-tree::compact') {
      return { entry_id: `compaction-${Date.now()}` };
    }
    if (fn === 'session-tree::append') {
      return { entry_id: `appended-${Date.now()}` };
    }
    if (fn === 'session-tree::append_synthetic') {
      return { entry_id: `synthetic-${Date.now()}` };
    }
    if (fn === 'session-tree::update_parts') {
      return { updated: 0 };
    }
    if (fn === 'models::get') {
      return { context_window: 200_000, max_output_tokens: 4_096 };
    }
    if (fn.startsWith('provider::')) {
      const summary = opts.summaryText ?? 'summary text here';
      const event = JSON.stringify({
        type: 'done',
        message: {
          role: 'assistant',
          content: [{ type: 'text', text: summary }],
          stop_reason: 'end',
          model: 'fake-model',
          provider: 'anthropic',
          timestamp: Date.now(),
        },
      });
      if (channelCb) channelCb(event);
      return undefined;
    }
    return null;
  });

  const createChannel = vi.fn(async () => channel);
  const registerFunction = vi.fn((id: string, h: Handler) => {
    handlers.set(id, h);
  });

  const iii = {
    trigger,
    createChannel,
    registerFunction,
    registerTrigger: vi.fn(),
    publish: vi.fn(),
    subscribe: vi.fn(),
    enqueue: vi.fn(),
  } as unknown as ISdk;

  return { iii, handlers, stateStore };
}

describe('context-compaction::compact_session via registered handler', () => {
  it('returns ok with tokens_before > 0 and auto_continued=true for a multi-turn session', async () => {
    // 2 user + 2 assistant turns. compact_session picks u2 as the
    // replay target → truncatedMessages = [u1, a1] → summarise.
    const entries = [
      userEntry('e1', 'first question'),
      asstEntry('e2', 'first answer'),
      userEntry('e3', 'second question — the most recent user message'),
      asstEntry('e4', 'second answer'),
    ];

    const { iii, handlers } = buildSdk({
      sessionEntries: entries,
      summaryText: 'summary covering turns 1-2',
    });

    await register(iii);

    const handler = handlers.get('context-compaction::compact_session');
    expect(handler).toBeDefined();

    const result = (await handler?.({
      session_id: 'test-session',
      model: { id: 'fake-model', providerID: 'anthropic' },
    })) as {
      status: string;
      tokens_before?: number;
      auto_continued?: boolean;
      tail_start_id?: string | null;
      summary_text?: string;
    };

    expect(result.status).toBe('ok');
    expect(result.auto_continued).toBe(true);
    // Bug repro target: this MUST be > 0 for any non-empty head.
    expect(typeof result.tokens_before).toBe('number');
    expect(result.tokens_before).toBeGreaterThan(0);
    // Surface every field so we can spot wire-format weirdness.
    expect(Object.keys(result).sort()).toEqual(
      ['auto_continued', 'status', 'summary_text', 'tail_start_id', 'tokens_before'].sort(),
    );
    // summary_text MUST be present and non-empty so the UI marker can ship
    // it as the next-turn <conversation-summary> block (Option A fix).
    expect(typeof result.summary_text).toBe('string');
    expect((result.summary_text ?? '').length).toBeGreaterThan(0);
  });

  it('returns empty when last user message is the FIRST entry (truncated is empty)', async () => {
    // Session with ONLY one user message that matches last_user_message_id.
    // - extractReplayTarget: idx=0 → replay=u1, truncatedMessages=[]
    // - summarize sees []: returns 'empty'
    // - handler-sync: returns { status: 'empty' }
    // This proves the suspected UI scenario actually returns 'empty', NOT 'ok'.
    // If a real call sees 'ok' with tokens=0, the bug is elsewhere.
    const entries = [userEntry('only-user', 'just one message')];
    const { iii, handlers } = buildSdk({ sessionEntries: entries });
    await register(iii);
    const handler = handlers.get('context-compaction::compact_session');

    const result = (await handler?.({
      session_id: 'one-msg-session',
      model: { id: 'fake-model', providerID: 'anthropic' },
    })) as { status: string; tokens_before?: number; auto_continued?: boolean };

    expect(result.status).toBe('empty');
    // 'empty' shape: just { status }. No tokens_before, no auto_continued.
    expect(result.tokens_before).toBeUndefined();
    expect(result.auto_continued).toBeUndefined();
  });
});
