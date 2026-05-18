import { describe, expect, it } from 'vitest';
import { decideExit } from '../../src/hook-fanout/exit.js';

describe('decideExit', () => {
  const future = (n: number) => Date.now() + n;

  it('fires expected_replies the moment count reaches target', () => {
    expect(
      decideExit({
        replies_count: 3,
        expected_replies: 3,
        quiescence_ms: 200,
        last_reply_at_ms: Date.now(),
        now_ms: Date.now(),
        deadline_ms: future(60_000),
      }),
    ).toBe('expected_replies');
  });

  it('does not fire expected_replies when count short', () => {
    expect(
      decideExit({
        replies_count: 2,
        expected_replies: 3,
        quiescence_ms: 200,
        last_reply_at_ms: Date.now(),
        now_ms: Date.now(),
        deadline_ms: future(60_000),
      }),
    ).toBeNull();
  });

  it('fires quiescence after idle window with at least one reply', () => {
    const now = Date.now();
    expect(
      decideExit({
        replies_count: 1,
        quiescence_ms: 200,
        last_reply_at_ms: now - 350,
        now_ms: now,
        deadline_ms: now + 60_000,
      }),
    ).toBe('quiescence');
  });

  it('quiescence does not fire without any reply seen', () => {
    const now = Date.now();
    expect(
      decideExit({
        replies_count: 0,
        quiescence_ms: 200,
        last_reply_at_ms: undefined,
        now_ms: now,
        deadline_ms: now + 60_000,
      }),
    ).toBeNull();
  });

  it('quiescence_ms = 0 disables the branch', () => {
    const now = Date.now();
    expect(
      decideExit({
        replies_count: 1,
        quiescence_ms: 0,
        last_reply_at_ms: now - 5000,
        now_ms: now,
        deadline_ms: now + 60_000,
      }),
    ).toBeNull();
  });

  it('deadline reason depends on whether any reply was seen', () => {
    const now = Date.now();
    expect(
      decideExit({
        replies_count: 1,
        quiescence_ms: 0,
        last_reply_at_ms: now - 50,
        now_ms: now,
        deadline_ms: now - 1,
      }),
    ).toBe('deadline');
    expect(
      decideExit({
        replies_count: 0,
        quiescence_ms: 0,
        now_ms: now,
        deadline_ms: now - 1,
      }),
    ).toBe('deadline_no_replies');
  });
});
