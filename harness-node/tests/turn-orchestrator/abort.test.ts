import { describe, expect, it } from 'vitest';
import { abortSideEffects } from '../../src/turn-orchestrator/abort.js';

describe('abortSideEffects', () => {
  it('returns state::set abort_signal followed by approval::sweep_session', () => {
    const effects = abortSideEffects('sess-a');
    expect(effects).toHaveLength(2);

    expect(effects[0]).toEqual({
      function_id: 'state::set',
      payload: {
        scope: 'agent',
        key: 'session/sess-a/abort_signal',
        value: true,
      },
    });

    expect(effects[1]).toEqual({
      function_id: 'approval::sweep_session',
      payload: { session_id: 'sess-a' },
    });
  });

  it('namespaces the abort key with the supplied session_id', () => {
    const effects = abortSideEffects('other-session');
    expect((effects[0]?.payload as { key: string }).key).toBe('session/other-session/abort_signal');
    expect((effects[1]?.payload as { session_id: string }).session_id).toBe('other-session');
  });
});
