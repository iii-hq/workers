import { describe, expect, it } from 'vitest';
import {
  mergeFieldMerge,
  mergeFirstBlockWins,
  mergePipelineLastWins,
} from '../../src/hook-fanout/merge.js';

describe('mergeFirstBlockWins', () => {
  it('returns first blocker', () => {
    const merged = mergeFirstBlockWins([
      {},
      { block: false },
      { block: true, reason: 'first' },
      { block: true, reason: 'second' },
    ]);
    expect(merged).toEqual({ block: true, reason: 'first' });
  });

  it('preserves denial envelope when blocker carries one', () => {
    const denial = { schema_version: 1, status: 'denied', denied_by: 'permissions' };
    const merged = mergeFirstBlockWins([{ block: true, reason: 'no', denial }]);
    expect((merged as { denial?: unknown }).denial).toEqual(denial);
  });

  it('defaults to no block', () => {
    expect(mergeFirstBlockWins([{}, { block: false }])).toEqual({ block: false });
  });
});

describe('mergeFieldMerge', () => {
  it('replaces content, merges details, updates terminate', () => {
    const initial = {
      content: [{ type: 'text', text: 'ok' }],
      details: { a: 1, b: 2 },
      terminate: false,
    };
    const merged = mergeFieldMerge(initial, [
      {
        content: [{ type: 'text', text: 'rewritten' }],
        details: { b: 99, c: 3 },
        terminate: true,
      },
    ]) as Record<string, unknown>;
    expect((merged.content as Array<{ text: string }>)[0].text).toBe('rewritten');
    expect(merged.details).toEqual({ a: 1, b: 99, c: 3 });
    expect(merged.terminate).toBe(true);
  });
});

describe('mergePipelineLastWins', () => {
  it('uses last decodable transform', () => {
    const original = [{ role: 'user', content: [], timestamp: 0 }];
    const out = mergePipelineLastWins(original, [
      { messages: [{ role: 'assistant', content: [], timestamp: 1 }] },
      { ignored: true },
      [{ role: 'user', content: [], timestamp: 2 }],
    ]);
    expect((out as Array<{ timestamp: number }>)[0].timestamp).toBe(2);
  });

  it('falls back to original when nothing decodes', () => {
    const original = [{ role: 'user', content: [] }];
    expect(mergePipelineLastWins(original, [{ ignored: true }])).toEqual(original);
  });
});
