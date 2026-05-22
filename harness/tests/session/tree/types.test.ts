import { describe, expect, it } from 'vitest';
import { type SessionEntry, entryTimestamp } from '../../../src/session/tree/types.js';

describe('entryTimestamp', () => {
  it('returns the timestamp for a message entry', () => {
    const e = {
      type: 'message',
      id: 'e-1',
      message: { role: 'user', content: [], timestamp: 100 },
      timestamp: 100,
    } as unknown as SessionEntry;
    expect(entryTimestamp(e)).toBe(100);
  });

  it('returns the timestamp for a custom_message entry', () => {
    const e: SessionEntry = {
      type: 'custom_message',
      id: 'e-2',
      custom_type: 'system',
      content: null,
      timestamp: 200,
    };
    expect(entryTimestamp(e)).toBe(200);
  });

  it('returns the timestamp for a branch_summary entry', () => {
    const e: SessionEntry = {
      type: 'branch_summary',
      id: 'e-3',
      summary: 'branched',
      from_id: 'e-0',
      timestamp: 300,
    };
    expect(entryTimestamp(e)).toBe(300);
  });

  it('returns the timestamp for a compaction entry', () => {
    const e: SessionEntry = {
      type: 'compaction',
      id: 'e-4',
      summary: 'compacted',
      tokens_before: 1024,
      details: {},
      timestamp: 400,
    };
    expect(entryTimestamp(e)).toBe(400);
  });
});
