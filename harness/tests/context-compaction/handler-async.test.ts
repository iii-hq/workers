import { describe, expect, it } from 'vitest';
import { parseOnTurnEnd } from '../../src/context-compaction/handler-async.js';

describe('parseOnTurnEnd', () => {
  it('parses a well-formed turn_end payload', () => {
    const out = parseOnTurnEnd({
      session_id: 'sess-1',
      usage: { input: 100, output: 50, cache_read: 800 },
      provider: 'anthropic',
      model: 'claude-haiku-4-5',
    });
    expect(out).toEqual({
      session_id: 'sess-1',
      usage: { input: 100, output: 50, cache_read: 800 },
      provider: 'anthropic',
      model: 'claude-haiku-4-5',
    });
  });

  it('defaults provider/model to empty strings when absent (handler falls back to session-tree)', () => {
    const out = parseOnTurnEnd({ session_id: 'sess-2', usage: { input: 10 } });
    expect(out).toEqual({ session_id: 'sess-2', usage: { input: 10 }, provider: '', model: '' });
  });

  it('returns null usage when usage is missing or malformed', () => {
    expect(parseOnTurnEnd({ session_id: 'sess-3' })?.usage).toBeNull();
    expect(parseOnTurnEnd({ session_id: 'sess-3', usage: 'nope' })?.usage).toBeNull();
  });

  it('returns null when session_id is missing', () => {
    expect(parseOnTurnEnd({ usage: { input: 1 } })).toBeNull();
    expect(parseOnTurnEnd({ session_id: '' })).toBeNull();
    expect(parseOnTurnEnd(null)).toBeNull();
    expect(parseOnTurnEnd(42)).toBeNull();
  });
});
