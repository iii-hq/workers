/**
 * Unit tests for the compaction round-trip helpers (src/runtime/compaction.ts):
 * window loading + summary anchoring, the best-effort assemble client with its
 * raw fallback, and additive round-trip persistence.
 */

import { describe, expect, it, vi } from 'vitest';
import {
  assembleContext,
  loadAssembleWindow,
  persistCompactionRoundTrip,
} from '../../src/runtime/compaction.js';
import type { ISdk } from '../../src/runtime/iii.js';
import type { AgentMessage } from '../../src/types/agent-message.js';
import type { MessageWithEntryId } from '../../src/runtime/session.js';
import { FakeContextManager, FakeSessionManager } from '../_helpers/fakeSessionManager.js';

function buildIii(): { iii: ISdk; sessions: FakeSessionManager; context: FakeContextManager } {
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
    registerFunction: vi.fn(),
    registerTrigger: vi.fn(),
  } as unknown as ISdk;
  return { iii, sessions, context };
}

const user = (text: string): AgentMessage => ({
  role: 'user',
  content: [{ type: 'text', text }],
  timestamp: 1,
});
const asst = (text: string): AgentMessage => ({
  role: 'assistant',
  content: text ? [{ type: 'text', text }] : [],
  stop_reason: 'end',
  model: 'm',
  provider: 'p',
  timestamp: 2,
});
const win = (entry_id: string, message: AgentMessage): MessageWithEntryId => ({
  entry_id,
  message,
});

describe('loadAssembleWindow', () => {
  it('returns the full window and no summary when there is no compaction', async () => {
    const { iii, sessions } = buildIii();
    sessions.seedMessages('s1', [user('a'), asst('b')], ['u1', 'a1']);

    const { window, previousSummary } = await loadAssembleWindow(iii, 's1');

    expect(window.map((w) => w.entry_id)).toEqual(['u1', 'a1']);
    expect(previousSummary).toBeUndefined();
  });

  it('slices from tail_start_entry_id and surfaces the anchored summary', async () => {
    const { iii, sessions } = buildIii();
    sessions.seedMessages(
      's1',
      [user('a'), asst('b'), user('c'), asst('d')],
      ['u1', 'a1', 'u2', 'a2'],
    );
    sessions.seedCompaction('s1', { summary: 'PRIOR', tail_start_entry_id: 'u2' });

    const { window, previousSummary } = await loadAssembleWindow(iii, 's1');

    expect(window.map((w) => w.entry_id)).toEqual(['u2', 'a2']);
    expect(previousSummary).toBe('PRIOR');
  });

  it('reads the legacy tail_start_id key for pre-migration sessions', async () => {
    const { iii, sessions } = buildIii();
    sessions.seedMessages('s1', [user('a'), asst('b'), user('c')], ['u1', 'a1', 'u2']);
    sessions.seedCompaction('s1', {
      summary: 'PRIOR',
      tail_start_entry_id: 'u2',
      legacyTailField: true,
    });

    const { window, previousSummary } = await loadAssembleWindow(iii, 's1');

    expect(window.map((w) => w.entry_id)).toEqual(['u2']);
    expect(previousSummary).toBe('PRIOR');
  });

  it('keeps only messages after the compaction entry when the tail is null', async () => {
    const { iii, sessions } = buildIii();
    sessions.seedMessages('s1', [user('a'), asst('b')], ['u1', 'a1']);
    sessions.seedCompaction('s1', { summary: 'PRIOR', tail_start_entry_id: null });
    sessions.seedMessages('s1', [user('c')], ['u2']);

    const { window } = await loadAssembleWindow(iii, 's1');

    expect(window.map((w) => w.entry_id)).toEqual(['u2']);
  });

  it('drops excluded entries and empty assistant placeholders', async () => {
    const { iii, sessions } = buildIii();
    sessions.seedMessages('s1', [user('a'), asst(''), asst('b')], ['u1', 'placeholder', 'a1']);

    const { window } = await loadAssembleWindow(iii, 's1', { excludeEntryIds: ['a1'] });

    // 'a1' excluded, the empty-content assistant ('placeholder') filtered out.
    expect(window.map((w) => w.entry_id)).toEqual(['u1']);
  });
});

describe('assembleContext', () => {
  it('sends inline limits + lease_key + previous_summary and returns the worker response', async () => {
    const { iii, context } = buildIii();
    context.assemble = () => ({
      system_prompt: 'BASE\n\n# Conversation summary\nX',
      messages: [user('hi')],
      applied: {
        pruned: false,
        pruned_tokens: 0,
        compacted: true,
        summary: 'X',
        tail_start_index: 0,
        tokens_before: 9,
      },
    });

    const res = await assembleContext(iii, {
      session_id: 's1',
      window: [win('u1', user('hi'))],
      baseSystemPrompt: 'BASE',
      modelId: 'm',
      provider: 'p',
      modelMeta: { id: 'm', provider: 'p', context_window: 1000, max_output_tokens: 100 },
      thinkingLevel: 'high',
      previousSummary: 'PRIOR',
    });

    expect(res.system_prompt).toContain('# Conversation summary');
    expect(res.applied.compacted).toBe(true);

    const req = context.callsTo('context::assemble')[0];
    expect(req?.options).toMatchObject({
      lease_key: 's1',
      previous_summary: 'PRIOR',
      thinking_level: 'high',
    });
    expect(req?.model).toMatchObject({
      id: 'm',
      provider: 'p',
      limits: { context_window: 1000, max_output_tokens: 100 },
    });
  });

  it('falls back to the raw window when context::assemble fails', async () => {
    const { iii, context } = buildIii();
    context.assemble = () => {
      throw new Error('worker unavailable');
    };
    const window = [win('u1', user('hi'))];

    const res = await assembleContext(iii, {
      session_id: 's1',
      window,
      baseSystemPrompt: 'BASE',
      modelId: 'm',
    });

    expect(res.system_prompt).toBe('BASE');
    expect(res.messages).toEqual([window[0]?.message]);
    expect(res.applied).toEqual({ pruned: false, pruned_tokens: 0, compacted: false });
  });
});

describe('persistCompactionRoundTrip', () => {
  it('writes an additive compaction entry mapping tail_start_index to an entry id', async () => {
    const { iii, sessions } = buildIii();
    const window = [win('u1', user('a')), win('a1', asst('b')), win('u2', user('c'))];

    const entryId = await persistCompactionRoundTrip(
      iii,
      's1',
      {
        pruned: false,
        pruned_tokens: 0,
        compacted: true,
        summary: 'S',
        tail_start_index: 2,
        tokens_before: 50,
      },
      window,
    );

    expect(typeof entryId).toBe('string');
    const data = (
      sessions
        .callsTo('session::append')
        .find((p) => (p.custom as { custom_type?: string })?.custom_type === 'compaction')
        ?.custom as { data: Record<string, unknown> }
    ).data;
    expect(data.tail_start_entry_id).toBe('u2');
    expect(data.summary).toBe('S');
    expect(data.tokens_before).toBe(50);
  });

  it('writes a null tail boundary when everything was summarised', async () => {
    const { iii, sessions } = buildIii();
    const window = [win('u1', user('a'))];

    await persistCompactionRoundTrip(
      iii,
      's1',
      {
        pruned: false,
        pruned_tokens: 0,
        compacted: true,
        summary: 'S',
        tail_start_index: null,
        tokens_before: 1,
      },
      window,
    );

    const data = (
      sessions
        .callsTo('session::append')
        .find((p) => (p.custom as { custom_type?: string })?.custom_type === 'compaction')
        ?.custom as { data: Record<string, unknown> }
    ).data;
    expect(data.tail_start_entry_id).toBeNull();
  });

  it('is a no-op (returns null, writes nothing) when not compacted', async () => {
    const { iii, sessions } = buildIii();
    const entryId = await persistCompactionRoundTrip(
      iii,
      's1',
      { pruned: false, pruned_tokens: 0, compacted: false },
      [win('u1', user('a'))],
    );

    expect(entryId).toBeNull();
    expect(sessions.callsTo('session::append')).toHaveLength(0);
  });
});
