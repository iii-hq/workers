import { describe, expect, it } from 'vitest';
import { identityBaggage, preview, runTurnSpan } from '../src/trace.js';

/**
 * The contract is the mapping: which identity key carries which value. That is
 * what the console's trace views read, and it is testable on its own — the
 * plumbing that copies baggage onto spans belongs to the SDK's OTel setup,
 * which no unit test runs.
 */
describe('trace identity', () => {
  it('carries the keys a harness turn carries', () => {
    expect(
      identityBaggage({
        sessionId: 'session-1',
        turnId: 'turn-1',
        kind: 'claude.terminal.turn',
        sessionName: 'Nightly triage',
        message: '  fix   the flaky test\n',
        displayName: 'Claude terminal · fix the flaky test',
      }),
    ).toEqual({
      'iii.session.id': 'session-1',
      'iii.message.id': 'turn-1',
      'iii.session.name': 'Nightly triage',
      'iii.tag.kind': 'claude.terminal.turn',
      // Collapsed to one line: a label, not a transcript.
      'iii.tag.message': 'fix the flaky test',
      'iii.tag.display_name': 'Claude terminal · fix the flaky test',
    });
  });

  it('omits every key it has no value for', () => {
    expect(identityBaggage({ sessionId: 's', kind: 'claude.run', message: '   ' })).toEqual({
      'iii.session.id': 's',
      'iii.tag.kind': 'claude.run',
    });
  });

  it('runs the work and returns its answer', async () => {
    const answer = await runTurnSpan(
      'claude terminal UserPromptSubmit',
      { sessionId: 's', kind: 'claude.terminal.turn' },
      async () => 'done',
    );
    expect(answer).toBe('done');
  });

  it('never swallows the work’s failure', async () => {
    await expect(
      runTurnSpan(
        'claude terminal PostToolUse',
        { sessionId: 's', kind: 'claude.run' },
        async () => {
          throw new Error('tool exploded');
        },
      ),
    ).rejects.toThrow('tool exploded');
  });

  it('previews one bounded line', () => {
    expect(preview('a\n\n  b   c ')).toBe('a b c');
    expect(preview('x'.repeat(200))).toHaveLength(120);
    expect(preview('x'.repeat(200)).endsWith('…')).toBe(true);
  });
});
