import { describe, expect, it, vi } from 'vitest';
import { prune } from '../../src/context-compaction/prune.js';
import type { AgentMessage } from '../../src/types/agent-message.js';

function user(): AgentMessage {
  return { role: 'user', content: [{ type: 'text', text: 'q' }], timestamp: 0 };
}
function asst(): AgentMessage {
  return {
    role: 'assistant',
    content: [{ type: 'text', text: 'a' }],
    stop_reason: 'end',
    model: 'm',
    provider: 'p',
    timestamp: 0,
  };
}
function tool(fn: string, output: string): AgentMessage {
  return {
    role: 'function_result',
    function_call_id: 'fc',
    function_id: fn,
    content: [{ type: 'text', text: output }],
    details: {},
    is_error: false,
    timestamp: 0,
  };
}

function makeIii(entries: Array<{ entry_id: string; message: AgentMessage }>) {
  const updates: Array<{ entry_id: string }> = [];
  const iii = {
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      if (function_id === 'session-tree::messages') return { messages: entries };
      if (function_id === 'session-tree::update_parts') {
        const items = (payload as { items?: Array<{ entry_id: string }> }).items ?? [];
        for (const it of items) updates.push({ entry_id: it.entry_id });
        return { updated: items.length };
      }
      return null;
    }),
  };
  return { iii: iii as unknown as Parameters<typeof prune>[0], updates };
}

describe('prune', () => {
  it('returns pruned: 0 when nothing exceeds protectTokens', async () => {
    const entries = [
      { entry_id: 'e1', message: user() },
      { entry_id: 'e2', message: asst() },
      { entry_id: 'e3', message: tool('shell::fs::ls', 'small') },
      { entry_id: 'e4', message: user() },
      { entry_id: 'e5', message: asst() },
    ];
    const { iii, updates } = makeIii(entries);
    const res = await prune(iii, 'sid', {
      protectTokens: 40_000,
      minFree: 20_000,
      protectedTools: [],
    });
    expect(res.pruned_parts).toBe(0);
    expect(updates).toHaveLength(0);
  });

  it('prunes older tool outputs beyond protectTokens once minFree is hit', async () => {
    const big = 'x'.repeat(200_000);
    const entries: Array<{ entry_id: string; message: AgentMessage }> = [];
    for (let i = 0; i < 5; i++) {
      entries.push({ entry_id: `u${i}`, message: user() });
      entries.push({ entry_id: `a${i}`, message: asst() });
      entries.push({ entry_id: `t${i}`, message: tool('shell::fs::cat', big) });
    }
    const { iii, updates } = makeIii(entries);
    const res = await prune(iii, 'sid', {
      protectTokens: 40_000,
      minFree: 20_000,
      protectedTools: [],
    });
    expect(res.pruned_parts).toBeGreaterThan(0);
    expect(updates.length).toBe(res.pruned_parts);
  });

  it('skips parts in protectedTools', async () => {
    const big = 'x'.repeat(200_000);
    const entries: Array<{ entry_id: string; message: AgentMessage }> = [];
    for (let i = 0; i < 3; i++) {
      entries.push({ entry_id: `u${i}`, message: user() });
    }
    entries.push({ entry_id: 't1', message: tool('skill::ask', big) });
    entries.push({ entry_id: 't2', message: tool('shell::fs::cat', big) });
    const { iii, updates } = makeIii(entries);
    await prune(iii, 'sid', { protectTokens: 0, minFree: 1000, protectedTools: ['skill::ask'] });
    expect(updates.some((u) => u.entry_id === 't1')).toBe(false);
  });
});
