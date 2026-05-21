import { describe, expect, it } from 'vitest';
import {
  completedCompactions,
  select,
  selectWithEntryIds,
  splitTurn,
  turns,
} from '../../src/context-compaction/selection.js';
import type { AgentMessage } from '../../src/types/agent-message.js';

function user(text = ''): AgentMessage {
  return { role: 'user', content: [{ type: 'text', text }], timestamp: 0 };
}
function asst(text = ''): AgentMessage {
  return {
    role: 'assistant',
    content: [{ type: 'text', text }],
    stop_reason: 'end',
    model: 'm',
    provider: 'p',
    timestamp: 0,
  };
}

describe('turns', () => {
  it('groups messages into user→...→user turns', () => {
    const msgs = [user('q1'), asst('a1'), asst('a1b'), user('q2'), asst('a2')];
    const ts = turns(msgs);
    expect(ts).toEqual([
      { index: 0, start: 0, end: 3 },
      { index: 3, start: 3, end: 5 },
    ]);
  });
  it('returns empty array for no-user transcript', () => {
    expect(turns([asst('x')])).toEqual([]);
  });
  it('produces non-overlapping contiguous ranges', () => {
    const msgs = [user(), asst(), user(), asst(), user(), asst()];
    const ts = turns(msgs);
    expect(ts[0]!.end).toBe(ts[1]!.start);
    expect(ts[1]!.end).toBe(ts[2]!.start);
    expect(ts[2]!.end).toBe(msgs.length);
  });
});

describe('splitTurn', () => {
  const fakeEstimate = (msgs: AgentMessage[]) => msgs.length * 1000;
  it('returns undefined when budget is non-positive', () => {
    const msgs = [user(), asst(), asst()];
    expect(
      splitTurn({
        messages: msgs,
        turn: { index: 0, start: 0, end: 3 },
        budget: 0,
        estimate: fakeEstimate,
      }),
    ).toBeUndefined();
  });
  it('returns undefined for single-message turns', () => {
    const msgs = [user()];
    expect(
      splitTurn({
        messages: msgs,
        turn: { index: 0, start: 0, end: 1 },
        budget: 999_999,
        estimate: fakeEstimate,
      }),
    ).toBeUndefined();
  });
  it('finds the first split point that fits the budget (walking forward)', () => {
    const msgs = [user(), asst(), asst(), asst(), asst()];
    const out = splitTurn({
      messages: msgs,
      turn: { index: 0, start: 0, end: 5 },
      budget: 2_500,
      estimate: fakeEstimate,
    });
    expect(out).toEqual({ start: 3, index: 3 });
  });
  it('returns undefined when no split fits', () => {
    const msgs = [user(), asst(), asst()];
    const out = splitTurn({
      messages: msgs,
      turn: { index: 0, start: 0, end: 3 },
      budget: 500,
      estimate: fakeEstimate,
    });
    expect(out).toBeUndefined();
  });
});

describe('select', () => {
  const sizeEach = 1000;
  const estimate = (msgs: AgentMessage[]) => msgs.length * sizeEach;
  it('keeps full tail when budget allows', () => {
    const msgs = [user('a'), asst(), user('b'), asst()];
    const r = select({ messages: msgs, budget: 99_999, tailTurns: 2, estimate });
    expect(r.head).toEqual(msgs);
    expect(r.tail_start_index).toBeUndefined();
  });
  it('walks backward, includes a turn that fits', () => {
    const msgs = [user('a'), asst(), asst(), user('b'), asst()];
    const r = select({ messages: msgs, budget: 2_000, tailTurns: 2, estimate });
    expect(r.tail_start_index).toBe(3);
    expect(r.head.length).toBe(3);
  });
  it('falls back to splitTurn when a single turn exceeds budget', () => {
    const msgs = [user(), asst(), asst(), asst(), asst()];
    const r = select({ messages: msgs, budget: 2_500, tailTurns: 2, estimate });
    expect(r.tail_start_index).toBe(3);
  });
  it('returns full messages when tailTurns is 0', () => {
    const msgs = [user(), asst()];
    const r = select({ messages: msgs, budget: 999, tailTurns: 0, estimate });
    expect(r.head).toEqual(msgs);
    expect(r.tail_start_index).toBeUndefined();
  });
  it('returns full messages when no turns found', () => {
    const r = select({ messages: [asst()], budget: 999, tailTurns: 2, estimate });
    expect(r.tail_start_index).toBeUndefined();
  });
});

describe('selectWithEntryIds', () => {
  it('maps tail_start_index back to entry_id', () => {
    const entries = [
      { entry_id: 'e1', message: user() },
      { entry_id: 'e2', message: asst() },
      { entry_id: 'e3', message: user() },
      { entry_id: 'e4', message: asst() },
    ];
    const r = selectWithEntryIds({
      entries,
      budget: 2_000,
      tailTurns: 1,
      estimate: (m) => m.length * 1000,
    });
    expect(r.tail_start_id).toBe('e3');
    expect(r.head.length).toBe(2);
  });
});

describe('completedCompactions', () => {
  it('returns summaries in timestamp order', () => {
    const entries = [
      { id: 'c2', summary: 'second', tokens_before: 100, timestamp: 200 },
      { id: 'c1', summary: 'first', tokens_before: 50, timestamp: 100 },
    ];
    expect(completedCompactions(entries)).toEqual([
      { summary: 'first', userIndex: -1, assistantIndex: -1 },
      { summary: 'second', userIndex: -1, assistantIndex: -1 },
    ]);
  });
  it('skips entries with empty summary', () => {
    const entries = [
      { id: 'c1', summary: '', tokens_before: 0, timestamp: 100 },
      { id: 'c2', summary: 'ok', tokens_before: 0, timestamp: 200 },
    ];
    expect(completedCompactions(entries)).toEqual([
      { summary: 'ok', userIndex: -1, assistantIndex: -1 },
    ]);
  });
});
