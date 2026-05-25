import { describe, expect, it } from 'vitest';
import { parseFlatMessages } from '../../src/turn-orchestrator/flat-messages.js';

describe('parseFlatMessages', () => {
  it('returns the array when messages are objects', () => {
    const messages = [{ role: 'user', content: [], timestamp: 1 }];
    expect(parseFlatMessages(messages)).toEqual(messages);
  });

  it('returns [] for null, undefined, and non-arrays', () => {
    expect(parseFlatMessages(null)).toEqual([]);
    expect(parseFlatMessages(undefined)).toEqual([]);
    expect(parseFlatMessages('bad')).toEqual([]);
    expect(parseFlatMessages({})).toEqual([]);
  });
});
