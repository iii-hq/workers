import { describe, expect, it } from 'vitest';
import { estimateMessages } from '../../src/turn-orchestrator/estimate.js';

describe('estimateMessages', () => {
  it('returns positive count for non-empty messages', () => {
    expect(
      estimateMessages([
        { role: 'user', content: [{ type: 'text', text: 'x'.repeat(100) }], timestamp: 0 },
      ]),
    ).toBeGreaterThan(0);
  });

  it('returns 0 for empty array', () => {
    expect(estimateMessages([])).toBe(0);
  });

  it('uses chars/4 heuristic', () => {
    const msg = { role: 'user' as const, content: [] as never[], timestamp: 0 };
    const serialized = JSON.stringify(msg);
    const expected = Math.floor(serialized.length / 4);
    expect(estimateMessages([msg])).toBe(expected);
  });

  it('accumulates across multiple messages', () => {
    const msgs = [
      { role: 'user' as const, content: [{ type: 'text' as const, text: 'hello' }], timestamp: 1 },
      { role: 'user' as const, content: [{ type: 'text' as const, text: 'world' }], timestamp: 2 },
    ];
    const single = estimateMessages(msgs.slice(0, 1));
    const both = estimateMessages(msgs);
    expect(both).toBeGreaterThan(single);
  });
});
