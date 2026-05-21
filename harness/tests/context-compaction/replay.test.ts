import { describe, expect, it, vi } from 'vitest';
import { extractReplayTarget, reinjectReplay } from '../../src/context-compaction/replay.js';
import type { AgentMessage } from '../../src/types/agent-message.js';

const user = (text = 'q'): { entry_id: string; message: AgentMessage } => ({
  entry_id: `u-${text}`,
  message: { role: 'user', content: [{ type: 'text', text }], timestamp: 0 },
});
const asst = (text = 'a'): { entry_id: string; message: AgentMessage } => ({
  entry_id: `a-${text}`,
  message: {
    role: 'assistant',
    content: [{ type: 'text', text }],
    stop_reason: 'end',
    model: 'm',
    provider: 'p',
    timestamp: 0,
  },
});

describe('extractReplayTarget', () => {
  it('extracts the message matching lastUserMessageId', () => {
    const entries = [user('q1'), asst('a1'), user('q2')];
    const { replay, truncatedMessages } = extractReplayTarget(entries, 'u-q2');
    expect(replay?.entry_id).toBe('u-q2');
    expect(truncatedMessages.map((e) => e.entry_id)).toEqual(['u-q1', 'a-a1']);
  });
  it('returns undefined when target not found', () => {
    const entries = [user('q1'), asst('a1')];
    const { replay, truncatedMessages } = extractReplayTarget(entries, 'missing');
    expect(replay).toBeUndefined();
    expect(truncatedMessages).toEqual(entries);
  });
  it('returns undefined when target is not a user message', () => {
    const entries = [user('q1'), asst('a1')];
    const { replay } = extractReplayTarget(entries, 'a-a1');
    expect(replay).toBeUndefined();
  });
});

describe('reinjectReplay', () => {
  it('writes the replay user message back via session-tree::append', async () => {
    const trigger = vi.fn(async () => ({ entry_id: 'new' }));
    const iii = { trigger } as unknown as Parameters<typeof reinjectReplay>[0];
    await reinjectReplay(iii, 'sid', user('q2'));
    expect(trigger).toHaveBeenCalledTimes(1);
    const call = trigger.mock.calls[0]![0] as {
      function_id: string;
      payload: { session_id: string; message: { role: string } };
    };
    expect(call.function_id).toBe('session-tree::append');
    expect(call.payload.session_id).toBe('sid');
    expect(call.payload.message.role).toBe('user');
  });
});
