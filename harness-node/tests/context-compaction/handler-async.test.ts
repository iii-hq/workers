import { describe, expect, it } from 'vitest';
import {
  extractEventPayload,
  turnEndUsage,
} from '../../src/context-compaction/handler-async.js';

describe('extractEventPayload', () => {
  it('handles camelCase envelope (event.data shape)', () => {
    const env = {
      groupId: 'sess-1',
      event: { data: { type: 'TurnEnd', message: {} } },
    };
    const out = extractEventPayload(env);
    expect(out?.session_id).toBe('sess-1');
    expect((out?.event as { type: string }).type).toBe('TurnEnd');
  });

  it('handles snake_case envelope (top-level data shape)', () => {
    const env = { group_id: 'sess-2', data: { type: 'TurnEnd' } };
    const out = extractEventPayload(env);
    expect(out?.session_id).toBe('sess-2');
    expect((out?.event as { type: string }).type).toBe('TurnEnd');
  });

  it('returns null when session id is missing', () => {
    expect(extractEventPayload({ data: { type: 'TurnEnd' } })).toBeNull();
    expect(extractEventPayload(null)).toBeNull();
    expect(extractEventPayload(42)).toBeNull();
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

  it('extracts usage on turn_end (snake_case variant)', () => {
    const event = {
      type: 'turn_end',
      message: { usage: { input: 200, output: 30 } },
    };
    expect(turnEndUsage(event)).toEqual({ input: 200, output: 30 });
  });

  it('returns null for non-TurnEnd events', () => {
    for (const kind of ['TurnStart', 'MessageStart', 'AgentStart']) {
      expect(turnEndUsage({ type: kind, message: { usage: { input: 9999 } } })).toBeNull();
    }
  });

  it('returns null when usage is missing', () => {
    expect(turnEndUsage({ type: 'TurnEnd', message: {} })).toBeNull();
    expect(turnEndUsage({ type: 'TurnEnd' })).toBeNull();
  });
});
