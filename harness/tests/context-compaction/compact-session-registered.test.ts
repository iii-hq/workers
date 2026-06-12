/**
 * End-to-end test for `context-compaction::compact_session` invoked via the
 * REGISTERED handler (not handleSync directly). Mirrors the UI `/compact`
 * slash-command path: provider/model in payload, session-manager mock that
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
    if (fn === 'state::update') {
      const key = p.key as string;
      const ops = (p.ops ?? []) as Array<{ type: string; value?: unknown }>;
      const oldValue = stateStore.has(key) ? stateStore.get(key) : null;
      let newValue: unknown = oldValue;
      for (const op of ops) {
        if (op.type === 'set') newValue = op.value;
      }
      if (newValue === null || newValue === undefined) stateStore.delete(key);
      else stateStore.set(key, newValue);
      return { old_value: oldValue ?? null, new_value: newValue ?? null };
    }
    if (fn === 'session::messages') {
      return { messages: opts.sessionEntries };
    }
    if (fn === 'session::append') {
      return { entry_id: `appended-${Date.now()}`, parent_id: null, timestamp: Date.now() };
    }
    if (fn === 'session::update_message') {
      return { updated: true, revision: 1 };
    }
    if (fn === 'router::models::get') {
      return { model: { context_window: 200_000, max_output_tokens: 4_096 } };
    }
    if (fn === 'router::complete') {
      const summary = opts.summaryText ?? 'summary text here';
      return {
        message: {
          role: 'assistant',
          content: [{ type: 'text', text: summary }],
          stop_reason: 'end',
          model: 'fake-model',
          provider: 'anthropic',
          timestamp: Date.now(),
        },
      };
    }
    return null;
  });

  const registerFunction = vi.fn((id: string, h: Handler) => {
    handlers.set(id, h);
  });

  const iii = {
    trigger,
    registerFunction,
    registerTrigger: vi.fn(),
    publish: vi.fn(),
    subscribe: vi.fn(),
    enqueue: vi.fn(),
  } as unknown as ISdk;

  return { iii, handlers, stateStore };
}

describe('context-compaction::compact_session via registered handler', () => {
  it('returns ok with tokens_before > 0 and auto_continued=false for a multi-turn session', async () => {
    // 2 user + 2 assistant turns. compact_session does NOT extract a
    // replay target (the replay mechanism is for compact_now's overflow
    // pre-flight only), so all 4 entries feed into selection. The summary
    // covers turn 1, turn 2 stays as tail.
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
    // /compact is user-initiated; no in-flight turn to auto-continue.
    expect(result.auto_continued).toBe(false);
    expect(typeof result.tokens_before).toBe('number');
    expect(result.tokens_before).toBeGreaterThan(0);
    // Surface every field so we can spot wire-format weirdness.
    expect(Object.keys(result).sort()).toEqual(
      ['auto_continued', 'status', 'summary_text', 'tail_start_id', 'tokens_before'].sort(),
    );
    // summary_text MUST be present and non-empty so the UI marker can ship
    // it as the next-turn <conversation-summary> block.
    expect(typeof result.summary_text).toBe('string');
    expect((result.summary_text ?? '').length).toBeGreaterThan(0);
  });

  it('summarises a session with only one user message and nothing else', async () => {
    // Before the fix this returned 'empty' because compact_session
    // extracted u1 as the replay target, leaving truncatedMessages = [].
    // After the fix the single user message is summarised normally.
    const entries = [userEntry('only-user', 'just one message')];
    const { iii, handlers } = buildSdk({
      sessionEntries: entries,
      summaryText: 'summary of the lone user message',
    });
    await register(iii);
    const handler = handlers.get('context-compaction::compact_session');

    const result = (await handler?.({
      session_id: 'one-msg-session',
      model: { id: 'fake-model', providerID: 'anthropic' },
    })) as { status: string; tokens_before?: number; auto_continued?: boolean };

    expect(result.status).toBe('ok');
    expect(result.tokens_before).toBeGreaterThan(0);
    expect(result.auto_continued).toBe(false);
  });

  it('summarises a single-user-turn session with many subsequent entries', async () => {
    // Regression: a long-running task with one user message + lots of
    // assistant/tool output ends up at ~30k+ tokens. Before the fix,
    // compact_session always extracted the last user message via
    // extractReplayTarget; with only one user message at idx=0,
    // truncatedMessages was [] and summarize returned 'empty'. The user
    // saw "compact: session is too small to summarise" despite a 30%
    // full context window.
    //
    // After the fix, compact_session does NOT extract a replay target
    // (that mechanism is for the compact_now overflow path), so the full
    // entry list is summarised normally.
    const longText = 'x'.repeat(8_000);
    const entries = [
      userEntry('u1', 'the only user question — start a long task'),
      asstEntry('a1', `${longText} first assistant chunk`),
      asstEntry('a2', `${longText} second assistant chunk`),
      asstEntry('a3', `${longText} third assistant chunk`),
      asstEntry('a4', `${longText} fourth assistant chunk`),
    ];

    const { iii, handlers } = buildSdk({
      sessionEntries: entries,
      summaryText: 'summary of the long task',
    });

    await register(iii);
    const handler = handlers.get('context-compaction::compact_session');

    const result = (await handler?.({
      session_id: 'one-turn-large-session',
      model: { id: 'fake-model', providerID: 'anthropic' },
    })) as {
      status: string;
      tokens_before?: number;
      auto_continued?: boolean;
      summary_text?: string;
    };

    expect(result.status).toBe('ok');
    expect(result.tokens_before).toBeGreaterThan(0);
    // /compact (user-initiated) does NOT auto-continue. Replay is only
    // appropriate for the orchestrator's overflow pre-flight.
    expect(result.auto_continued).toBe(false);
  });
});
