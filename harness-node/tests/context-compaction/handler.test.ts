import { describe, expect, it } from 'vitest';
import {
  extractEventPayload,
  turnEndUsage,
  usageTotal,
} from '../../src/context-compaction/handler.js';

describe('extractEventPayload', () => {
  it('handles camelCase envelope', () => {
    const env = {
      groupId: 'sess-1',
      event: { data: { type: 'TurnEnd', message: {} } },
    };
    const out = extractEventPayload(env);
    expect(out?.session_id).toBe('sess-1');
    expect((out?.event as { type: string }).type).toBe('TurnEnd');
  });

  it('handles snake_case envelope', () => {
    const env = { group_id: 'sess-2', data: { type: 'TurnEnd' } };
    const out = extractEventPayload(env);
    expect(out?.session_id).toBe('sess-2');
  });
});

describe('turnEndUsage', () => {
  it('extracts usage on TurnEnd', () => {
    const event = {
      type: 'TurnEnd',
      message: { usage: { input: 100, output: 50, cache_read: 800 } },
    };
    expect(turnEndUsage(event)).toEqual({ input: 100, output: 50, cache_read: 800 });
  });

  it('skips non-TurnEnd events', () => {
    for (const kind of ['TurnStart', 'MessageStart', 'AgentStart']) {
      expect(turnEndUsage({ type: kind, message: { usage: { input: 9999 } } })).toBeNull();
    }
  });

  it('skips when usage missing', () => {
    expect(turnEndUsage({ type: 'TurnEnd', message: {} })).toBeNull();
  });
});

describe('usageTotal', () => {
  it('sums input + output + cache_read; excludes cache_write', () => {
    expect(usageTotal({ input: 100, output: 50, cache_read: 800, cache_write: 200 })).toBe(950);
  });

  it('handles missing fields', () => {
    expect(usageTotal({ input: 100 })).toBe(100);
    expect(usageTotal({})).toBe(0);
  });
});
