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
        kind: 'pi.terminal.turn',
        sessionName: 'Nightly triage',
        message: '  fix   the flaky test\n',
        displayName: 'pi terminal · fix the flaky test',
      }),
    ).toEqual({
      'iii.session.id': 'session-1',
      'iii.message.id': 'turn-1',
      'iii.session.name': 'Nightly triage',
      'iii.tag.kind': 'pi.terminal.turn',
      // Collapsed to one line: a label, not a transcript.
      'iii.tag.message': 'fix the flaky test',
      'iii.tag.display_name': 'pi terminal · fix the flaky test',
    });
  });

  it('omits every key it has no value for', () => {
    expect(identityBaggage({ sessionId: 's', kind: 'pi.task', message: '   ' })).toEqual({
      'iii.session.id': 's',
      'iii.tag.kind': 'pi.task',
    });
  });

  it('runs the work and returns its answer', async () => {
    const answer = await runTurnSpan(
      'pi agent_start',
      { sessionId: 's', kind: 'pi.terminal.turn' },
      async () => 'done',
    );
    expect(answer).toBe('done');
  });

  it('never swallows the work’s failure', async () => {
    await expect(
      runTurnSpan('pi tool_end', { sessionId: 's', kind: 'pi.task' }, async () => {
        throw new Error('tool exploded');
      }),
    ).rejects.toThrow('tool exploded');
  });

  it('previews one bounded line', () => {
    expect(preview('a\n\n  b   c ')).toBe('a b c');
    expect(preview('x'.repeat(200))).toHaveLength(120);
    expect(preview('x'.repeat(200)).endsWith('…')).toBe(true);
  });
});
